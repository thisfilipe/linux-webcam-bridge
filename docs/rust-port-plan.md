# Rust port plan — Linux Webcam Bridge

**Status:** working draft; no runtime change has been made so far.
**Date:** 2026-10-19
**Target branch:** `rust-port` (`main` remains Python until gate G8)
**Scope:** port all project-owned code to Rust while preserving the behaviour
validated on hardware. System libraries and services (PipeWire, WirePlumber,
GStreamer, logind, sudo, v4l2loopback, the UVC driver) remain as they are.

> **Privacy:** this document uses `$REPO`, `$HOME`, `$XDG_RUNTIME_DIR` and
> `BBB/DDD` instead of paths, PIDs, USB addresses, serial numbers or hostnames.
> Hashes, versions and node names are kept because they do not identify the
> user. The public repository must not receive data specific to this
> installation.

---

## 0. How to use this document

1. This is the **context contract** for the port. Update it at the end of each
   phase.
2. **No gate is marked as passed without a reproducible command and result.**
   Record the date, command and output (or the log path) in the gate section.
3. `main` does not change until G8. Work happens on `rust-port` (or separate
   worktrees, one writer per checkout).
4. Do not install packages, stop services, reset USB, suspend or hibernate
   without explicit user authorisation.
5. The live deployment is the **behaviour reference**; the repository HEAD is the
   **new-code reference**. The divergences are listed in §1.4 and require a
   decision.
6. When documents conflict, evidence measured on this computer prevails over
   memory or older documentation.

**Legend:** `[F]` verified fact · `[I]` inference · `[?]` unverified.

---

## 1. Verified baseline

### 1.1 Repository

- 31 tracked files, **3,964 lines**.
- `src/` 3 files/931 lines; `tools/` 6/820; `tests/` 6/1,198; `config/` 2/46;
  `patches/` 1/72; `docs/` 6/643; root 4/183.
- Current runtime: two Python processes plus a Python root helper.
- `main` clean, one commit, remote `origin/main`.

### 1.2 Live deployment (measured)

| Item | Value |
|---|---|
| Unit | user unit, `active (running)`, `Result=success`, `NRestarts=0` |
| Rate | ~29.7–29.9 fps; thousands of frames; `recovery=streaming`, `error=null` |
| Node | `node.name=jieli_720p30`, `node.description=Jieli 720p30` |
| Properties | `media.class=Video/Source`, `media.role=Camera`, `node.virtual=TRUE`, `node.pause-on-idle=FALSE`, `priority.session=1000`, `node.driver=true`, `node.want-driver=true`, `stream.is-live=true`, `node.async=true` |
| Format | `I420 1280x720 @ 30/1` (single EnumFormat) |
| Port | `object.path=jieli_720p30:capture_0`, `port.alias=Jieli 720p30:capture_1`, meta `Busy`, IO `Buffers`/`AsyncBuffers` |
| Default camera | `default.video.source=jieli_720p30` |
| Physical camera | Jieli `1224:2a25`; **MJPG 1280x720@30**; YUYV 720p only **5 fps**, so MJPEG is mandatory |
| Loopback | `/dev/video10`, card `Jieli 720p30 V4L2`, YU12 1280x720 30 fps, 8 buffers |
| Module | `v4l2loopback 0.15.3`, loaded, `video_nr=10,-1,...`, single virtual device |
| Inhibitor | `sleep / "Jieli camera bridge" / "Close webcam before sleep" / delay`, active |
| Private plugin | `$HOME/.local/lib/jieli-gstreamer/libgstpipewire.so`, version 1.6.8, SHA-256 `77ff2682e43b5e08031facb153ddb8b91a819036c392284f468ac805957be14d`, `ldd -r` clean |
| Distribution plugin | `/usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstpipewire.so`, SHA-256 `b718a871b9bc33efa83081f9f92e670de401493e3e230b21c1a8ca0931a18642` |
| Root helper | `5673b774c9c54d8cc1d531f078ed9ad74d4ed430a5a74ab87497d5762e9be0d0`, identical to the repository file |
| Manifest | **absent** (`$HOME/.local/state/jieli-camera/installed-files.json`) |
| Versions | Pop!_OS 24.04, kernel 7.2.6, PipeWire 1.6.8, WirePlumber 0.5.12, GStreamer 1.24.2, Python 3.12.3 |
| Rust toolchain | rustc/cargo 1.99.0, host x86_64-unknown-linux-gnu |

Read-only reproduction commands:

```sh
systemctl --user status jieli-camera-bridge.service --no-pager
systemctl --user show jieli-camera-bridge.service \
  -p ActiveState -p SubState -p MainPID -p NRestarts -p Result
pw-dump | jq '[.[] | select(.info.props["node.name"]=="jieli_720p30")]'
pw-dump | jq '[.[] | select(.info.props["node.id"]==<id>)]'
v4l2-ctl --list-devices
v4l2-ctl -d /dev/video10 --all
cat "$XDG_RUNTIME_DIR/jieli-camera-bridge.json"
systemd-inhibit --list --no-pager
sha256sum "$HOME/.local/lib/jieli-gstreamer/libgstpipewire.so" \
  /usr/lib/x86_64-linux-gnu/gstreamer-1.0/libgstpipewire.so \
  /usr/local/libexec/jieli-usb-reset
```

