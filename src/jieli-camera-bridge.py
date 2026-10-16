#!/usr/bin/python3
"""Keep one physical capture and an independent, persistent virtual camera.

Only owned raw bytes cross into the publisher. Hardware transitions run serially
outside GLib; logind's delay lease bounds sleep preparation, never the UI thread.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import threading
import time

import gi
gi.require_version("Gst", "1.0")
from gi.repository import Gio, GLib, Gst

Gst.init(None)
CAPS = "video/x-raw,format=I420,width=1280,height=720,framerate=30/1"
DEVICE = Path("/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0")
RESET = ("/usr/bin/sudo", "-n", "/usr/local/libexec/jieli-usb-reset")
MAX_RESETS = 3
CACHE_DIR = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "jieli-camera"


def background(work, complete):
    def run():
        try:
            result, error = work(), None
        except Exception as exc:
            result, error = None, exc
        GLib.idle_add(complete, result, error)
    threading.Thread(target=run, daemon=True, name="camera-transition").start()


class SleepMonitor:
    """Own a logind delay FD; an independent timer releases a hung teardown."""
    def __init__(self, bridge):
        self.bridge = bridge
        self.proxy = None
        self.fd = None
        self.pending = False
        self.generation = 0
        self.limit = None
        self.timer = None
        self.lock = threading.Lock()
        self.error = None

    def start(self):
        def connect():
            proxy = Gio.DBusProxy.new_for_bus_sync(
                Gio.BusType.SYSTEM, Gio.DBusProxyFlags.NONE, None,
                "org.freedesktop.login1", "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager", None)
            limit = proxy.get_cached_property("InhibitDelayMaxUSec")
            if limit is None or limit.unpack() <= 0:
                raise RuntimeError("logind did not provide InhibitDelayMaxUSec")
            return proxy, limit.unpack() / 1_000_000
        def connected(result, error):
            if self.bridge.stopping:
                return False
            if error:
                self.error = f"logind unavailable: {error}"
                self.bridge.log(self.error)
            else:
                self.proxy, self.limit = result
                self.proxy.connect("g-signal", self.signal)
                self.acquire()
            return False
        background(connect, connected)

    def acquire(self):
        if self.proxy is None or self.pending or self.fd is not None:
            return
        self.pending = True
        token = self.generation
        def request():
            value, descriptors = self.proxy.call_with_unix_fd_list_sync(
                "Inhibit", GLib.Variant("(ssss)", (
                    "sleep", "Jieli camera bridge", "Close webcam before sleep", "delay")),
                Gio.DBusCallFlags.NONE, 2000, None, None)
            return descriptors.get(value.unpack()[0])
        def acquired(fd, error):
            if token != self.generation or self.bridge.stopping or self.bridge.sleeping:
                if fd is not None:
                    os.close(fd)
                return False
            self.pending = False
            if error:
                self.error = f"sleep delay inhibitor unavailable: {error}"
                self.bridge.log(self.error)
            else:
                self.fd = fd
                self.error = None
            return False
        background(request, acquired)

    def release(self, expected_generation=None):
        with self.lock:
            if expected_generation is not None and expected_generation != self.generation:
                return False
            timer, self.timer = self.timer, None
            fd, self.fd = self.fd, None
        if timer is not None:
            timer.cancel()
        if fd is not None:
            os.close(fd)
        return True

    def signal(self, proxy, sender, name, parameters):
        if name != "PrepareForSleep":
            return
        sleeping = parameters.unpack()[0]
        self.generation += 1
        self.pending = False
        if sleeping:
            # Timer is not on GLib: even a stuck driver cannot hold the lease.
            if self.fd is not None:
                budget = max(0.001, self.limit - min(0.1, self.limit * 0.1))
                token = self.generation
                def expired():
                    if self.release(token):
                        GLib.idle_add(self.bridge.sleep_deadline)
                self.timer = threading.Timer(budget, expired)
                self.timer.daemon = True
                self.timer.start()
            self.bridge.prepare_sleep()
        else:
            self.release()
            self.bridge.resume()
            self.acquire()

    def close(self):
        self.generation += 1
        self.release()


class ResetUnavailable(RuntimeError):
    pass


class Bridge:
    def __init__(self, args):
        self.args = args
        self.loop = GLib.MainLoop()
        self.capture = self.publisher = self.publish_queue = None
        self.publisher_stop = None
        self.lock = threading.Lock()
        self.hardware_lock = threading.Lock()
        self.identity = self.blocked = self.seen = None
        self.capture_sessions = self.publisher_sessions = self.frames = 0
        self.started = self.last_frame = self.last_report = 0.0
        self.report_frames = 0
        self.fps = 0.0
        self.error = None
        self.publisher_retry = 0.0
        self.stopping = self.sleeping = False
        self.capture_generation = self.recovery_generation = 0
        self.capture_closing = self.capture_starting = self.reset_pending = False
        self.close_failed = None
        self.recovery = "starting"
        self.retries = 0
        self.retry_at = 0.0
        self.reset_required = False
        self.reset_barrier = False
        self.settle_after = self.settle_deadline = 0.0
        self.settle_seen = None
        self.last_reset = None
        self.sleep = None if args.test_source else SleepMonitor(self)

    def log(self, text):
        print(text, flush=True)

    def hardware(self, work, complete):
        def serialized():
            with self.hardware_lock:
                return work()
        background(serialized, complete)

    def device_identity(self):
        if self.args.test_source:
            return "synthetic"
        try:
            info = DEVICE.stat()
            return f"{DEVICE.resolve()}:{info.st_ino}:{info.st_rdev}"
        except OSError:
            return None

    def bus_message(self, bus, message, pipeline, generation):
        if pipeline is not self.capture or generation != self.capture_generation:
            return
        if message.type == Gst.MessageType.ERROR:
            error, debug = message.parse_error()
            self.recover(f"capture: {error.message}")
        elif message.type == Gst.MessageType.EOS:
            self.recover("capture: end of stream")

    def start_capture(self, identity):
        if self.args.test_source:
            source = f"videotestsrc is-live=true pattern=ball ! {CAPS}"
        else:
            source = (f'v4l2src device="{DEVICE}" io-mode=mmap do-timestamp=true '
                      '! image/jpeg,width=1280,height=720,framerate=30/1 '
                      f'! jpegparse ! jpegdec ! videoconvert ! {CAPS}')
        try:
            pipeline = Gst.parse_launch(source +
                ' ! appsink name=frames emit-signals=true sync=false '
                'max-buffers=2 drop=true enable-last-sample=false')
        except GLib.Error as exc:
            self.recover(f"capture construction failed: {exc}")
            return
        self.capture_generation += 1
        generation = self.capture_generation
        self.capture, self.identity = pipeline, identity
        pipeline.get_by_name("frames").connect("new-sample", self.new_sample, generation)
        bus = pipeline.get_bus()
        bus.add_signal_watch()
        bus.connect("message", self.bus_message, pipeline, generation)
        self.frames = self.report_frames = 0
        self.started = self.last_report = time.monotonic()
        self.last_frame = self.fps = 0.0
        self.capture_sessions += 1
        self.capture_starting = True
        self.recovery = "opening"
        self.log(f"Starting capture session {self.capture_sessions}: 720p30")
        def started(result, error):
            if generation == self.capture_generation and pipeline is self.capture:
                self.capture_starting = False
                if error or result == Gst.StateChangeReturn.FAILURE:
                    self.recover(f"Capture failed to enter PLAYING: {error or result}")
                else:
                    self.recovery = "waiting-frames"
            return False
        self.hardware(lambda: pipeline.set_state(Gst.State.PLAYING), started)

    def new_sample(self, sink, generation):
        sample = sink.emit("pull-sample")
        if sample is None:
            return Gst.FlowReturn.EOS
        # Extraction precedes the lock; generation is checked after copying too.
        buffer = sample.get_buffer()
        frame = buffer.extract_dup(0, buffer.get_size())
        with self.lock:
            if generation != self.capture_generation or self.sleeping or self.stopping:
                return Gst.FlowReturn.OK
            self.frames += 1
            self.last_frame = time.monotonic()
            target = self.publish_queue
            if target is not None:
                try:
                    target.get_nowait()
                except queue.Empty:
                    pass
                target.put_nowait((generation, frame))
        return Gst.FlowReturn.OK

    def start_publisher(self):
        environment = os.environ.copy()
        plugin = Path.home() / ".local/lib/jieli-gstreamer"
        if (plugin / "libgstpipewire.so").exists():
            environment["GST_PLUGIN_PATH_1_0"] = str(plugin)
            cache = CACHE_DIR
            cache.mkdir(parents=True, exist_ok=True)
            environment["GST_REGISTRY_1_0"] = str(cache / "gst-registry.bin")
        process = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("jieli-pipewire-publisher.py")),
             "--node-name", self.args.node_name,
             "--description", "Jieli validation" if self.args.test_source else "Jieli 720p30"],
            stdin=subprocess.PIPE, bufsize=0, env=environment,
        )
        target = queue.Queue(maxsize=1)
        stopping = threading.Event()
        self.publisher, self.publisher_stop = process, stopping
        with self.lock:
            self.publish_queue = target

        def send_frames():
            try:
                while not stopping.is_set():
                    try:
                        generation, data = target.get(timeout=0.5)
                        if generation != self.capture_generation:
                            continue
                        frame = memoryview(data)
                    except queue.Empty:
                        continue
                    while frame and not stopping.is_set():
                        frame = frame[os.write(process.stdin.fileno(), frame):]
            except (OSError, ValueError):
                pass

        threading.Thread(target=send_frames, name="publisher-input", daemon=True).start()
        self.publisher_sessions += 1
        self.log(f"Publishing {self.args.node_name}, session {self.publisher_sessions}")

    def stop_publisher(self):
        with self.lock:
            self.publish_queue = None
        process, self.publisher = self.publisher, None
        if process is not None:
            self.publisher_stop.set()
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            process.stdin.close()

    def stop_capture(self):
        with self.lock:
            pipeline, self.capture = self.capture, None
            self.capture_generation += 1
            if self.publish_queue is not None:
                try:
                    self.publish_queue.get_nowait()
                except queue.Empty:
                    pass
        self.capture_starting = False
        if pipeline is None:
            if self.sleeping and not self.capture_closing and not self.reset_pending and self.sleep:
                self.sleep.release()
            return
        self.capture_closing = True
        pipeline.get_bus().remove_signal_watch()
        def close():
            pipeline.set_state(Gst.State.NULL)
            if pipeline.get_state(0)[1] != Gst.State.NULL:
                raise RuntimeError("capture teardown did not reach NULL")
        def closed(result, error):
            self.capture_closing = False
            if error:
                # Retain ownership; never reset/reopen over an unclosed pipeline.
                self.close_failed = pipeline
                self.error = f"Capture teardown failed: {error}; restart bridge required"
                self.log(self.error)
            if self.sleeping and not self.reset_pending and self.sleep:
                self.sleep.release()
            return False
        self.hardware(close, closed)

    def prepare_sleep(self):
        self.sleeping = True
        self.recovery_generation += 1
        self.recovery = "sleeping"
        self.stop_capture()

    def sleep_deadline(self):
        if self.sleeping and self.capture_closing:
            self.error = "Capture teardown exceeded logind sleep delay; recovery waits for teardown"
            self.log(self.error)
        return False

    def resume(self):
        self.sleeping = False
        self.recover("Resuming camera after system sleep", fresh=True)

    def recover(self, reason, fresh=False):
        self.error = reason
        self.log(reason)
        self.recovery_generation += 1
        if fresh:
            self.retries = 0
            self.blocked = None
        self.reset_required = not self.args.test_source
        self.retry_at = time.monotonic() + (min(32, 8 * 2 ** (self.retries - 1)) if self.retries else 0)
        self.recovery = "backoff"
        self.stop_capture()

    @staticmethod
    def reset_usb():
        if not Path(RESET[2]).is_file():
            raise ResetUnavailable(f"Reset helper missing: {RESET[2]}")
        try:
            result = subprocess.run(RESET, capture_output=True, text=True, timeout=12)
        except FileNotFoundError as exc:
            raise ResetUnavailable("sudo or reset helper missing") from exc
        text = (result.stderr + result.stdout).strip()[:1000]
        if result.returncode and ("sudo:" in text or "permission denied" in text.lower()):
            raise ResetUnavailable(f"Reset helper not authorized: {text}")
        # Any nonzero status keeps capture closed; a timed-out root ioctl may
        # still own the helper lock. Only a subsequent rc=0 clears the barrier.
        return {"returncode": result.returncode, "message": text}

    def begin_reset(self):
        if self.retries >= MAX_RESETS:
            self.recovery = "exhausted"
            self.blocked = self.device_identity() or self.identity or "missing"
            self.error = f"Camera recovery exhausted after {MAX_RESETS} resets; reconnect camera or restart bridge"
            self.log(self.error)
            return
        self.retries += 1
        self.reset_pending = True
        self.reset_barrier = True
        self.recovery = "resetting"
        token = self.recovery_generation
        def reset_done(result, error):
            self.reset_pending = False
            if self.sleeping and not self.capture_closing and self.sleep:
                self.sleep.release()
            if token != self.recovery_generation or self.sleeping or self.stopping:
                return False
            if isinstance(error, ResetUnavailable):
                self.recovery = "helper-unavailable"
                self.blocked = self.device_identity() or self.identity or "missing"
                self.error = str(error)
                self.log(self.error)
            elif error:
                self.recover(f"USB reset failed: {error}")
            elif result["returncode"] != 0:
                self.last_reset = result
                self.recover(f"USB reset helper returned {result['returncode']}: {result['message']}")
            else:
                self.last_reset = result
                self.reset_barrier = False
                self.reset_required = False
                self.recovery = "settling"
                self.settle_after = time.monotonic() + 1
                self.settle_deadline = time.monotonic() + 10
                self.settle_seen = None
            return False
        self.hardware(self.reset_usb, reset_done)

    def tick(self):
        if self.stopping:
            return False
        now = time.monotonic()
        if self.publisher is not None and self.publisher.poll() is not None:
            self.log("Publisher exited; capture remains independent")
            self.stop_publisher()
            self.publisher_retry = now + 1
        if self.publisher is None and now >= self.publisher_retry:
            try:
                self.start_publisher()
            except OSError as exc:
                self.error = f"Publisher unavailable: {exc}"
                self.publisher_retry = now + 8
        identity = self.device_identity()
        if self.capture is not None and identity != self.identity:
            self.recover("Camera disconnected; waiting for USB recovery", fresh=True)
        if self.blocked is not None and identity and identity != self.blocked:
            self.blocked = None
            self.retries = 0
            self.reset_required = self.reset_barrier
            self.recovery = "starting"
        ready = (not self.sleeping and not self.capture_closing and not self.close_failed
                 and not self.reset_pending and self.capture is None)
        inhibited = self.sleep is None or self.sleep.fd is not None
        if ready and inhibited and self.recovery not in ("helper-unavailable", "exhausted"):
            if self.recovery == "settling":
                if identity and identity == self.settle_seen and now >= self.settle_after:
                    self.start_capture(identity)
                elif now >= self.settle_deadline:
                    self.recover("USB node did not stabilize after reset")
                self.settle_seen = identity
            elif now >= self.retry_at:
                if self.reset_required:
                    # The USB helper identifies VID/PID even when V4L2 is absent.
                    self.begin_reset()
                elif identity and identity == self.seen:
                    self.start_capture(identity)
        self.seen = identity
        if self.capture is not None:
            if now - (self.last_frame or self.started) > 12:
                self.recover("No decoded frames for 12 s; attempting bounded USB recovery")
            elif (self.last_frame and now - self.last_frame < 0.5
                  and now - self.started >= 10 and self.frames >= 90):
                self.retries = 0
                self.error = None
                self.recovery = "streaming"
            if now - self.last_report >= 10:
                self.fps = (self.frames - self.report_frames) / (now - self.last_report)
                self.report_frames, self.last_report = self.frames, now
                self.log(f"720p: {self.fps:.2f} fps, decoded={self.frames}, capture_sessions={self.capture_sessions}, publisher_sessions={self.publisher_sessions}")
        status = {
            "pid": os.getpid(), "node_name": self.args.node_name,
            "capture_active": self.capture is not None,
            "capture_closing": self.capture_closing,
            "capture_starting": self.capture_starting,
            "publisher_active": self.publisher is not None,
            "capture_sessions": self.capture_sessions, "publisher_sessions": self.publisher_sessions,
            "frames": self.frames, "fps_10s": round(self.fps, 3),
            "seconds_since_frame": round(now - self.last_frame, 3) if self.last_frame else None,
            "sleeping": self.sleeping, "recovery": self.recovery,
            "retries": self.retries, "retry_in_seconds": round(max(0, self.retry_at - now), 2),
            "reset_pending": self.reset_pending, "reset_barrier": self.reset_barrier,
            "last_reset": self.last_reset,
            "sleep_inhibited": inhibited, "sleep_monitor_error": self.sleep.error if self.sleep else None,
            "placeholder": self.sleeping or self.capture is None or not self.last_frame or now - self.last_frame > 0.25,
            "error": self.error or (self.sleep.error if self.sleep else None),
        }
        self.args.state_file.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.args.state_file.with_suffix(".tmp")
        temporary.write_text(json.dumps(status) + "\n")
        temporary.replace(self.args.state_file)
        return True

    def quit(self, *unused):
        self.stopping = True
        self.loop.quit()
        return False

    def run(self):
        if self.sleep:
            self.sleep.start()
        GLib.timeout_add_seconds(1, self.tick)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, self.quit)
        GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, self.quit)
        try:
            self.loop.run()
        finally:
            self.stop_capture()
            if self.sleep:
                self.sleep.close()
            self.stop_publisher()
            self.args.state_file.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-source", action="store_true")
    parser.add_argument("--node-name", default="jieli_720p30")
    parser.add_argument("--state-file", type=Path, default=Path(os.environ.get("XDG_RUNTIME_DIR", CACHE_DIR / "runtime")) / "jieli-camera-bridge.json")
    args = parser.parse_args()
    if args.test_source and args.node_name == "jieli_720p30":
        args.node_name = "jieli_validation"
    Bridge(args).run()


if __name__ == "__main__":
    main()
