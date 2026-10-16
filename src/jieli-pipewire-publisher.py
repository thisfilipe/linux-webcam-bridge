#!/usr/bin/python3
"""Publish fixed-size I420 stdin frames to PipeWire and optional V4L2 loopback."""
import argparse
import fcntl
import os
from pathlib import Path
import signal
import select
import sys
import threading
import time
import gi
gi.require_version("Gst", "1.0")
from gi.repository import GLib, Gst

Gst.init(None)
FRAME_SIZE = 1280 * 720 * 3 // 2
CACHE_DIR = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "jieli-camera"
CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "jieli-camera"


def unavailable_frame():
    """Neutral I420 with raster text; no image files or optional font plugins."""
    glyphs = {
        "A": [14,17,17,31,17,17,17], "B": [30,17,17,30,17,17,30],
        "C": [14,17,16,16,16,17,14], "D": [30,17,17,17,17,17,30],
        "E": [31,16,16,30,16,16,31], "I": [31,4,4,4,4,4,31],
        "G": [14,17,16,23,17,17,15],
        "L": [16,16,16,16,16,16,31], "M": [17,27,21,21,17,17,17],
        "N": [17,25,25,21,19,19,17], "O": [14,17,17,17,17,17,14],
        "P": [30,17,17,30,16,16,16], "R": [30,17,17,30,20,18,17],
        "S": [15,16,16,14,1,1,30], "T": [31,4,4,4,4,4,4],
        "U": [17,17,17,17,17,17,14], "V": [17,17,17,17,17,10,4],
    }
    y = bytearray([32]) * (1280 * 720)
    for text, top in (("CAMERA UNAVAILABLE", 312), ("RECONNECTING", 380)):
        left = (1280 - len(text) * 24) // 2
        for index, char in enumerate(text):
            for row, bits in enumerate(glyphs.get(char, [0] * 7)):
                for column in range(5):
                    if bits & (1 << (4 - column)):
                        for dy in range(4):
                            offset = (top + row * 4 + dy) * 1280 + left + index * 24 + column * 4
                            y[offset:offset + 4] = bytes([235]) * 4
    return bytes(y) + bytes([128]) * (1280 * 720 // 2)


def input_frames(fd, clock=time.monotonic, wait=select.select, read=os.read):
    """Yield owned real bytes or fresh neutral frames after 250 ms starvation.

    The last real frame is never retained for replay. Partial pipe reads remain
    private to this receiver; EOF terminates publication with the bridge.
    """
    fallback = unavailable_frame()
    data = bytearray()
    last_real = None
    next_fallback = clock()
    while True:
        now = clock()
        deadline = next_fallback if last_real is None else max(next_fallback, last_real + 0.25)
        ready, _, _ = wait([fd], [], [], max(0, deadline - now))
        if ready:
            chunk = read(fd, FRAME_SIZE - len(data))
            if not chunk:
                return
            data.extend(chunk)
            if len(data) == FRAME_SIZE:
                frame = bytes(data)
                data.clear()
                last_real = clock()
                next_fallback = last_real + 0.25
                yield frame, False
                continue
        now = clock()
        if (last_real is None or now - last_real >= 0.25) and now >= next_fallback:
            next_fallback = now + 1 / 30
            yield fallback, True


def main():
    try:
        fcntl.fcntl(sys.stdin.fileno(), fcntl.F_SETPIPE_SZ, 1024 * 1024)
    except OSError:
        pass
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node-name", required=True)
    parser.add_argument("--description", required=True)
    args = parser.parse_args()
    pid_file = Path(os.environ.get("XDG_RUNTIME_DIR", CACHE_DIR / "runtime")) / f"jieli-publisher-{args.node_name}.pid"
    pid_file.parent.mkdir(parents=True, exist_ok=True)
    pid_file.write_text(str(os.getpid()) + "\n")
    print(f"publisher pid={os.getpid()}, plugin={Gst.ElementFactory.find('pipewiresink').get_plugin().get_filename()}", flush=True)
    loop = GLib.MainLoop()
    pipeline = Gst.parse_launch(
        'appsrc name=frames is-live=true do-timestamp=true format=time '
        'block=false max-buffers=2 max-bytes=0 max-time=0 leaky-type=downstream '
        'caps="video/x-raw,format=I420,width=1280,height=720,framerate=30/1" '
        '! queue leaky=downstream max-size-buffers=2 max-size-bytes=0 max-size-time=0 '
        '! pipewiresink name=pw mode=provide use-bufferpool=false '
        'sync=false async=false enable-last-sample=false'
    )
    source = pipeline.get_by_name("frames")
    sink = pipeline.get_by_name("pw")
    props = Gst.Structure.new_empty("props")
    for key, value in {
        "media.class": "Video/Source", "media.role": "Camera",
        "node.name": args.node_name, "node.description": args.description,
        "node.virtual": True, "node.pause-on-idle": False,
        "priority.session": 1000,
    }.items():
        props.set_value(key, value)
    sink.set_property("stream-properties", props)
    failed = False
    received_frames = 0
    placeholder_frames = 0


    class V4L2Output:
        """Optional loopback output; errors never stop the PipeWire pipeline."""
        label = "Jieli 720p30 V4L2"

        def __init__(self):
            self.pipeline = self.source = self.device = None
            self.retry_after = 0.0
            self.pushed = 0

        def find_device(self):
            if (CONFIG_DIR / "disable-v4l2").exists():
                return None
            for entry in Path("/sys/class/video4linux").glob("video*"):
                try:
                    label = (entry / "name").read_text().strip()
                    if label != self.label:
                        continue
                    if not entry.resolve().is_relative_to("/sys/devices/virtual/video4linux"):
                        continue
                    device = Path("/dev") / entry.name
                    if os.access(device, os.R_OK | os.W_OK):
                        return device
                except OSError:
                    pass
            return None

        def stop(self):
            self.source = None
            pipeline, self.pipeline = self.pipeline, None
            self.device = None
            if pipeline is not None:
                pipeline.set_state(Gst.State.NULL)
                pipeline.get_bus().remove_signal_watch()

        def message(self, bus, message):
            if message.type == Gst.MessageType.ERROR:
                error, debug = message.parse_error()
                print(f"V4L2 output unavailable: {error.message}; {debug}; PipeWire continues", flush=True)
                self.stop()
                self.retry_after = time.monotonic() + 30

        def tick(self):
            device = self.find_device()
            if self.pipeline is not None and device != self.device:
                self.stop()
            if self.pipeline is None and device and time.monotonic() >= self.retry_after:
                pipeline = Gst.parse_launch(
                    'appsrc name=virtual is-live=true do-timestamp=true format=time '
                    'block=false max-buffers=2 max-bytes=0 max-time=0 leaky-type=downstream '
                    'caps="video/x-raw,format=I420,width=1280,height=720,framerate=30/1,'
                    'pixel-aspect-ratio=1/1,interlace-mode=progressive,colorimetry=bt601" '
                    '! queue leaky=downstream max-size-buffers=2 max-size-bytes=0 max-size-time=0 '
                    f'! v4l2sink name=virtualsink device="{device}" io-mode=mmap sync=false async=false '
                    'enable-last-sample=false'
                )
                bus = pipeline.get_bus()
                bus.add_signal_watch()
                bus.connect("message", self.message)
                self.pipeline = pipeline
                self.device = device
                self.source = pipeline.get_by_name("virtual")
                if pipeline.set_state(Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE:
                    self.stop()
                    self.retry_after = time.monotonic() + 30
                else:
                    print(f"V4L2 output: {device}, 1280x720 I420 30 fps", flush=True)
            return True

        def push(self, buffer):
            source = self.source
            if source is not None:
                source.emit("push-buffer", buffer.copy())
                self.pushed += 1


    virtual_camera = V4L2Output() if args.node_name == "jieli_720p30" else None


    def bus_message(bus, message):
        nonlocal failed
        if message.type == Gst.MessageType.ERROR:
            error, debug = message.parse_error()
            print(f"publisher: {error.message}", flush=True)
            failed = True
            loop.quit()
        elif message.type == Gst.MessageType.EOS:
            loop.quit()


    def receive():
        nonlocal received_frames, placeholder_frames
        for data, unavailable in input_frames(sys.stdin.fileno()):
            if unavailable:
                placeholder_frames += 1
            else:
                received_frames += 1
            buffer = Gst.Buffer.new_allocate(None, len(data), None)
            # Preserve the direct bytes path; bytearray conversion costs a full frame.
            buffer.fill(0, data)
            buffer.duration = Gst.SECOND // 30
            source.emit("push-buffer", buffer)
            if virtual_camera is not None:
                virtual_camera.push(buffer)
        GLib.idle_add(loop.quit)


    def quit_signal():
        loop.quit()
        return False


    def debug_signal():
        Gst.debug_set_threshold_from_string("pipewirepool:7,pipewiresink:5", False)
        def disable():
            Gst.debug_set_threshold_from_string("*:0", True)
            return False
        GLib.timeout_add_seconds(20, disable)
        return True


    def report_stats():
        print(f"publisher received={received_frames}, placeholder={placeholder_frames}, sink={sink.get_property('stats').to_string()}", flush=True)
        if virtual_camera is not None and virtual_camera.pipeline is not None:
            output = virtual_camera.pipeline.get_by_name("virtualsink")
            print(f"V4L2 pushed={virtual_camera.pushed}, sink={output.get_property('stats').to_string()}", flush=True)
        return True


    bus = pipeline.get_bus()
    bus.add_signal_watch()
    bus.connect("message", bus_message)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, quit_signal)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, quit_signal)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1, debug_signal)
    GLib.timeout_add_seconds(10, report_stats)
    if (CACHE_DIR / f"debug-{args.node_name}").exists():
        debug_signal()
    pipeline.set_state(Gst.State.PLAYING)
    if virtual_camera is not None:
        virtual_camera.tick()
        GLib.timeout_add_seconds(2, virtual_camera.tick)
    threading.Thread(target=receive, name="raw-input", daemon=True).start()
    try:
        loop.run()
    finally:
        if virtual_camera is not None:
            virtual_camera.stop()
        pipeline.set_state(Gst.State.NULL)
        pid_file.unlink(missing_ok=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