### 1.3 Recorded validation (historical)

- **Suspend:** the original bridge **failed before the fix**
  (`publisher removed by watchdog`). After the fix, `capture_sessions 1→2`,
  `publisher_sessions` stayed at **1**, and the publisher and PipeWire identity
  were preserved (same `node_id`/`serial`).
- **Hibernate:** same pattern, `capture_sessions 3`, publisher preserved.
- **Hotplug:** automatic recovery, `capture_sessions 1→2`, publisher `8→9`,
  30.06 fps.
- **Consumers:** PipeWire `[29.82; 30.11; 30.51]`; V4L2
  `[30.10; 30.00; 29.78]`; Firefox `28.50` and `28.75` fps; simultaneous `29.68`.
- **Placeholder:** 1,382,400 B; Y=32, text=235, UV=128; 5×7 glyphs, scale 4,
  24 px/character, top 312/380.
- **Microbenchmark:** `Gst.Buffer.fill(bytearray)` = 36.648 ms/frame vs
  `bytes` = 0.15 ms.
- **Not tested:** `system_reboot`, `real_remote_video_call`.

### 1.4 Repository ↔ live divergences (decision required)

The deployment was installed before the public extraction; HEAD is not the
artefact validated on hardware.

| # | Item | Repository HEAD | Live deployment |
|---|---|---|---|
| 1 | Placeholder | `CAMERA UNAVAILABLE` / `RECONNECTING` (EN, `G` glyph) | `CAMERA INDISPONIVEL` / `RECONECTANDO` (PT) |
| 2 | video10 discovery | exact label | also accepts `Jieli` for `video10` |
| 3 | XDG paths | `CACHE_DIR`/`CONFIG_DIR` | `$HOME/.cache`, `$HOME/.config`, `/tmp` fallback |
| 4 | `--test-source` | renames the node to `jieli_validation` | does not rename |
| 5 | Unit | `%h`, “Linux webcam bridge” | hard-coded home, “camera bridge” |
| 6 | Manifest | required | absent |
| 7 | v4l2loopback configs | includes the `# Managed by…` marker | no marker |
| 8 | Plugin artefact | `build/plugin` `8a0a14fa…`; `build/rebuild` `efd81af8…` | installed `77ff2682…`, local build with `-g` and `DMA_DRM` |
| 9 | Python | 535 + 271 lines | 531 + 267 lines |
| 10 | Docs | kernel 7.1.5 | kernel 7.2.6 |

**Proposed rule:** parity against the **live** behaviour for everything the user
can observe; adopt the repository's XDG corrections only after revalidating them
on hardware. Divergences 1 and 2 require an explicit decision.

### 1.5 Automated baseline

```sh
python3 -m unittest discover -s tests -p 'test_*.py'
# 76 tests, OK
python3 tests/check_pool.py
# baseline: expected failure in "busy-then-free"
# patched: 17 deterministic scenarios pass
```

The C harness tests the real `acquire_buffer()` function from the original and
patched sources. The value is not the test count: it is the **“baseline must
fail”** property.

---

## 2. Contracts to preserve

### A. Supervisor lifecycle

| ID | Contract | Current value |
|---|---|---|
| A1 | One physical capture; generation invalidates stale frames/callbacks | generation increments on start/stop; check after the copy |
| A2 | Independent publisher; survives teardown/reopen and is restarted if it dies | `poll()` every 1 s; retry 1 s; spawn failure → 8 s |
| A3 | Serialised teardown; never reset/open over an unclosed pipeline | lock; `set_state(NULL)` plus verification; terminal `close_failed` |
| A4 | Reset budget | max 3; backoff 8/16/32 s; success clears `retries` |
| A5 | Reset barrier | only `rc=0` clears `reset_barrier`; `rc≠0`/timeout keeps capture closed |
| A6 | Device identity | `stat()` of the by-id node: `ino` + `rdev`; change → recovery; `blocked` released when identity changes |
| A7 | Sleep | `delay` lease acquired before opening; `PrepareForSleep(true)` closes; releases on close or at 90% of the limit (max 100 ms margin); `(false)` → reset/settle/reopen |
| A8 | No lease, no capture | `sleep is None or sleep.fd is not None` |
| A9 | Watchdog | 12 s without a frame → recovery; 0.5 s + 10 s + 90 frames → `streaming`, `retries=0` |
| A10 | State | 1 Hz JSON, atomic (`tmp`+rename), removed on exit; 22 keys |
| A11 | Signals | clean SIGTERM/SIGINT; SIGUSR1 on the publisher → 20 s of debug |
| A12 | Publisher | `GST_PLUGIN_PATH_1_0`/`GST_REGISTRY_1_0` only in the child; `terminate` → 3 s → `kill`; close stdin |

**22 state keys, in this order:**
`pid, node_name, capture_active, capture_closing, capture_starting,
publisher_active, capture_sessions, publisher_sessions, frames, fps_10s,
seconds_since_frame, sleeping, recovery, retries, retry_in_seconds,
reset_pending, reset_barrier, last_reset, sleep_inhibited, sleep_monitor_error,
placeholder, error`.

