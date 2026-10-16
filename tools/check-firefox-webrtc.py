#!/usr/bin/python3
"""Measure real Jieli WebRTC frames using an isolated Firefox profile.

Only metadata is posted to a server bound to 127.0.0.1; no frames are saved.
The normal Firefox profile and its permissions are not opened or changed.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading

PAGE = b'''<!doctype html><meta charset="utf-8"><video autoplay muted playsinline></video>
<script>
(async () => {
  let stream;
  const result = {rounds: []};
  const video = document.querySelector('video');
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    result.cameras = devices.filter(d => d.kind === 'videoinput').map(d => d.label);
    for (let round = 0; round < 2; round++) {
      stream = await navigator.mediaDevices.getUserMedia({audio: false, video: {
        width: {exact: 1280}, height: {exact: 720},
        frameRate: {ideal: 30, max: 30}
      }});
      const label = stream.getVideoTracks()[0].label;
      if (label !== 'Jieli 720p30 V4L2') throw new Error('Unexpected real device: ' + label);
      result.device = label;
      video.srcObject = stream;
      await video.play();
      await sleep(2000);
      const settings = stream.getVideoTracks()[0].getSettings();
      const before = video.getVideoPlaybackQuality().totalVideoFrames;
      let callbacks = 0;
      let collecting = true;
      let metadata;
      const onFrame = (now, frame) => {
        if (!collecting) return;
        callbacks++;
        metadata = {width: frame.width, height: frame.height, mediaTime: frame.mediaTime};
        video.requestVideoFrameCallback(onFrame);
      };
      const callbackId = video.requestVideoFrameCallback(onFrame);
      const start = performance.now();
      await sleep(8000);
      collecting = false;
      video.cancelVideoFrameCallback(callbackId);
      const seconds = (performance.now() - start) / 1000;
      const frames = video.getVideoPlaybackQuality().totalVideoFrames - before;
      result.rounds.push({width: settings.width, height: settings.height,
        negotiated_fps: settings.frameRate, playback_quality_frames: frames,
        frames: callbacks, seconds, fps: callbacks / seconds, last_frame: metadata});
      stream.getTracks().forEach(t => t.stop());
      stream = null;
      video.srcObject = null;
      await sleep(1000);
    }
    result.passed = result.rounds.every(r => r.width === 1280 && r.height === 720 && r.fps >= 28);
  } catch (error) {
    result.error = String(error);
    result.passed = false;
  } finally {
    if (stream) stream.getTracks().forEach(t => t.stop());
    await fetch('/result', {method: 'POST', body: JSON.stringify(result)});
  }
})();
</script>'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--firefox', default='firefox', help='Firefox executable (not a Flatpak command)')
    parser.add_argument('--output', type=Path, help='Optional metadata JSON; no camera frames')
    args = parser.parse_args()
    done = threading.Event()
    results = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(PAGE)

        def do_POST(self):
            length = min(int(self.headers.get("Content-Length", "0")), 16384)
            results.append(json.loads(self.rfile.read(length)))
            self.send_response(200)
            self.end_headers()
            done.set()

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="jieli-firefox-") as directory:
        profile = Path(directory)
        preferences = {
            "media.navigator.permission.disabled": True,
            "media.navigator.streams.fake": False,
            "media.video_loopback_dev": "Jieli 720p30 V4L2",
            "media.webrtc.camera.allow-pipewire": False,
            "browser.shell.checkDefaultBrowser": False,
            "browser.aboutwelcome.enabled": False,
            "browser.startup.homepage": "about:blank",
            "datareporting.policy.dataSubmissionEnabled": False,
        }
        (profile / "user.js").write_text("\n".join(
            f"user_pref({json.dumps(key)}, {json.dumps(value)});" for key, value in preferences.items()))
        with (profile / "browser.log").open("w+") as log:
            try:
                for attempt in range(1):
                    done.clear()
                    process = subprocess.Popen(
                        [args.firefox, "--headless", "--no-remote", "--profile", directory,
                         f"http://127.0.0.1:{server.server_port}/"],
                        stdout=log, stderr=log, start_new_session=True)
                    try:
                        if not done.wait(35):
                            raise RuntimeError("Browser did not return metadata within 35 s")
                        result = results[-1]
                    finally:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                    break
                print(json.dumps(result, ensure_ascii=False), flush=True)
                if args.output:
                    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
                if not result.get("passed"):
                    raise SystemExit(1)
            finally:
                server.shutdown()


if __name__ == "__main__":
    main()
