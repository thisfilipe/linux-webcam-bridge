#!/usr/bin/python3
"""Synthetic only: no D-Bus connections, USB access, or service operations."""
import importlib.util
import os
from pathlib import Path
import queue
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).resolve().parents[1] / 'src' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


bridge = load('jieli_bridge_fixture', 'jieli-camera-bridge.py')
publisher = load('jieli_publisher_fixture', 'jieli-pipewire-publisher.py')


class Pipeline:
    def __init__(self):
        self.state = bridge.Gst.State.PLAYING
        self.bus = Mock()

    def get_bus(self):
        return self.bus

    def set_state(self, state):
        self.state = state
        return bridge.Gst.StateChangeReturn.SUCCESS

    def get_state(self, timeout):
        return bridge.Gst.StateChangeReturn.SUCCESS, self.state, bridge.Gst.State.VOID_PENDING


class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        args = SimpleNamespace(test_source=True, node_name='fixture',
                               state_file=Path(self.directory.name) / 'state.json')
        self.b = bridge.Bridge(args)
        self.b.args.test_source = False
        self.b.log = Mock()
        self.b.publisher = SimpleNamespace(poll=lambda: None)
        self.b.publish_queue = queue.Queue(maxsize=1)
        self.now = 100.0
        self.clock = patch.object(bridge.time, 'monotonic', lambda: self.now)
        self.clock.start()
        self.b.device_identity = Mock(return_value='camera-A')
        self.jobs = []
        self.b.hardware = lambda work, done: self.jobs.append((work, done))
        self.b.start_capture = Mock()
        self.b.seen = 'camera-A'

    def tearDown(self):
        self.clock.stop()
        self.directory.cleanup()

    def complete(self):
        work, done = self.jobs.pop(0)
        try:
            result, error = work(), None
        except Exception as exc:
            result, error = None, exc
        done(result, error)

    def active(self):
        self.b.capture = Pipeline()
        self.b.identity = 'camera-A'
        self.b.capture_generation = 4
        self.b.started = self.b.last_frame = self.now

    def test_capture_close_keeps_publisher_and_invalidates_queue(self):
        self.active()
        publication = self.b.publisher
        self.b.publish_queue.put((4, b'private'))
        self.b.stop_capture()
        self.assertIs(self.b.publisher, publication)
        self.assertTrue(self.b.capture_closing)
        self.assertTrue(self.b.publish_queue.empty())
        self.assertEqual(self.b.capture_generation, 5)
        self.complete()
        self.assertFalse(self.b.capture_closing)

    def test_reset_waits_for_capture_teardown(self):
        self.active()
        self.b.recover('no frames')
        self.b.reset_usb = Mock(return_value={'returncode': 0, 'message': 'ok'})
        self.b.tick()
        self.assertEqual(len(self.jobs), 1)
        self.b.reset_usb.assert_not_called()
        self.complete()
        self.b.tick()
        self.assertTrue(self.b.reset_pending)
        self.complete()
        self.b.reset_usb.assert_called_once()
        self.assertEqual(self.b.recovery, 'settling')
        self.b.tick()
        self.b.start_capture.assert_not_called()
        self.now += 1.1
        self.b.tick()
        self.b.start_capture.assert_called_once_with('camera-A')

    def test_resume_reinitializes_budget_but_keeps_publication(self):
        self.active()
        publication = self.b.publisher
        self.b.retries = 3
        self.b.prepare_sleep()
        self.assertTrue(self.b.sleeping)
        self.assertEqual(len(self.jobs), 1)
        self.b.resume()
        self.assertFalse(self.b.sleeping)
        self.assertEqual(self.b.retries, 0)
        self.assertTrue(self.b.reset_required)
        self.b.tick()
        self.assertEqual(len(self.jobs), 1)
        self.assertIs(self.b.publisher, publication)

    def test_failure_budget_and_backoff_are_bounded(self):
        self.b.reset_required = True
        self.b.reset_usb = Mock(side_effect=subprocess.TimeoutExpired(bridge.RESET, 12))
        for attempt in range(1, 4):
            self.b.tick()
            self.complete()
            self.assertEqual(self.b.retries, attempt)
            self.assertGreaterEqual(self.b.retry_at - self.now, 8)
            self.b.start_capture.assert_not_called()
            self.now = self.b.retry_at
        self.b.tick()
        self.assertEqual(self.b.recovery, 'exhausted')
        self.assertEqual(self.b.reset_usb.call_count, 3)
        self.now += 1000
        self.b.tick()
        self.assertEqual(len(self.jobs), 0)

    def test_nonzero_helper_never_allows_open(self):
        self.b.reset_required = True
        self.b.reset_usb = Mock(return_value={'returncode': 1, 'message': 'No such device'})
        self.b.tick()
        self.complete()
        self.assertTrue(self.b.reset_required)
        self.assertEqual(self.b.recovery, 'backoff')
        self.b.start_capture.assert_not_called()

    def test_missing_authorization_is_terminal_without_hot_loop(self):
        self.b.reset_required = True
        self.b.reset_usb = Mock(side_effect=bridge.ResetUnavailable('helper not authorized'))
        self.b.tick()
        self.complete()
        self.assertEqual(self.b.recovery, 'helper-unavailable')
        for _ in range(10):
            self.now += 100
            self.b.tick()
        self.assertEqual(self.b.reset_usb.call_count, 1)
        self.b.start_capture.assert_not_called()

    def test_failed_teardown_prevents_reset_or_open(self):
        self.active()
        self.b.capture.set_state = Mock(side_effect=RuntimeError('stuck'))
        self.b.recover('failure')
        self.complete()
        self.assertIsNotNone(self.b.close_failed)
        self.b.tick()
        self.assertEqual(len(self.jobs), 0)
        self.b.start_capture.assert_not_called()

    def test_stale_sample_cannot_publish_or_change_metrics(self):
        sample = Mock()
        sample.get_buffer.return_value.extract_dup.return_value = b'private'
        sample.get_buffer.return_value.get_size.return_value = 7
        sink = Mock()
        sink.emit.return_value = sample
        self.b.capture_generation = 5
        self.b.new_sample(sink, 4)
        self.assertEqual(self.b.frames, 0)
        self.assertTrue(self.b.publish_queue.empty())

    def test_stale_reset_callback_cannot_reopen_after_sleep(self):
        self.b.reset_required = True
        self.b.reset_usb = Mock(return_value={'returncode': 0, 'message': 'ok'})
        self.b.tick()
        self.b.prepare_sleep()
        self.complete()
        self.assertFalse(self.b.reset_pending)
        self.assertEqual(self.b.recovery, 'sleeping')
        self.b.start_capture.assert_not_called()

    def test_sleep_lease_waits_for_reset_already_in_flight(self):
        self.b.sleep = SimpleNamespace(fd=123, error=None, release=Mock())
        self.b.reset_required = True
        self.b.reset_usb = Mock(return_value={'returncode': 0, 'message': 'ok'})
        self.b.tick()
        self.b.prepare_sleep()
        self.b.sleep.release.assert_not_called()
        self.complete()
        self.b.sleep.release.assert_called_once()
        self.b.start_capture.assert_not_called()

    def test_no_frames_startup_attempts_recovery(self):
        self.active()
        self.b.started = self.now - 13
        self.b.last_frame = 0
        self.b.tick()
        self.assertTrue(self.b.reset_required)
        self.assertTrue(self.b.capture_closing)
        self.assertIsNotNone(self.b.publisher)

    def test_physical_reenumeration_releases_exhaustion(self):
        self.b.blocked = 'camera-old'
        self.b.recovery = 'exhausted'
        self.b.retries = 3
        self.b.tick()
        self.assertEqual(self.b.retries, 0)
        self.assertIsNone(self.b.blocked)
        self.b.start_capture.assert_called_once_with('camera-A')

    def test_timeout_barrier_survives_physical_reenumeration(self):
        self.b.reset_barrier = True
        self.b.blocked = 'camera-old'
        self.b.recovery = 'exhausted'
        self.b.retries = 3
        self.b.tick()
        self.assertTrue(self.b.reset_pending)
        self.assertTrue(self.b.reset_required)
        self.b.start_capture.assert_not_called()

    def test_known_recovery_can_reset_without_v4l2_node(self):
        self.b.device_identity.return_value = None
        self.b.reset_required = True
        self.b.tick()
        self.assertTrue(self.b.reset_pending)
        self.b.start_capture.assert_not_called()

    def test_old_frame_burst_cannot_erase_recovery_budget(self):
        self.active()
        self.b.started = self.now - 11
        self.b.last_frame = self.now - 7
        self.b.frames = 100
        self.b.retries = 2
        self.b.tick()
        self.assertEqual(self.b.retries, 2)

    def test_exact_reset_command_and_timeout(self):
        result = SimpleNamespace(returncode=0, stdout='reset ok', stderr='')
        with patch.object(bridge.Path, 'is_file', return_value=True), patch.object(bridge.subprocess, 'run', return_value=result) as run:
            self.assertEqual(self.b.reset_usb()['returncode'], 0)
        run.assert_called_once_with(('/usr/bin/sudo', '-n', '/usr/local/libexec/jieli-usb-reset'),
                                   capture_output=True, text=True, timeout=12)