**`recovery` states:** `starting, opening, waiting-frames, backoff, sleeping,
resetting, settling, exhausted, helper-unavailable, streaming`.

### B. Capture

| ID | Contract | Value |
|---|---|---|
| B1 | Device | `/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0` |
| B2 | Negotiation | MJPG 1280x720@30 |
| B3 | Output | I420 1280x720 30/1, **1,382,400 B** |
| B4 | Queues | appsink `max-buffers=2 drop=true sync=false`; publication depth 1, drops the oldest |
| B5 | Ownership | immutable bytes; extraction before the lock; generation checked after the copy |
| B6 | Test source | `videotestsrc is-live=true pattern=ball` |
| B7 | Teardown | must reach `NULL`; otherwise terminal state |

### C. Publisher / PipeWire node

| ID | Contract | Value |
|---|---|---|
| C1 | Identity | `jieli_720p30` / `Jieli 720p30` (test source: `jieli_validation` / `Jieli validation`) |
| C2 | Properties | `Video/Source`, `Camera`, `node.virtual=TRUE`, `node.pause-on-idle=FALSE`, `priority.session=1000`, driver/want-driver, live |
| C3 | Format | I420 1280x720 30/1 |
| C4 | Placeholder | after 250 ms; Y=32, text=235, UV=128; never replays a real frame; 30 fps |
| C5 | Pipe | partial reads accumulate; EOF terminates |
| C6 | Retained buffer | never overwrite; closing/reopening a consumer does not recreate capture or publication |
| C7 | No stall | scan `n_buffers` slots on `EBUSY`; retry 1 ms (active) / 10 ms (empty/paused) for `EBUSY/EPIPE/EAGAIN` |
| C8 | V4L2 | an error never stops PipeWire; retry 30 s |
| C9 | PID/SIGUSR1 | `$XDG_RUNTIME_DIR/jieli-publisher-<node>.pid`; 20 s debug |
| C10 | Exit | 1 on pipeline error; 0 otherwise |

**I420 on the wire:** 3 data blocks — Y 921,600 B, U 230,400 B, V 230,400 B.
The GStreamer sink uses `blocks=n_planes`; the native publisher must fill the
three planes with the correct offsets and strides.

### D. V4L2 output

| ID | Contract | Value |
|---|---|---|
| D1 | Main node only | `node_name == jieli_720p30` |
| D2 | Discovery | label `Jieli 720p30 V4L2`, under `/sys/devices/virtual/video4linux`; flag `$XDG_CONFIG_HOME/jieli-camera/disable-v4l2` |
| D3 | I/O | mmap, YU12 1280x720 30 fps, `exclusive_caps=1` (rw does not work in GStreamer 1.24.2) |
| D4 | Device change | rebuild; never unload the module |
| D5 | Isolation | a V4L2 failure does not affect PipeWire |

### E. Root helper

| ID | Contract | Value |
|---|---|---|
| E1 | Interface | no arguments; euid 0 |
| E2 | Discovery | exactly one `1224:2a25` |
| E3 | Open | `/dev/bus/usb/BBB/DDD`, `O_RDWR|O_CLOEXEC|O_NOFOLLOW`; major 189; minor `(bus-1)*128+addr-1` |
| E4 | Identity | 18 bytes from the **same fd**; `\x12\x01`; VID/PID at offset 8 |
| E5 | Action | only `USBDEVFS_RESET` = 0x5514 |
| E6 | Lock | `/run/lock/jieli-usb-reset.lock`, root-owned, regular, `nlink=1`, no group/other write, `flock` non-blocking |
| E7 | Limits | 8 s cooldown; 8 s alarm |
| E8 | Re-enumeration | `ENODEV` → recheck for up to 3 s; success message |
| E9 | Sudoers | `USER ALL=(root) NOPASSWD: /usr/local/libexec/jieli-usb-reset ""` |
| E10 | Installation | artefact hash pinned; the integrity model is in §3.6 |

### F. Installers

| ID | Contract |
|---|---|
| F1 | `setup-user`: 5 destinations, JSON manifest, backups, refuses an active service/symlink, `--replace`, modes 0755/0644/0600 |
| F2 | `setup-usb-recovery`: root plus isolated mode, helper hash, conservative username regex, `visudo -cf`, helper before rule, rule removed first, root-only tree, single-link files |
| F3 | `setup-v4l2`: 2 files with a marker, exact parameters, refuses an occupied video10/foreign loopback, `modinfo`/`modprobe`, sysfs runtime verification, removes only the exact files, never `rmmod` |

### G. Configuration

| ID | Contract |
|---|---|
| G1 | WirePlumber disables the raw Jieli V4L2 node and the duplicate loopback |
| G2 | Unit: `Restart=on-failure`, `RestartSec=5`, `TimeoutStopSec=10`, `WantedBy=default.target` |

### H. Opt-in checks

| ID | Contract |
|---|---|
| H1 | Consumer: 3 rounds, ≥28 fps, CRC of the retained buffer, producer survives |
| H2 | Firefox: 2 sessions, 1280x720, ≥28 fps, isolated profile, metadata only on 127.0.0.1 |

