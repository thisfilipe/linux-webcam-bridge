# Installation

## Scope and prerequisites

Reference environment: Pop!_OS based on Ubuntu 24.04, GNOME Wayland,
PipeWire **1.6.8**, WirePlumber **0.5.12**, GStreamer **1.24.2** and Rust
**1.92+**. The build helper pins Rust `1.99.0` for its reproducible build; use
`rustup` if your distribution package is older.

The camera must expose MJPEG 1280×720 at 30 FPS at:

```text
/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0
```

This release supports exactly one `1224:2a25` camera. The USB helper refuses
zero or multiple matching devices. Other UVC cameras and PipeWire versions
need separate review; the plugin builder deliberately checks its version.
WirePlumber 0.4 uses a different configuration format and is not supported.
Systems without systemd/logind are not supported.

Check the device and versions before installing:

```sh
lsusb -d 1224:2a25
v4l2-ctl --device=/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0 --list-formats-ext
pw-cli --version
wireplumber --version
gst-inspect-1.0 --version
rustc --version
```

Format listing does not start capture. Avoid repeatedly opening the physical
camera to reproduce the known fault once you have established it.

### Dependencies on Debian / Ubuntu / Pop!_OS

Review your package manager's proposed changes. Package versions must match
the installed multimedia stack; do not add an unrelated repository or upgrade
the operating system just to satisfy this build.

```sh
sudo apt install --no-install-recommends \
  git build-essential pkg-config patch curl ca-certificates \
  gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-pipewire libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev libpipewire-0.3-dev libspa-0.2-dev \
  libglib2.0-dev v4l-utils
```

The project does not replace the PipeWire daemon, system GStreamer libraries,
UVC driver, boot arguments, GPU configuration or hibernation configuration.

## 1. Obtain and review the source

```sh
git clone https://github.com/thisfilipe/linux-webcam-bridge.git
cd linux-webcam-bridge
```

Read [security.md](security.md) before executing privileged steps. No
one-line download-and-run installer is provided.

## 2. Build the isolated plugin and Rust binaries

```sh
tools/build-plugin.sh

cargo build --release -p jieli-bridge -p jieli-publisher -p jieli-install \
  --features jieli-bridge/native,jieli-publisher/native

tools/build-helper.sh build
```

`tools/build-plugin.sh` downloads the official PipeWire 1.6.8 archive, checks
its SHA-256, extracts a fresh source tree, applies `patches/gstpipewirepool-busy-slot.patch`,
compiles only the GStreamer plugin and runs `ldd -r`. No install or service
change occurs. An existing archive can be supplied with
`--archive /path/to/1.6.8.tar.gz`; use `--build-dir build/rebuild` for another
build. `--source-only` prepares the pinned source without compiling the plugin.

`tools/build-helper.sh` requires the pinned Rust toolchain `1.99.0`; it builds
the helper twice, verifies byte-identical output, and writes
`dist/jieli-usb-reset` plus its SHA-256 manifest. Do not install a helper from a
missing or changed artifact.

For an optional load check without connecting to a camera:

```sh
GST_PLUGIN_PATH_1_0="$PWD/build/plugin" \
GST_REGISTRY_1_0="$PWD/build/plugin/inspect-registry.bin" \
  gst-inspect-1.0 pipewiresink
```

Confirm the reported plugin filename points to the newly built plugin and
version 1.6.8. Do not set these variables globally in your login environment.

## 3. Install the user files

Stop the service before replacing files; this ends physical capture and may
require a reconnect on the affected device. Run the installer from the
repository root. The default plan changes nothing:

```sh
systemctl --user stop jieli-camera-bridge.service

target/release/jieli-install
target/release/jieli-install --install
```

The installer writes the Rust supervisor and publisher, the Rust unit, the
WirePlumber camera-filter rule and the locally built plugin
(`build/plugin/libgstpipewire.so`). It refuses to run while the unit is active,
refuses symlinks and different existing files unless `--replace` is explicitly
reviewed, archives replaced files under
`${XDG_STATE_HOME:-$HOME/.local/state}/jieli-camera/backups/`, and records a
manifest at `${XDG_STATE_HOME:-$HOME/.local/state}/jieli-camera/installed-files.json`.

Use a custom plugin path with `--plugin /path/to/libgstpipewire.so`. Removal
uses the same manifest and refuses modified files:

```sh
systemctl --user stop jieli-camera-bridge.service
target/release/jieli-install --remove
```

The installer never starts or stops the service. The installed unit uses `%h`,
so it does not depend on your checkout directory or account name. Runtime
status uses `$XDG_RUNTIME_DIR`; caches and flags follow `XDG_CACHE_HOME` /
`XDG_CONFIG_HOME`.

The publisher prefers `~/.local/lib/jieli-gstreamer/libgstpipewire.so`; this is
the patched plugin that releases stale buffer slots after an unclean consumer.
Do not remove it while the Rust service is installed.

## 4. Install the privileged USB helper

This is needed for unattended recovery after suspend/hibernate or a capture
failure. Without it, the initial stream may work, but later recovery can
remain on a placeholder until you physically reconnect the camera.

The helper path and no-argument interface are fixed. Review the commands and
the [precise permission](security.md) before running them as root:

```sh
sudo install -Dm755 dist/jieli-usb-reset /usr/local/libexec/jieli-usb-reset
printf '%s  /usr/local/libexec/jieli-usb-reset\n' \
  "$(sha256sum dist/jieli-usb-reset | cut -d' ' -f1)" |
  sudo tee /usr/local/libexec/jieli-usb-reset.sha256 >/dev/null

rule=$(mktemp)
printf '%s ALL=(root) NOPASSWD: /usr/local/libexec/jieli-usb-reset ""\n' \
  "$(id -un)" >"$rule"
sudo visudo -cf "$rule"
sudo install -o root -g root -m 0440 "$rule" \
  /etc/sudoers.d/90-jieli-camera-reset
rm -f "$rule"
```

Validate the installed permission and helper:

```sh
sudo -n /usr/local/libexec/jieli-usb-reset
sha256sum -c /usr/local/libexec/jieli-usb-reset.sha256
```

The helper only resets a positively identified Jieli `1224:2a25`, uses a
root-owned lock and enforces an eight-second cooldown. It also briefly
interrupts the camera's composite microphone.

## 5. Optional direct V4L2 support

Skip this if all your applications use the shared PipeWire camera. Direct V4L2
clients need the distribution `v4l2loopback` kernel module. DKMS requires
headers for your current kernel and may require module signing on Secure Boot
systems. The module must rebuild after kernel updates.

```sh
sudo apt install --no-install-recommends "linux-headers-$(uname -r)" v4l2loopback-dkms
```

Configure one loopback instance at `/dev/video10`, with the same label and
format used by the publisher:

```sh
printf '%s\n' 'v4l2loopback' |
  sudo tee /etc/modules-load.d/90-jieli-loopback.conf >/dev/null
printf '%s\n' \
  'options v4l2loopback video_nr=10 card_label="Jieli 720p30 V4L2" exclusive_caps=1' |
  sudo tee /etc/modprobe.d/90-jieli-loopback.conf >/dev/null
```

If `v4l2loopback` is not already loaded, load it explicitly after reviewing
the configuration:

```sh
sudo modprobe v4l2loopback
```

If an unrelated loopback instance is already loaded, stop and inspect its
users before unloading it. Never unload another application's active loopback.
The publisher uses **mmap** output; GStreamer 1.24.2's V4L2 `rw` write path is
not implemented.

## 6. Start the bridge

Close camera applications first. Restarting WirePlumber can briefly interrupt
audio/video policy in the current session.

```sh
systemctl --user daemon-reload
systemctl --user restart wireplumber.service
systemctl --user enable --now jieli-camera-bridge.service
systemctl --user status jieli-camera-bridge.service
journalctl --user -u jieli-camera-bridge.service -n 40 --no-pager
```

Select **Jieli 720p30** in PipeWire camera clients, or **Jieli 720p30 V4L2**
in direct V4L2 applications. The integrated camera remains available. Avoid
opening the raw physical Jieli device concurrently with the bridge.

Run the [validation checklist](validation.md), including suspend and hibernate
only when you are ready to initiate them yourself.

## Updates

The local plugin override is private to the publisher, but it still depends on
installed PipeWire/GStreamer libraries. Recheck compatibility after their
updates. A different PipeWire release needs a reviewed patch port; the current
builder intentionally refuses it. Do not repeatedly restart physical capture
to troubleshoot publisher-only failures.

See [rollback.md](rollback.md) for removal.