class SleepLease(unittest.TestCase):
    def setUp(self):
        self.b = SimpleNamespace(stopping=False, sleeping=False, log=Mock(),
                                 prepare_sleep=Mock(), resume=Mock(), sleep_deadline=Mock())
        self.m = bridge.SleepMonitor(self.b)
        self.read, self.write = os.pipe()
        self.m.fd = self.read
        self.m.limit = 0.05

    def tearDown(self):
        self.m.close()
        os.close(self.write)

    def test_sleep_callback_only_schedules_close_and_deadline_releases_fd(self):
        timers = []
        class Timer:
            def __init__(self, delay, callback):
                self.delay, self.callback = delay, callback
                timers.append(self)
            def start(self): pass
            def cancel(self): pass
        with patch.object(bridge.threading, 'Timer', Timer), patch.object(bridge.GLib, 'idle_add') as idle:
            self.m.signal(None, None, 'PrepareForSleep', bridge.GLib.Variant('(b)', (True,)))
            self.b.prepare_sleep.assert_called_once()
            self.assertLess(timers[0].delay, self.m.limit)
            self.assertIsNotNone(self.m.fd)
            timers[0].callback()
            self.assertIsNone(self.m.fd)
            idle.assert_called_once_with(self.b.sleep_deadline)
            with self.assertRaises(OSError):
                os.fstat(self.read)

    def test_old_deadline_cannot_release_new_resume_lease(self):
        token = self.m.generation
        self.m.generation += 1
        self.assertFalse(self.m.release(token))
        os.fstat(self.read)

    def test_acquire_requests_delay_and_stale_reply_closes_fd(self):
        self.m.release()
        fd, writer = os.pipe()
        self.m.proxy = Mock()
        self.m.proxy.call_with_unix_fd_list_sync.return_value = (
            bridge.GLib.Variant('(h)', (0,)), SimpleNamespace(get=lambda index: fd))
        jobs = []
        with patch.object(bridge, 'background', lambda work, done: jobs.append((work, done))):
            self.m.acquire()
            result = jobs[0][0]()
            self.m.generation += 1
            jobs[0][1](result, None)
        call = self.m.proxy.call_with_unix_fd_list_sync.call_args.args
        self.assertEqual(call[0], 'Inhibit')
        self.assertEqual(call[1].unpack()[0::3], ('sleep', 'delay'))
        with self.assertRaises(OSError):
            os.fstat(fd)
        os.close(writer)