---

## 3. Target architecture

### 3.1 Principle

“Total Rust” means all **project-owned code** in Rust. PipeWire, GStreamer,
WirePlumber, logind, sudo, v4l2loopback and the UVC driver remain system
libraries/services. The real decision is how much of the multimedia stack Rust
talks to directly.

### 3.2 Decisions and alternatives

| Decision | Options | Recommendation | Rationale | Risk |
|---|---|---|---|---|
| Capture | `gstreamer-rs` vs raw V4L2 + Rust decode | **`gstreamer-rs` first**; raw V4L2 only as an optional phase | The current pipeline already absorbs the device quirk; the bug is firmware/USB-level | Raw V4L2 reimplements colour, subsampling and incomplete-JPEG handling |
| Publication | `gstreamer-rs` + patch vs `pipewire-rs` | **`pipewire-rs`**, after Phase 0, with a watchdog | Removes the patch, `build-plugin.py`, the 1.6.8 gate and the private registry | I420 planes, pacing, binding soundness, #5378 wedge |
| Loopback | `v4l2sink` vs V4L2 output API | **`v4` directly** if the publisher is native | Removes GStreamer from the publisher; mmap YU12 is direct | `v4` has been stalled since 2023 |
| Processes | 2 vs 1 | **keep 2** | Crash isolation, independent restart, `publisher_sessions` contract | One 1.38 MB/frame copy (~41 MB/s) |
| Helper | raw ioctl vs `rusb`/`nusb` | **raw ioctl** (`rustix`/`libc`) | Preserves fd-based verification (major/minor/descriptor on the same fd) | None; it is the most isolatable component |
| State | GLib callbacks vs pure state machine | **pure state machine** | Deterministic tests; stronger than thread mocks | None |
| logind | `gio` vs `zbus` | **`gio::DBusProxy`** | Same stack as the current code; avoids zbus 6 plus zbus_systemd 5.x | Less “pure Rust” |

### 3.3 What disappears / remains

**Disappears:** `patches/`, `tools/build-plugin.py`, the 1.6.8 gate, the private
registry, `GST_PLUGIN_PATH_1_0`, the two runtime `.py` files, `check_pool.py`
(replaced by pump tests), PyGObject and the plugin override.

**Remains:** PipeWire 1.6.8, WirePlumber 0.5.12, GStreamer 1.24.2 (if capture
stays on `gstreamer-rs`), logind, sudo, v4l2loopback and the UVC driver.

### 3.4 Workspace and modules

```
Cargo.toml                       # workspace, edition 2024, rust-version 1.92+
rust-toolchain.toml              # pinned toolchain
Cargo.lock                       # committed
crates/
  jieli-core/                    # no system dependencies
    state.rs                     # pure state machine
    frame.rs                     # Frame, LatestFrame, generation
    placeholder.rs               # neutral I420 + bitmap font
    framing.rs                   # partial reads/EOF
    config.rs                    # XDG paths, CLI, defaults
    report.rs                    # state JSON + rounding
    pump.rs                      # EBUSY/EPIPE/EAGAIN policy
  jieli-pw-raw/                  # narrow seam: dequeue/queue + errno
  jieli-usb/                     # helper + verification
  jieli-capture/                 # GStreamer adapter (phase 1) / V4L2 (optional)
  jieli-publish/                 # native publisher (or gst in phase 1)
  jieli-bridge/                  # supervisor
  jieli-install/                 # setup-user, setup-usb-recovery, setup-v4l2
  jieli-check/                   # consumer-check, firefox-check
config/                          # unit + WirePlumber (unchanged)
tests/                           # integration + parity harness
```

**Crates and versions confirmed on 2026-10-19:**

| Crate | Version | Use |
|---|---|---|
| `pipewire` / `libspa` | 0.10.1 | native publisher; features `v0_3_50` |
| `gstreamer` / `gstreamer-app` | 0.25.4 / 0.25.2 | capture; Rust ≥1.92 |
| `v4` | 0.14.0 | V4L2 output (stalled; see risks) |
| `zbus` | 5.19 (if used) | alternative logind; **not** zbus 6 |
| `rustix` / `libc` | 1.1.5 / — | helper |
| `signal-hook` | 0.4.5 | signals |
| `serde` / `serde_json` | 1.x | state/manifest |
| `sha2`, `tempfile`, `clap`, `crossbeam-channel` | — | hashes, atomic writes, CLI, events |

### 3.5 Native publisher: loop and risks

```
[capture thread]  reads MJPEG → GStreamer/decoder → owned I420 frame (3 planes)
        │ (bounded channel, capacity 1–2, drops the oldest)
        ▼
[PipeWire main loop]  process callback (DRIVER):
    dequeue_raw_buffer()
    if NULL: read errno; EBUSY/EPIPE/EAGAIN → policy; otherwise → error
    if buffer: copy 3 planes; chunk.offset/size/stride; queue_raw_buffer
    trigger_process() according to pacing
```

