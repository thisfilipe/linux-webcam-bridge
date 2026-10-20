# Rust cutover

The `rust-port` branch contains the native replacement for the Python bridge.
The cutover was performed after the capture, reset and publication gates
passed on the reference hardware. `main` remains Python until the change is
merged.

## Installed layout

```text
~/.local/libexec/jieli-bridge          # Rust supervisor
~/.local/libexec/jieli-publisher       # Rust GStreamer publisher + native V4L2 writer
~/.local/libexec/jieli-camera-bridge.py         # retained Python rollback copy
~/.local/libexec/jieli-pipewire-publisher.py    # retained Python rollback copy
~/.config/systemd/user/jieli-camera-bridge.service
```

The unit in `config/jieli-camera-bridge.rust.service` is the installed unit. It
runs `%h/.local/libexec/jieli-bridge`; the supervisor starts the publisher from
the same directory and keeps the live node name `jieli_720p30`, the description
`Jieli 720p30` and the default state file
`$XDG_RUNTIME_DIR/jieli-camera-bridge.json`.

The root USB helper path and interface are unchanged. The reference cutover
kept the existing installed helper; replacing that root-owned file with
`target/release/jieli-usb-reset` requires an explicit privileged step.

## Build and install

```sh
cargo build --release -p jieli-bridge --features native \
  -p jieli-publisher --features native

backup="$HOME/.local/state/jieli-camera/cutover-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$backup"
cp -a "$HOME/.local/libexec/jieli-camera-bridge.py" "$backup/"
cp -a "$HOME/.local/libexec/jieli-pipewire-publisher.py" "$backup/"
cp -a "$HOME/.config/systemd/user/jieli-camera-bridge.service" "$backup/"

systemctl --user stop jieli-camera-bridge.service
install -m 0755 target/release/jieli-bridge "$HOME/.local/libexec/jieli-bridge"
install -m 0755 target/release/jieli-publisher "$HOME/.local/libexec/jieli-publisher"
install -m 0644 config/jieli-camera-bridge.rust.service \
  "$HOME/.config/systemd/user/jieli-camera-bridge.service"
systemctl --user daemon-reload
systemctl --user start jieli-camera-bridge.service
```

Set `state` expectations accordingly: the first real capture after a restart
may need the existing 12 s watchdog reset before frames flow.

## Checks performed at cutover

- systemd restart, real capture and reset recovery.
- Three direct V4L2 sessions of 120 frames each at about 29.9 fps.
- PipeWire consumer cadence of 30.0 fps without retained-buffer backpressure;
  the retained-buffer ownership check passed.
- `NRestarts=0`, state `recovery=streaming`, publisher and capture sessions
  stable, and no test node or process left behind.

## Publisher regression fix (2026-10-20)

The initial native `pipewire-rs` publisher passed the synthetic gates but
regressed Snapshot after minimising/unfocusing the window: the consumer stopped
draining buffers and playback fell to a few frames until the camera was
reselected. The native publisher was not shipped as the final publication path.

The installed publisher now uses `gstreamer-rs` with the same contract as the
validated Python publisher: live `appsrc` (`do-timestamp=true`), leaky queue and
`pipewiresink mode=provide use-bufferpool=false sync=false async=false`. When
`~/.local/lib/jieli-gstreamer/libgstpipewire.so` exists, the publisher prepends
that directory to `GST_PLUGIN_PATH` before starting threads/GStreamer; this is
the patched plugin that releases stale busy buffer slots after an unclean
consumer. Without the local plugin, GStreamer falls back to the system plugin.

Post-fix checks on 2026-10-20:

- consumer idle for 10 s with the pipeline still PLAYING: 30.1 fps before,
  31.0 fps after resume;
- three PipeWire consumer rounds: 30.11, 30.09 and 31.01 fps, with the retained
  buffer unchanged;
- three direct V4L2 sessions of 120 frames: 29.91 fps each;
- user validation in Snapshot: minimise/restore, switch camera and return,
  close/reopen all preserve continuous playback;
- service `recovery=streaming`, about 29.9 fps, `NRestarts=0`.

## Suspend/resume recovery

A real suspend/resume exposed an xHCI controller failure: the kernel logged
`xHC error in resume`, `HC died; cleaning up` and `failed to resume async:
-110`; the Jieli device disappeared from the bus and only a reboot restored the
controller. No userspace process can revive a dead xHCI controller.

Two mitigations are prepared:

- the bridge classifies the helper's `found 0` result as a missing device and
  keeps retrying with the placeholder instead of entering the terminal
  `exhausted` state; it also logs `prepare`/`resume` and capture teardown;
- `config/jieli-usb-resume` is a systemd-sleep `post` hook that rebinds only the
  affected xHCI controller (`0000:06:00.4`) when the camera is still absent
  after resume and waits for re-enumeration. It must be installed as root:
  `sudo install -m 0755 config/jieli-usb-resume /usr/lib/systemd/system-sleep/50-jieli-usb-resume`.
  This systemd version reads only `/usr/lib/systemd/system-sleep/`; `/etc` is not a hook path here.

Do not remove the hook on the reference machine until suspend/resume has been
retested with it. The next controlled test must be done by the user; reboot is
the fallback if the controller still does not return.

The headless Firefox check is not usable in this reference session: Firefox
crashes in its software renderer before returning metadata. Test Firefox
manually, with the normal desktop session. The clean reboot gate passed after
the camera returned; suspend/hibernate remain under test.

## Rollback

Stop the Rust service, restore the backed-up Python unit, and start it again.
The Python programs were not removed.

```sh
systemctl --user stop jieli-camera-bridge.service
cp -a "$backup/jieli-camera-bridge.service" \
  "$HOME/.config/systemd/user/jieli-camera-bridge.service"
systemctl --user daemon-reload
systemctl --user start jieli-camera-bridge.service
systemctl --user status jieli-camera-bridge.service --no-pager
```

If no backup directory is available, restore `config/jieli-camera-bridge.service`
from the Python baseline and confirm the two Python programs still exist under
`~/.local/libexec/`. The Python publisher remains a separate child of the
Python supervisor; no separate unit is required.

After the Rust version has passed the user-facing checks, the Python files and
the legacy `src/` implementation can be removed in the P6 cleanup. Keep the
cutover backup until that cleanup is reviewed. Do not remove the local patched
PipeWire plugin (`~/.local/lib/jieli-gstreamer/libgstpipewire.so`): the
GStreamer publisher prefers it for safe buffer recycling after an unclean
consumer; P6 must install or retain it and document the fallback to the system
plugin.
