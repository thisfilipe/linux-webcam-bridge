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
| GStreamer | 1.24.2 |
| Rust | 1.92+ (helper build pinned to 1.99.0) |
| Snapshot | Flatpak stable 51.0 |
| v4l2loopback | 0.15.3, distribution DKMS package |

| Scenario | Observation |
| --- | --- |
| Continuous physical capture | Decoded 1280×720 near 30 FPS; image fluid for several minutes |
| Snapshot close/reopen | Working image after independent app reopening |
| Integrated ↔ Jieli switching | Repeated switching passed; integrated camera unaffected |
| Direct V4L2 repeated open | Three 120-frame sessions: 30.10, 30.00, 29.78 FPS |
| Simultaneous PipeWire consumer | 29.68 FPS during V4L2 validation |
| Physical USB reconnect | Automatic recovery; PipeWire 30.06, V4L2 30.02, decoded 30.00 FPS |
| Real Firefox WebRTC (reference baseline) | Two genuine getUserMedia sessions, 1280×720 negotiated at 30 FPS |
| Suspend/resume | Passed twice on the Rust stack, including with Snapshot active |
| Hibernate/resume | Passed twice on the Rust stack, including with Snapshot active |
| Clean reboot | Service auto-started, Jieli enumerated, capture `streaming` |
| Real Rust-helper reset | `1224:2a25` re-enumerated and bridge returned to `streaming` |
| Remote conferencing call | **Unverified** |
| Other models / multiple identical cameras | **Unverified / unsupported** |

The reference Firefox test was performed before the Rust cutover. A later
Firefox-specific check was blocked by a local renderer crash in the reference
session; Edge and Meet validated the Rust WebRTC/camera path, including camera
switching and close/reopen. This is an environmental limitation, not a claim
that Firefox works on every system.

## Automated checks: no camera or root actions

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The Rust tests cover the state machine, frame ownership, placeholder, report,
helper discovery/verification/lock logic, installer planning, manifest,
backups, conflict handling and modified-file protection. They do not connect
to D-Bus, reset USB, load modules or change services.

The plugin build is checked separately against the pinned source and installed
development headers:

```sh
tools/build-plugin.sh
GST_PLUGIN_PATH_1_0="$PWD/build/plugin" \
GST_REGISTRY_1_0="$PWD/build/plugin/inspect-registry.bin" \
  gst-inspect-1.0 pipewiresink
```

The builder verifies the upstream archive SHA-256, applies the patch, compiles
only the GStreamer plugin and fails on unresolved runtime symbols. It is not a
substitute for full threaded PipeWire integration testing.

## Opt-in checks against your running installation

These access live video in memory. They save no images. Close applications
that are using the raw physical camera; use the shared source instead.

### Shared PipeWire source

Inspect the node and verify that a separate client receives moving video:

```sh
pw-dump | grep -A20 -B5 'jieli_720p30'
gst-launch-1.0 pipewiresrc target-object=jieli_720p30 ! \
  videoconvert ! fakesink sync=false
```

The exact GStreamer target property may vary with your GStreamer/WirePlumber
version. A working desktop camera app is the authoritative functional check.

### Direct V4L2 source

When optional loopback is installed and the publisher reports it active:

```sh
v4l2-ctl --device=/dev/video10 --get-fmt-video
v4l2-ctl --device=/dev/video10 --stream-mmap=3 --stream-count=120 \
  --stream-poll --stream-to=/dev/null
```

Repeat the latter command three times. It opens the **virtual** device, not the
physical Jieli. Also confirm a real app shows moving, intact video; sink
statistics alone do not establish functional capture.

### Manual desktop checklist

1. Select **Jieli 720p30** in Snapshot and watch moving video for several minutes.
2. Switch to the integrated camera and back at least twice.
3. Close the app and reopen it independently.
4. Disconnect and reconnect the Jieli; verify automatic recovery.
5. When ready, suspend the system yourself, resume, then repeat steps 1–3.
6. If hibernation already works on your system, test it yourself and repeat.
7. At a convenient time, perform a clean reboot and report the separate result.

Never initiate sleep, hibernate or reboot just to satisfy a test on someone
else's machine.

## Repository packaging validation

The source tree contains the Rust runtime, Rust helper, Rust user installer,
the pinned C patch and the shell build wrapper. Generated binaries, local logs
and private diagnostic reports are excluded from the repository. The release
has not been root-installed over every supported distribution.