**Flags:** `DRIVER | MAP_BUFFERS`. Do **not** add `ALLOC_BUFFERS` unless the
publisher implements and validates its own buffer pool. In the isolated P0b
spike, `ALLOC_BUFFERS` produced three data blocks with `maxsize=0`, `fd=-1` and
no usable memory; removing it let the consumer-allocated pool provide valid
MemFd buffers (`maxsize=1,382,400`, mapped `data`, `short=0`). This matches the
live `pipewiresink` path when the GStreamer shared-memory allocator is absent.
The live sink additionally uses `ASYNC | EARLY_PROCESS`; those flags are not
exposed by `pipewire-rs` 0.10.1 `StreamFlags` and can be added through a raw
seam only if latency measurements require them.

**`trigger_process`:** it exists, but is gated by
`#[cfg(feature = "v0_3_34")]`; use `features = ["v0_3_50"]`.

**`errno`:** the safe `dequeue_buffer() -> Option<Buffer>` API **discards**
`errno` (issue #45). For the fine-grained policy, use `dequeue_raw_buffer()` plus
`std::io::Error::last_os_error()` in a narrow module (`jieli-pw-raw`), with
`#[deny(unsafe_code)]` everywhere else and `#[allow(unsafe_code)]` only there.

**`node.reliable`:** do not enable it (the live node does not set it); `EAGAIN`
remains a defensive case.

**Mandatory watchdog:** PipeWire issue **#5378** shows that a `provide` node can
stop producing after an unclean consumer disconnect, including with non-GStreamer
producers. The native publisher must detect lack of progress and **recreate the
stream**; `disconnect`/`connect` is not enough.

**Binding soundness:** open issues #121 (UAF on listener drop), #103, #109 and
#19 (thread safety). Keep all objects/streams on one thread, use explicit drop
order, never drop a listener inside a callback, pin 0.10.1, and stress
create/destroy.

### 3.6 Helper: integrity and reproducibility

The current model installs **the Python source bytes** after checking
`HELPER_SHA256`; the root artefact is byte-identical to the reviewed source. A
Rust binary is not bit-reproducible by default. Options:

- **A.** Reproducible build plus a published SHA-256 (pinned toolchain,
  `--remap-path-prefix`, `SOURCE_DATE_EPOCH`, `Cargo.lock`). Changes the
  “no binaries in the repository” policy.
- **B.** Local build plus source hash, without a binary pin. Weakens the
  guarantee.
- **C.** Keep the helper in Python (125 lines, `-I`, source hash pinned) and port
  the rest.

**Recommendation:** A only if two independent builds are bit-identical;
otherwise **C**. Do not choose B silently. The sudoers rule and the path do not
change if the no-argument interface is preserved.

**Design:** `struct VerifiedDevice(OwnedFd)` constructed only by
`open_verified()`, and `reset(&VerifiedDevice)` receiving the already-validated
fd. `unsafe` confined to ioctl/signal; no `clap`/`serde`/`tokio`.

---

## 4. Parity and testing

### 4.1 Oracle

Before any Rust code, freeze the **live** behaviour:

- copies of the two installed `.py` files, unit, configs and hashes in
  `parity/live-<date>/` (on the `rust-port` branch, not `main`);
- `pw-dump` of the node and port;
- state JSON with every field;
- SHA-256 of the placeholder in both variants (PT/EN);
- framing matrix (partial reads, EOF);
- helper outputs and exit codes;
- installer plan and modes.

The harness runs against Python and then against Rust. No gate advances unless
the harness passes both.

### 4.2 Mapping of current tests

| Today | Rust |
|---|---|
| 16 `Lifecycle` | `jieli-core::state` — event sequences plus properties |
| 3 `SleepLease` | `jieli-core::sleep` — real fd, token, deadline |
| 3 `Placeholder` | `jieli-core::placeholder` plus `framing` |
| 12 helper | `jieli-usb` — pure functions plus integration |
| 4 `test_build` | disappear with the patch |
| 17 `check_pool` | `jieli-core::pump` — 17 equivalent scenarios |
| ~32 installer | `jieli-install` — tempdirs plus mocked runner |
| — | new: node properties, placeholder hash, JSON schema, decoder parity |

**Replacing the C harness:** a table-driven test over `publish_once` with at
least: `EBUSY→free`, all `EBUSY`, `EPIPE`, `EPIPE→free` (silent release),
`EAGAIN`, `EAGAIN→free`, non-transient error, flushing/stream error, paused,
zero buffers, `max(n,1)` scan limit, never overwrite a retained slot,
placeholder cadence, no replay, counters. Include a naive blocking
implementation (`#[cfg(test)]`) that **must fail**, preserving “baseline must
fail”.

### 4.3 Gates

| Gate | Pass criteria | Abort if |
|---|---|---|
| **G0** Baseline | harness passes on the live deployment; artefacts frozen | unexplained divergence |
| **G1** Helper | 12 tests plus a supervised real reset plus codes/messages | identity/permission difference |
| **G2** Installers | tempdir plus identical plan; zero real writes | unplanned sudo required |
| **G3** Core | 22 scenarios plus properties | reachable state diverges |
| **G4** Capture | 720p30, 3 reopens, teardown NULL, 12 s watchdog | frames <29 fps or failed teardown |
| **G5** Supervisor + Python publisher | suspend, hibernate, hotplug; publisher preserved | `publisher_sessions` >1 on suspend |
| **G6** Native publisher | properties via `pw-dump`; 17 scenarios; retained buffer; **unclean consumer plus stream recreation**; 3 reopens; Firefox ≥28 fps | stall, wrong plane, missing node |
| **G7** Full Rust stack | Snapshot 2 cycles, hotplug, suspend, hibernate | visual/functional regression |
| **G8** Cutover | same `node.name`, default camera, clean reboot | reboot not tested |
| **G9** Cleanup | remove Python/patch/build; final audit | unintended artefact remains |

### 4.4 Phase 0 details

**Goal:** prove the native publisher in isolation, outside the repository,
without root and without stopping the service.

**Prerequisites:**

- a `libclang` shared library available to `bindgen`; `pipewire-sys` always
  runs bindgen. A system install is one option; P0b instead extracted
  `libclang-18-dev`, `libclang-common-18-dev` and `libclang1-18` into a
  temporary prefix, with no root and no persistent system change. A system
  install remains optional and can use `pkexec`/`sudo` if preferred;
- the existing sysroot with PipeWire 1.6.8 and GStreamer 1.24.2 headers/.pc;
- `pipewire` 0.10.1 with `features = ["v0_3_50"]`;
- network access to crates.io (confirmed).

**Criteria (all):**

1. builds with libclang plus the sysroot;
2. `DRIVER | MAP_BUFFERS` plus `trigger_process()`; `ALLOC_BUFFERS` is tested
   separately and must not be enabled without a native pool;
3. errno visible through `dequeue_raw_buffer` plus `last_os_error()`; distinguishes
   `EBUSY/EPIPE/EAGAIN`;
4. node with 3 I420 planes (Y 921,600, U 230,400, V 230,400) and `pw-dump`
   matching the live node (ignoring id/serial/pid/media.name/library.name);
5. normal retention plus a consumer that dies holding a buffer;
6. recovery by stream recreation when there is no progress;
7. 1,000 create/destroy cycles without UAF/crash;
8. CPU/RSS within budget (≤ +10% CPU, ≥ −2 fps).

If it fails, fallback 2a: `gstreamer-rs` plus the C patch, keeping the
supervisor/helper/installers in Rust.

### 4.5 Phase 0b results (measured 2026-10-19)

**Setup.** The spike lived in an isolated temporary directory outside the
repository. It used the existing sysroot, `pipewire` 0.10.1 with
`features = ["v0_3_50"]`, and a locally extracted `libclang`; no system package
was installed, no root command was used and `jieli-camera-bridge.service`
stayed active. `LIBCLANG_PATH` pointed at the extracted library and
`BINDGEN_EXTRA_CLANG_ARGS` supplied the clang resource directory and the GCC
include directory. First debug build: ~9.5 s; release build: ~15.5 s; release
binary 718 KB.

**Results.**

| # | Criterion | Result | Evidence |
|---|---|---|---|
| 1 | Build with libclang + sysroot | **pass** | `pipewire-sys` bindgen plus `pipewire` 0.10.1 built against the sysroot; no system install |
| 2 | Flags + `trigger_process` | **pass with correction** | `DRIVER \| MAP_BUFFERS` works; `ALLOC_BUFFERS` removed (see below); the timer calls `trigger_process()` only while Streaming |
| 3 | errno via raw dequeue | **partial pass** | `dequeue_raw_buffer()` plus `last_os_error()` returned `errno=16` (`EBUSY`) 15 times while a consumer held buffers; `EPIPE`/`EAGAIN` are mapped but were not independently observed |
| 4 | 3 I420 planes + `pw-dump` parity | **pass** | 3 data blocks at runtime; Y 921,600, U/V 230,400, strides 1280/640; node properties match the live node except ids/serial, `media.name`, test name/description and `node.loop.name` |
| 5 | Retention + consumer dies holding a buffer | **pass** | `SIGSTOP` consumer → EBUSY; `SIGKILL` → Paused; a new consumer → Streaming and frames resumed |
| 6 | Recovery by stream recreation when no progress | **not proven** | The #5378 wedge did not reproduce; reconnect recovered without recreation (168 → 318 frames). Watchdog/recreation remains a P4a requirement |
| 7 | 1,000 create/destroy cycles | **pass** | exit 0, no panic/UAF; `cycles done 1000` |
| 8 | CPU/RSS/fps | **pass for publication** | release: ~0.4% CPU average (0.3% current), RSS ~39.5 MB, 30.0 fps (240 frames); consumer ~0.2%. The live Python publisher uses ~2.8% CPU and ~45 MB RSS, but includes camera capture |

**Buffer flag finding.** With `ALLOC_BUFFERS` the stream still reported
`n_datas=3`, but each block had `type=MemId(4)`, `maxsize=0`, `fd=-1` and
`data=(void*)-1`; every frame had three short blocks and no usable pixels.
Without `ALLOC_BUFFERS`, the consumer-allocated pool gave `type=MemFd(2)`,
`maxsize=1,382,400`, a valid `fd`, mapped `data` and `short=0`. The native
publisher must therefore follow the live `pipewiresink` behaviour (consumer
allocates) until it implements a native pool.

**Consumer harness finding.** In this environment `pipewiresrc` fails with
`target not found` for every target, including the live node, because
WirePlumber 0.5.12 `getDefaultNode` concatenates a nil `media.type`. Passing
`stream-properties="props,media.type=Video,media.category=Capture"` to the
consumer fixes the harness. This is a consumer/session-manager issue, not a
publisher defect, and must be used in future A/B tests.

**Parity notes.** The corrected producer properties match the live node,
including `node.virtual="TRUE"` and `node.pause-on-idle="FALSE"`. The remaining
differences are `media.name`, ids/serial, the test name/description and
`node.loop.name` (`main-loop` vs `pipewire-main-l`). `node.loop.name` is an
implementation detail; decide later whether to ignore it in the parity check or
set it explicitly.

**Caveats.** The spike fills synthetic frames with `write_bytes`, not decoded
camera MJPEG; the CPU comparison is publication-only. The `SPA_META_Header`
parameter was negotiated, but its fields were not populated in P0b. `EPIPE` and
`EAGAIN` were not independently reproduced. The #5378 wedge was not reproduced;
fallback 2a remains available if it appears during P4a.

**Live-service check.** After the spike, the service remained active/running,
`Result=success`, `NRestarts=0`; the state file reported `recovery=streaming`,
`error=null`, and frames advanced by ~119 in 4 s. No spike node or consumer
process remained.

**Decision.** Proceed with the native publisher design (3 binaries / 2 runtime
processes), with the buffer-flag correction and a P4a gate for watchdog/stream
recreation. Keep the `gstreamer-rs` capture path and the C-patch fallback 2a
until the native publisher passes G6 on real hardware.

---

## 5. Risks

| # | Risk | P/I | Mitigation / abort |
|---|---|---|---|
| 1 | Regression only visible on hardware | high/high | sequential A/B with Python; full matrix; abort cutover |
| 2 | `pipewire-rs` soundness (#121/#103/#109/#19) | high/high | single thread, drop order, pin, stress; if it fails, 2a |
| 3 | #5378 wedge even without GStreamer | high/high | watchdog plus recreation; unclean-consumer test |
| 4 | Rust helper not reproducible | high/high | prove two builds; otherwise keep Python (C) |
| 5 | libclang/bindgen/build broken | certain/high | install libclang; if #122 appears, pin the toolchain |
| 6 | Wrong I420 planes/meta | medium/high | spike plus `pw-dump` plus GStreamer consumer |
| 7 | Capture regression | medium/high | identical pipeline; test source first; A/B |
| 8 | Installer refuses the live deployment | high/medium | `--replace` plus legacy-config compatibility |
| 9 | `v4` stalled | medium/medium | small surface; raw-ioctl fallback |
| 10 | JSON/placeholder divergence | low/low | golden fixtures; tolerance only on floats |
| 11 | Two captures at once | medium/medium | Python service stopped during hardware tests |
| 12 | CPU/thermal over budget | medium/medium | measure; ≤ +10% CPU, ≥ −2 fps |

---

## 6. Phases and schedule

| Phase | Deliverable | Gate | Effort* |
|---|---|---|---|
| P0 | Oracle plus parity harness | G0 | 1–2 d |
| P0b | Isolated `pipewire-rs` spike | 4.4 (6/8 full; #3 partial; #6 at P4a) | 2–4 d |
| P1 | `jieli-core`: state, framing, placeholder, JSON | G3 | 4–6 d |
| P2 | Rust helper or decision C | G1 | 2–3 d |
| P3 | Supervisor plus `gstreamer-rs` capture, Python publisher | G4–G5 | 4–6 d |
| P4a | Native publisher, PipeWire only | G6 | 5–8 d |
| P4b | Native V4L2 output | 3×120 plus Firefox | 2–4 d |
| P5 | Full Rust stack plus cutover | G7–G8 | 4–6 d |
| P6 | Rust installers/tools, remove Python/patch | G9 | 3–5 d |

\* rough estimate, not a commitment.

**Branch:** `rust-port`; `main` untouched until G8. First commit:
`parity/live-<date>/` with artefacts and hashes. Small per-phase PRs. Cutover in
a single PR, tag `v2.0.0-rust`, keep tag `v1.x`.

**Cutover rollback:** dated tar of the five user files plus unit and plugin; stop
the service; restore; `daemon-reload`; start; validate 720p30 and the node. The
helper and sudoers rule do not change if the path/interface are preserved.

---

## 7. Open decisions

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| 1 | `libclang` for `pipewire-sys` | resolved in P0b by local extraction; system install optional | — |
| 2 | Helper: reproducible Rust (A) or Python (C) | A only if bit-identical; otherwise C | P2 |
| 3 | GStreamer: keep in capture or remove | keep `gstreamer-rs` | P1/P3 |
| 4 | Native publisher with watchdog or fallback 2a | proceed with native publisher; keep 2a; watchdog gate at P4a | P4a |
| 5 | Baseline: live (PT/fallback) or HEAD (EN/XDG) | live for visible behaviour; XDG after revalidation | G0 |
| 6 | Installers: now or in P6 | in P6 | P6 |
| 7 | Hardware windows (stop service, suspend/hibernate/unplug) | define who/when | G4–G8 |
| 8 | Publisher heartbeat/watchdog | after cutover | — |
| 9 | Extra JSON fields (`drops`, `no_slot`) | later, in a separate log | — |

---

## 8. Not verified

- Clean reboot (never tested by the current project).
- Real remote call.
- Native publisher watchdog/recreation after a #5378 wedge (planes, pacing and
  consumer allocation were verified in P0b; header fields were not populated).
- `EPIPE`/`EAGAIN` independently reproduced (only `EBUSY` was observed in P0b).
- Exact `gstreamer-rs` vs Python capture parity.
- Whether any app depends on `media.name=python3.12` or
  `library.name=videoconvert/libspa-videoconvert` (incidental).
- Suspend/hibernate on the current kernel with the Rust stack.
- Exact `inhibit` signature in `zbus_systemd` (if zbus is chosen).
- Whether the delay inhibitor is reusable after each sleep cycle.

---

## 9. References

**Repository:**
- `src/jieli-camera-bridge.py` (535 lines) — supervisor, sleep, reset, state.
- `src/jieli-pipewire-publisher.py` (271) — PipeWire, placeholder, V4L2.
- `src/jieli-usb-reset` (125) — root helper.
- `tools/build-plugin.py` (127) — pinned plugin build.
- `tools/setup-user.py`, `setup-usb-recovery.py`, `setup-v4l2.py`.
- `patches/gstpipewirepool-busy-slot.patch` (72).
- `tests/test_resume_lifecycle.py` (361), `test_installers.py` (383),
  `test_usb_recovery_helper.py` (117), `tests/check_pool.py` (181).
- `docs/diagnosis.md`, `installation.md`, `security.md`, `validation.md`,
  `troubleshooting.md`, `rollback.md`.

**Upstream (consulted 2026-10-19):**
- pipewire-rs 0.10.1: `Stream::connect`, `dequeue_raw_buffer`,
  `queue_raw_buffer`, `update_params`, `trigger_process` (feature `v0_3_34`).
- PipeWire 1.6.8 `src/pipewire/stream.c` — `EBUSY/EPIPE/EAGAIN`.
- PipeWire `src/examples/video-src-sync.c` — `DRIVER | ALLOC_BUFFERS | MAP_BUFFERS`;
  P0b showed this is not sufficient for planar, consumer-allocated buffers.
- P0b spike (2026-10-19) — `pipewire` 0.10.1, `DRIVER | MAP_BUFFERS`,
  consumer-allocated MemFd buffers, 3 I420 blocks, 1,000 create/destroy cycles.
- pipewire-rs issues: #45 (errno), #99 (update_properties), #103/#109/#121
  (listeners/UAF), #19 (thread safety), #122 (rustc/clang build).
- PipeWire issues: #5378 (provide-node wedge), #5412 (pipewiresink/Firefox),
  #5171, #5123.
- `xdg-desktop-portal/desktop-portal/camera.c` — `Video/Source` plus `Camera`
  filter.
- systemd `org.freedesktop.login1` — `Inhibit`, `PrepareForSleep`,
  `InhibitDelayMaxSec` default 5 s.
- `v4l2loopback` README — `exclusive_caps` for Chrome/WebRTC.
- `turbojpeg::decompress_to_yuv` — the only documented native YUV420 path.
- `v4` 0.14.0 (2023; last push 2024) — `Type::VideoOutput`/mmap.
- `cargo-deb` 3.8.0 — system units; a user unit is installed per user.

---

## 10. Decision log

| Date | Event | Result |
|---|---|---|
| 2026-10-19 | Repository baseline | 76 tests OK; 17 C scenarios OK; baseline fails as expected |
| 2026-10-19 | Live deployment measured | 720p30, `jieli_720p30` node, V4L2 `/dev/video10`, active inhibitor |
| 2026-10-19 | Repository↔live divergences | 10 items; baseline decision pending |
| 2026-10-19 | Upstream research | native publisher does not remove the #5378 class; errno needs a raw seam; `trigger_process` feature-gated |
| 2026-10-19 | This plan | created; no runtime change |
| 2026-10-19 | P0b build | `pipewire` 0.10.1, local `libclang` extraction, existing sysroot; no system install |
| 2026-10-19 | P0b buffer flags | `ALLOC_BUFFERS` yields empty blocks; consumer-allocated `DRIVER \| MAP_BUFFERS` works |
| 2026-10-19 | P0b errno | raw dequeue exposes `EBUSY`; `EPIPE`/`EAGAIN` mapped but not reproduced |
| 2026-10-19 | P0b stress | 1,000 create/destroy cycles clean; 30 fps; ~0.4% CPU in release |
| 2026-10-19 | P0b wedge | #5378 not reproduced; reconnect recovered; watchdog/recreation is a P4a gate |

**Next step:** confirm the baseline choice (§1.4/§7) and start P1
(`jieli-core`: pure state, framing, placeholder, JSON), with the remaining P0b
items (EPIPE/EAGAIN reproduction, meta header population, watchdog/recreation)
folded into P4a. No runtime change until G8.
