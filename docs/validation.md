# Validation

## Reference hardware results

These results describe the working deployment from which the project was
extracted. No image, USB trace, private runtime JSON, hostname, account path or
complete system journal is included in this repository. Software versions are
recorded to bound the claim, not as minimum guarantees for every distribution.

| Component | Tested version |
| --- | --- |
| Camera | Jieli Technology USB PHY 2.0, `1224:2a25` |
| OS / desktop | Pop!_OS based on Ubuntu 24.04, GNOME 46 Wayland |
| Kernel | `7.1.5-76070105-generic` |
| PipeWire / WirePlumber | 1.6.8 / 0.5.12 |
| GStreamer / Python | 1.24.2 / 3.12 |
| Snapshot | Flatpak stable 51.0 |
| Firefox | 157.0.1, isolated test profile |
| v4l2loopback | 0.15.3, distribution DKMS package |

| Scenario | Observation |
| --- | --- |
| Continuous physical capture | Decoded 1280×720 near 30 FPS; image fluid for several minutes |
| Snapshot close/reopen | Working image after independent app reopening |
| Integrated ↔ Jieli switching | Repeated switching passed; integrated camera unaffected |
| Direct V4L2 repeated open | Three 120-frame sessions: 30.10, 30.00, 29.78 FPS |
| Simultaneous PipeWire consumer | 29.68 FPS during V4L2 validation |
| Physical USB reconnect | Automatic recovery; PipeWire 30.06, V4L2 30.02, decoded 30.00 FPS |
| Real Firefox WebRTC | Two genuine getUserMedia sessions, 1280×720 negotiated at 30 FPS |
| Firefox frame presentation | Headless requestVideoFrameCallback measured 28.50 and 28.75 FPS |
| Suspend/resume | Publisher identity survived; physical capture reset/reopened; user confirmed image |
| Hibernate/resume | Same publisher survived; another capture session opened; user confirmed image |
| Clean reboot | Deferred by the tester; **unverified** |
| Remote conferencing call | **Unverified** |
| Other models / multiple identical cameras | **Unverified / unsupported** |

The suspend and hibernate checks used the restricted helper, with no manual
USB reset on resume. Capture session count advanced while publisher session
count stayed at one. Those checks do not substitute for a clean-boot test.

## Automated checks: no camera or root actions

Use the distribution Python with PyGObject available:

```sh
python3 -m unittest discover -s tests -p 'test_*.py'
```

The tests load repository source and use mocks/temporary files. They cover
capture teardown ordering, publisher survival, stale frames and callbacks,
sleep delay release, reset authorization/failures/timeouts, retry budgets,
neutral placeholders, helper device/FD checks and installer conflicts.
They do not connect to D-Bus, reset USB, load modules or change services.

After preparing a source tree:

```sh
python3 tools/build-plugin.py --source-only --build-dir build/regression
python3 tests/check_pool.py --build-dir build/regression
```

The C harness extracts the real `acquire_buffer()` function from original and
patched source and compiles it with deterministic fake dequeue/wait calls.
The original is expected to fail the busy-then-free regression; the patched
function must pass all 17 scenarios. This checks bounded scanning, silent
release, `EBUSY` / `EPIPE` / `EAGAIN`, other errors, flushing, nonblocking and
paused paths, and exact active/idle retry deadlines. It is not a substitute
for full threaded PipeWire integration testing.

CI runs the mocked tests and this source regression without any camera.
The full plugin build is also validated locally against matching development
headers and installed runtime libraries. CI does not provision PipeWire 1.6.8
or claim a full plugin runtime/hardware test.

## Opt-in checks against your running installation

These access live video in memory. They save no images. Close applications
that are using the raw physical camera; use the shared source instead.

### Shared PipeWire source

```sh
python3 tools/jieli-consumer-check.py \
  --node-name jieli_720p30 \
  --state-file "$XDG_RUNTIME_DIR/jieli-camera-bridge.json" \
  --rounds 3 --seconds 4 --idle 7 --hold-buffer
```

This checks I420 frame size, measured cadence, retained-buffer integrity and
that consumer close/reopen does not recreate physical capture or publication.
Status output may contain local runtime metadata; sanitize before publishing.

### Direct V4L2 source

When optional loopback is installed and the publisher reports it active:

```sh
v4l2-ctl --device=/dev/video10 --get-fmt-video
v4l2-ctl --device=/dev/video10 --stream-mmap=3 --stream-count=120 \
  --stream-poll --stream-to=/dev/null
```

Repeat the latter command three times. It opens the **virtual** device, not
the physical Jieli. Also confirm a real app shows moving, intact video; sink
statistics alone do not establish functional capture.

### Firefox WebRTC

```sh
python3 tools/check-firefox-webrtc.py
```

Requires a native Firefox executable and the optional V4L2 device. The check
creates an isolated temporary profile, forces the real `Jieli 720p30 V4L2`
device, disables fake streams and opens it twice. It counts actual video
presentation callbacks, verifies dimensions and requires at least 28 FPS.
It posts metadata only to an ephemeral `127.0.0.1` server. The normal profile
is never opened; temporary browser permissions are deleted afterward.
Use `--firefox /path/to/firefox` or `--output result.json` if desired.
Browser behavior/preferences can change in later releases.

### Manual desktop checklist

1. Select **Jieli 720p30** in Snapshot and watch moving video for several minutes.
2. Switch to the integrated camera and back at least twice.
3. Close the app and reopen it independently.
4. Disconnect and reconnect the Jieli; verify automatic recovery.
5. When ready, suspend the system yourself, resume, then repeat steps 1–3.
6. If hibernation already works on your system, test it yourself and repeat.
7. At a convenient time, perform a clean reboot and report the separate result.

The test scripts never initiate system sleep, hibernate or reboot.

## Repository packaging validation

The public extraction changes install paths, English placeholder text and
installer interfaces, not the recovery design. Mocked tests, the pinned-source
patch regression, isolated plugin build/link/load checks and publication audit
are run on the extracted tree. The working deployment is preserved throughout;
the portable installers have not been root-installed over it.

The initial public extraction passed **76 Python tests**, the **17-scenario C
regression** (with the unpatched baseline failing as expected), syntax checks,
user-unit verification, a fresh official archive download/checksum, and the
full isolated plugin compile/link/load check. Publication review checks the
tracked text and commit identity; generated binaries, local logs and private
diagnostic reports are excluded.
