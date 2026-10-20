# Rust cutover

The `rust-port` branch contains the native replacement for the Python bridge.
The cutover was performed after the capture, reset and publication gates
passed on the reference hardware. `main` remains Python until the change is
merged.

## Installed layout

```text
~/.local/libexec/jieli-bridge          # Rust supervisor
~/.local/libexec/jieli-publisher       # Rust PipeWire + V4L2 publisher
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

The headless Firefox check is not usable in this reference session: Firefox
crashes in its software renderer before returning metadata. Test Firefox
manually, with the normal desktop session. Suspend, hibernate and reboot were
not initiated by the cutover.

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
cutover backup until that cleanup is reviewed.
