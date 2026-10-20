# Rollback

Close camera clients first. Stopping capture may leave this camera requiring
a reconnect before another raw stream. Restarting WirePlumber can briefly
interrupt audio/video policy. Run user commands as your desktop user.

## 1. Stop and remove the user installation

```sh
systemctl --user disable --now jieli-camera-bridge.service
target/release/jieli-install --remove
systemctl --user daemon-reload
systemctl --user restart wireplumber.service
```

The installer verifies its manifest and refuses modified files. Removed files
are archived under `${XDG_STATE_HOME:-$HOME/.local/state}/jieli-camera/backups/`.
It does not remove the optional root helper, kernel module or system packages.

If the installation was made manually and has no manifest, inspect the paths
below and remove/archive only files you recognize as this bridge:

```text
~/.local/libexec/jieli-bridge
~/.local/libexec/jieli-publisher
~/.local/lib/jieli-gstreamer/libgstpipewire.so
${XDG_CONFIG_HOME:-~/.config}/systemd/user/jieli-camera-bridge.service
${XDG_CONFIG_HOME:-~/.config}/wireplumber/wireplumber.conf.d/90-jieli-bridge.conf
```

Removing the plugin override does not alter any distribution GStreamer library.

## 2. Remove the reset permission

Remove the sudoers rule first, then the helper. Do not edit a command while a
bridge reset is pending.

```sh
sudo rm -f /etc/sudoers.d/90-jieli-camera-reset
sudo rm -f /usr/local/libexec/jieli-usb-reset
sudo rm -f /usr/local/libexec/jieli-usb-reset.sha256
```

Removing the permission does not itself reset any USB device and does not stop
an already-running bridge.

## 3. Remove optional V4L2 configuration

Only if you installed the loopback configuration:

```sh
sudo rm -f /etc/modules-load.d/90-jieli-loopback.conf
sudo rm -f /etc/modprobe.d/90-jieli-loopback.conf
```

This does not unload a running module or remove distribution packages. After
all loopback producers and consumers have stopped, you may explicitly unload
it:

```sh
sudo modprobe -r v4l2loopback
```

If another app uses the module, leave it loaded. To remove the DKMS package,
review `apt remove v4l2loopback-dkms` yourself and accept only intended changes.

## 4. Optional local cache cleanup

After reviewing them, remove this project's cache directory
`${XDG_CACHE_HOME:-$HOME/.cache}/jieli-camera/` and repository `build/` output.
Backups and local status reports may contain account paths; keep them private.
No bootloader, GPU, swap or hibernation configuration needs reverting.
