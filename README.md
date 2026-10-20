# Linux Webcam Bridge

Keep a troublesome USB webcam usable across app restarts, camera switching,
USB reconnection, suspend and hibernate by sharing one persistent capture.

**Validated hardware: Jieli Technology USB PHY 2.0 (`1224:2a25`), 1280×720 at
30 FPS.** The approach may help other cameras with similar stop/start failures,
but this release includes device-specific capture settings, WirePlumber rules
and a Jieli-only reset helper. Other devices are **unverified and require a
reviewed port**. The project is a userspace workaround; the kernel UVC driver
and camera firmware remain unchanged.

## The problem

On the reference camera, a first MJPEG stream after USB reset works. Stopping
and reopening it leaves applications with black video. An intact USB capture
showed that the second session delivered no complete JPEG frames even though
UVC negotiation succeeded. Resetting the device restored the first stream.

A [2020 Linux media report for the same USB ID](https://www.spinics.net/lists/linux-media/msg180041.html)
describes a similar failure requiring reconnection. Similar symptoms alone do
not establish that another camera has the same underlying fault.

## What this project does

```mermaid
flowchart LR
    USB["Physical webcam: one MJPEG capture"] --> Decode["Decode to owned I420 frames"]
    Decode --> Publisher["Persistent publisher"]
    Publisher --> PW["PipeWire: Jieli 720p30"]
    Publisher --> V4L2["Optional V4L2 loopback"]
    PW --> Apps["Camera apps and portal clients"]
    V4L2 --> Direct["Direct V4L2 clients and browsers"]
    Sleep["logind sleep/resume"] --> Recovery["Close capture, bounded USB reset, reopen"]
    Recovery --> USB
```

- Keeps physical capture running while clients come and go.
- Publishes a decoded shared PipeWire camera; optionally feeds `/dev/video10`.
- Includes an isolated PipeWire 1.6.8 GStreamer buffer-pool patch that prevents
  a separate publisher stall when consumers retain or release buffers.
- Keeps the publisher present through disconnect and system sleep, using a
  neutral **Camera unavailable / Reconnecting** frame after 250 ms without
  input. It does not replay the last camera image.
- Coordinates capture teardown with logind's bounded delay inhibitor and
  performs limited recovery after resume.
- Leaves the integrated camera available. Hides only the raw Jieli and the
  optional duplicate loopback from WirePlumber's camera list.

**Privacy and tradeoffs:** the physical camera remains active, and its LED may
stay on, whenever the user service is running and the camera is connected.
Video is decoded in memory and offered to local camera clients. The bridge
does not save frames or transmit them over the network. Continuous decoding
costs CPU/power; raw I420 is about 41 MB/s at 720p30 before copies. USB recovery
briefly interrupts the webcam's composite microphone too.

## Rust cutover status

`main` now contains the Rust supervisor, publisher and privileged reset helper.
The reference installation was cut over from Python and validated with capture,
PipeWire/V4L2 consumers, Snapshot, Edge/Meet WebRTC, suspend, hibernate, a real
USB reset and a clean reboot. The Python programs, the previous unit and the
previous helper are retained as rollback; they are not started by the installed
Rust unit. See [Rust cutover](docs/rust-cutover.md) for the installed layout,
validation scope and rollback.

The portable user-file and V4L2 installers are still the legacy Python tools;
the Rust helper now has a reproducible artifact and manifest, and the runtime
does not depend on Python. P6 packaging cleanup remains tracked in the
[Rust port plan](docs/rust-port-plan.md).

## Start here

1. Check the [supported environment and installation steps](docs/installation.md).
2. Review [privileges and security](docs/security.md), especially before
   installing the optional root reset helper or kernel loopback module.
3. Use the [validation guide](docs/validation.md) to check your own device.

For failures, see [troubleshooting](docs/troubleshooting.md) and
[rollback](docs/rollback.md).

The reference system used Pop!_OS / Ubuntu 24.04, GNOME Wayland, PipeWire 1.6.8,
WirePlumber 0.5.12, GStreamer 1.24.2 and Python 3.12. The plugin builder currently
requires **PipeWire 1.6.8**; it refuses other versions. This is not a universal
installer for every Linux distribution.

| Real hardware check | Reference result |
| --- | --- |
| Continuous 720p30 image | Passed for several minutes |
| Snapshot 51 Flatpak: reopen and switch cameras | Passed |
| Direct V4L2: three reopen cycles | 30.10 / 30.00 / 29.78 FPS |
| Firefox: two real WebRTC capture sessions (baseline) | 1280×720, negotiated 30 FPS; presented 28.50 / 28.75 FPS headless |
| Physical unplug/replug | Automatic recovery passed |
| Suspend/resume | Passed twice on Rust, including with Snapshot active |
| Hibernate/resume | Passed twice on Rust, including with Snapshot active |
| Clean reboot | Passed on Rust; service auto-started and capture returned |
| Real Rust-helper reset | Passed through the bridge's `/usr/bin/sudo -n` path |
| Firefox-specific check on Rust | Blocked by a local Firefox renderer crash; Edge/Meet validated WebRTC |
| Remote video call / other webcam models | **Not tested** |

These hardware results were collected from the working deployment from which
this repository was extracted. The portable packaging is checked separately;
it has not been reinstalled over that deployment. See [full validation scope](docs/validation.md).

## Repository layout

| Path | Purpose |
| --- | --- |
| `crates/` | Rust supervisor, core state machine, publisher and privileged reset helper |
| `src/` | Legacy Python baseline retained for rollback/oracle |
| `patches/` | Patch against upstream PipeWire 1.6.8 |
| `tools/` | Reproducible builds, explicit installers, opt-in consumer checks |
| `config/` | Rust/Python user units, WirePlumber rules and sleep hook |
| `tests/` | Mocked lifecycle, permission and installer checks; deterministic C regression |
| `docs/` | Installation, diagnosis, security, validation, rollback |

There are no compiled modules, plugins, USB traces, camera images, private
diagnostic logs or user-specific installation paths in the repository.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Reports for other models are welcome:
include USB ID, software versions, exact reproduction and measured results.
Please remove usernames, paths, serial numbers and tokens from logs first.

## License

New project code is [MIT licensed](LICENSE). The PipeWire patch derives from
MIT-licensed upstream code; its notices are preserved in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Dependencies remain under
their own licenses.