class Placeholder(unittest.TestCase):
    def test_neutral_i420_with_text(self):
        frame = publisher.unavailable_frame()
        self.assertEqual(len(frame), publisher.FRAME_SIZE)
        self.assertEqual(set(frame[1280 * 720:]), {128})
        self.assertEqual(set(frame[:1280 * 720]), {32, 235})
        self.assertIsInstance(frame, bytes)

    def test_partial_reads_preserve_bytes_then_neutral_never_replays(self):
        private = b'X' * publisher.FRAME_SIZE
        now = [0.0]
        chunks = [private[:100], private[100:]]
        def wait(r, w, x, timeout):
            if chunks:
                return r, [], []
            now[0] += timeout
            return [], [], []
        def read(fd, size):
            return chunks.pop(0)
        stream = publisher.input_frames(0, clock=lambda: now[0], wait=wait, read=read)
        real, unavailable = next(stream)
        if unavailable:  # Startup may publish neutral during a partial pipe read.
            real, unavailable = next(stream)
        self.assertFalse(unavailable)
        self.assertEqual(real, private)
        neutral, unavailable = next(stream)
        self.assertTrue(unavailable)
        self.assertGreaterEqual(now[0], 0.25)
        self.assertNotEqual(neutral, private)
        t = now[0]
        for _ in range(30):
            frame, unavailable = next(stream)
            self.assertTrue(unavailable)
            self.assertEqual(frame, neutral)
        self.assertAlmostEqual(now[0] - t, 1.0, places=6)

    def test_eof_does_not_replay_partial_private_bytes(self):
        chunks = [b'private fragment', b'']
        stream = publisher.input_frames(0, clock=lambda: 0.0,
                    wait=lambda *args: ([0], [], []), read=lambda *args: chunks.pop(0))
        # Initial neutral fallback may precede EOF, but cannot contain the fragment.
        for frame, unavailable in stream:
            self.assertTrue(unavailable)
            self.assertNotIn(b'private fragment', frame)


if __name__ == '__main__':
    unittest.main(verbosity=2)
