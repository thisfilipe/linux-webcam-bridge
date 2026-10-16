#!/usr/bin/python3
"""Exercise a PipeWire raw source; print metadata only, retain no images."""
import argparse
import json
from pathlib import Path
import time
import zlib
import gi
gi.require_version("Gst", "1.0")
from gi.repository import Gst

Gst.init(None)
parser = argparse.ArgumentParser()
parser.add_argument("--node-name", required=True)
parser.add_argument("--state-file", type=Path, required=True)
parser.add_argument("--rounds", type=int, default=3)
parser.add_argument("--seconds", type=float, default=3)
parser.add_argument("--idle", type=float, default=7)
parser.add_argument("--hold-buffer", action="store_true")
args = parser.parse_args()
previous = json.loads(args.state_file.read_text())
for iteration in range(args.rounds):
    pipeline = Gst.parse_launch(
        f'pipewiresrc target-object={args.node_name} always-copy=false '
        '! video/x-raw,format=I420,width=1280,height=720,framerate=30/1 '
        '! appsink name=frames sync=false max-buffers=2 drop=true enable-last-sample=false'
    )
    sink = pipeline.get_by_name("frames")
    count = 0
    retained = None
    start = None
    deadline = time.monotonic() + 12
    pipeline.set_state(Gst.State.PLAYING)
    try:
        while time.monotonic() < deadline:
            message = pipeline.get_bus().pop_filtered(Gst.MessageType.ERROR)
            if message:
                raise RuntimeError(str(message.parse_error()))
            sample = sink.emit("try-pull-sample", Gst.SECOND)
            if sample is None:
                continue
            now = time.monotonic()
            if start is None:
                start = now
                deadline = now + args.seconds
            buffer = sample.get_buffer()
            if buffer.get_size() != 1280 * 720 * 3 // 2:
                raise RuntimeError(f"Invalid frame size {buffer.get_size()}")
            if args.hold_buffer and retained is None:
                retained = sample
                checksum = zlib.crc32(buffer.extract_dup(0, buffer.get_size()))
            count += 1
            sample = buffer = None
        if count < 2:
            raise RuntimeError("No continuous frames")
        if retained:
            buffer = retained.get_buffer()
            unchanged = checksum == zlib.crc32(buffer.extract_dup(0, buffer.get_size()))
        else:
            unchanged = True
        elapsed = time.monotonic() - start
        fps = (count - 1) / elapsed
        if not unchanged or fps < 28:
            raise RuntimeError(f"Buffer ownership={unchanged}, actual fps={fps:.2f}")
        print(json.dumps({"round": iteration + 1, "frames": count,
                          "seconds": round(elapsed, 3), "fps": round(fps, 2),
                          "retained_buffer_unchanged": unchanged}), flush=True)
    finally:
        retained = None
        sample = None
        pipeline.set_state(Gst.State.NULL)
    if iteration + 1 < args.rounds:
        time.sleep(args.idle)
        current = json.loads(args.state_file.read_text())
        if not current["capture_active"] or current["capture_sessions"] != previous["capture_sessions"] or current["publisher_sessions"] != previous["publisher_sessions"] or current["frames"] <= previous["frames"]:
            raise RuntimeError(f"Producer did not survive client removal: {current}")
        print(json.dumps({"after_idle": current}), flush=True)
        previous = current
