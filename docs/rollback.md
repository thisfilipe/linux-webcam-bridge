# Rollback

Close camera clients first. Stopping capture may leave this camera requiring
a reconnect before another raw stream. Restarting WirePlumber can briefly
interrupt audio/video policy. Run user commands as your desktop user.

## Rust installation

The Rust runtime is installed manually as `jieli-bridge`, `jieli-publisher`,
the Rust user unit, the WirePlumber rule and the private GStreamer plugin.
Archive any files you customized, then stop and disable the service before
reviewing removals:

```sh
systemctl --user disable --now jieli-camera-bridge.service
target/release/jieli-install --remove
```

The Rust installer verifies its manifest and refuses modified files. If no
manifest exists (for example, an older manual installation), remove only the
files you recognize as this installation. Keep
`~/.local/lib/jieli-gstreamer/libgstpipewire.so` if another local build uses
it, and keep the Python files if you still want the rollback path.

## Legacy Python installation

The following removal procedure applies to the retained Python baseline, not
to the Rust runtime.

## 1. Stop the bridge and remove its user files

```sh
systemctl --user disable --now jieli-camera-bridge.service
python3 tools/setup-user.py --remove
systemctl --user daemon-reload
systemctl --user restart wireplumber.service
```

Removal uses the installation manifest, refuses modified files and archives
removed files under `${XDG_STATE_HOME:-$HOME/.local/state}/jieli-camera/backups/`.
It removes only the two programs, plugin, user unit and WirePlumber rule that
this installer recorded. Modified files require your review; they are not
silently deleted. It does not automatically restore an older customization
from a backup; inspect and restore the desired copy yourself.

For a legacy installation without this repository's manifest, inspect the
following paths and remove/archive only files you recognize as this bridge:

```text
~/.local/libexec/jieli-camera-bridge.py
~/.local/libexec/jieli-pipewire-publisher.py
~/.local/lib/jieli-gstreamer/libgstpipewire.so
${XDG_CONFIG_HOME:-~/.config}/systemd/user/jieli-camera-bridge.service
${XDG_CONFIG_HOME:-~/.config}/wireplumber/wireplumber.conf.d/90-jieli-bridge.conf
```

Then reload systemd and restart WirePlumber as above. Removing the plugin
override does not alter any distribution GStreamer library.

## 2. Remove the reset permission

```sh
sudo /usr/bin/python3 -I tools/setup-usb-recovery.py --user "$(id -un)" --remove
```

This removes only the exact generated sudoers rule and matching helper. The
permission is removed first. If the installed files differ, the script stops
for review. Removing the permission does not itself reset any USB device.
Keep the release used for installation: upgrading a changed helper requires
removing it with that original release and the same `--user`, then reviewing
and installing the new version.

## 3. Remove optional V4L2 configuration

Only if you installed the loopback portion:

```sh
sudo /usr/bin/python3 -I tools/setup-v4l2.py --remove
```

This removes this project's two exact loading/parameter files. It does not
unload a running module or remove distribution packages. After all loopback
producers and consumers have stopped, you may explicitly unload it:

```sh
sudo modprobe -r v4l2loopback
```

If another app uses the module, leave it loaded. To remove the DKMS package,
review `apt remove v4l2loopback-dkms` yourself and accept only intended changes.
The module will otherwise stay installed for other uses.

## 4. Rust cutover rollback

If the installed unit runs `~/.local/libexec/jieli-bridge`, restore the Python
unit saved by the cutover instead of removing the installation:

```sh
systemctl --user stop jieli-camera-bridge.service
cp -a "$HOME/.local/state/jieli-camera/cutover-<stamp>/jieli-camera-bridge.service" \
  "$HOME/.config/systemd/user/jieli-camera-bridge.service"
systemctl --user daemon-reload
systemctl --user start jieli-camera-bridge.service
```

The Python programs were retained in `~/.local/libexec/` during the cutover.
See [rust-cutover.md](rust-cutover.md) for the full procedure and validation
scope. If no cutover backup exists, restore `config/jieli-camera-bridge.service`
from the Python baseline.

## 5. Optional local cache cleanup

After reviewing them, remove this project's cache directory
`${XDG_CACHE_HOME:-$HOME/.cache}/jieli-camera/` and repository `build/` output.
Backups and local status reports may contain account paths; keep them private.
No bootloader, GPU, swap or hibernation configuration needs reverting.
