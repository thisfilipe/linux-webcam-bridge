# Installation

## Scope and prerequisites

Reference environment: Pop!_OS based on Ubuntu 24.04, GNOME Wayland,
PipeWire **1.6.8**, WirePlumber **0.5.12**, GStreamer **1.24.2**, Python **3.12**.
The scripts derive user paths from the current account and XDG directories.
Build and user setup must run **without sudo**, from your desktop session.

The camera must expose MJPEG 1280×720 at 30 FPS at:

```text
/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0
```

This release supports exactly one `1224:2a25` camera. The USB helper refuses
zero or multiple matching devices. Other UVC cameras and PipeWire versions
need separate review; the plugin builder deliberately checks its version.
WirePlumber 0.4 uses a different configuration format and is not supported by
the supplied rules. Systems without systemd/logind are not supported.

Check the device and versions before installing:

```sh
lsusb -d 1224:2a25
v4l2-ctl --device=/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0 --list-formats-ext
pw-cli --version
wireplumber --version
gst-inspect-1.0 --version
python3 --version
```

Format listing does not start capture. Avoid repeatedly opening the physical
camera to reproduce the known fault once you have established it.

### Dependencies on Debian / Ubuntu / Pop!_OS

Review your package manager's proposed changes. Package versions must match
the installed multimedia stack; do not add an unrelated repository or upgrade
the operating system just to satisfy this build.

```sh
sudo apt install --no-install-recommends \
  git build-essential pkg-config patch \
  python3-gi gir1.2-gstreamer-1.0 gir1.2-gst-plugins-base-1.0 \
  gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-pipewire libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev libpipewire-0.3-dev libspa-0.2-dev \
  libglib2.0-dev v4l-utils
```

Rust `1.99.0` (with `cargo`) is required for the runtime and the reproducible
helper; install it with `rustup` if the distribution package is older. The
native build also needs the PipeWire/GStreamer development packages above.

The project does not replace the PipeWire daemon, system GStreamer libraries,
UVC driver, boot arguments, GPU configuration or hibernation configuration.

## 1. Obtain and review the source

```sh
git clone https://github.com/thisfilipe/linux-webcam-bridge.git
cd linux-webcam-bridge
```

Read [security.md](security.md) and the installer scripts before executing
privileged steps. No one-line download-and-run installer is provided.

## 2. Build the isolated plugin and Rust binaries

```sh
python3 tools/build-plugin.py
python3 tests/check_pool.py
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --release -p jieli-bridge -p jieli-publisher \
  --features jieli-bridge/native,jieli-publisher/native
tools/build-helper.sh build
```

`tools/build-helper.sh` requires the pinned Rust toolchain `1.99.0`; it builds
the helper twice, verifies byte-identical output, and writes
`dist/jieli-usb-reset` plus its SHA-256 manifest. The USB installer refuses a
missing or changed artifact. The native publisher also needs the local
`~/.local/lib/jieli-gstreamer/libgstpipewire.so` override built below; it
falls back to the distribution plugin without it.

The builder downloads the official PipeWire 1.6.8 archive, checks SHA-256,
extracts into `build/plugin/`, applies the patch, compiles only the GStreamer
plugin and checks unresolved symbols. No install or service change occurs.

An existing archive can be supplied with `--archive /path/to/1.6.8.tar.gz`.
Every build requires a fresh source directory; use `--build-dir build/rebuild`
for another build and pass that plugin path to user setup with `--plugin`.
`--source-only` prepares the pinned source and patch for C regression tests
without development headers.

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
require a reconnect on the affected device. Review and archive any existing
destinations first. The installed unit uses `%h`, so it does not depend on your
checkout directory or account name. Runtime status uses `$XDG_RUNTIME_DIR`;
caches and flags follow `XDG_CACHE_HOME` / `XDG_CONFIG_HOME`.

```sh
systemctl --user stop jieli-camera-bridge.service

install -Dm755 target/release/jieli-bridge ~/.local/libexec/jieli-bridge
install -Dm755 target/release/jieli-publisher ~/.local/libexec/jieli-publisher
install -Dm644 config/jieli-camera-bridge.rust.service \
  ~/.config/systemd/user/jieli-camera-bridge.service
install -Dm644 config/90-jieli-bridge.conf \
  ~/.config/wireplumber/wireplumber.conf.d/90-jieli-bridge.conf
install -Dm755 build/plugin/libgstpipewire.so \
  ~/.local/lib/jieli-gstreamer/libgstpipewire.so
```

This installs the Rust supervisor and publisher, the Rust unit, the
WirePlumber camera-filter rule and the locally built PipeWire plugin. It does
not start the service. The legacy `tools/setup-user.py` installs the retained
Python rollback files, not the Rust runtime; do not run it for a new Rust
installation. If replacing an existing Rust installation, keep a private copy
of any files you customized.

The publisher prefers `~/.local/lib/jieli-gstreamer/libgstpipewire.so`; this is
the patched plugin that releases stale buffer slots after an unclean consumer.
Do not remove it while the Rust service is installed.

## 4. Enable automatic USB recovery

This is needed for unattended recovery after suspend/hibernate or a capture
failure. Without it, the initial stream may work, but later recovery can
remain on a placeholder until you physically reconnect the camera.

```sh
python3 tools/setup-usb-recovery.py --user "$(id -un)"
sudo /usr/bin/python3 -I tools/setup-usb-recovery.py --user "$(id -un)" --install
```

The first command is a preview. Build `dist/jieli-usb-reset` first with
`tools/build-helper.sh build`; the installer verifies its pinned SHA-256
manifest. The second command installs the root-owned Rust helper, its manifest
and a sudoers entry for the explicitly selected non-root user. It validates the
rule with `visudo`. It does not reset the camera during setup. The helper only
resets a positively identified Jieli `1224:2a25` and also briefly interrupts its
microphone. Review [the precise permission](security.md).

## 5. Optional direct V4L2 support

Skip this if all your applications use the shared PipeWire camera. Direct
V4L2 clients need an additional kernel module, `v4l2loopback`, from your
distribution. DKMS requires headers for your current kernel and may require
module signing on Secure Boot systems. The module must rebuild after kernel
updates. The reference deployment used v4l2loopback 0.15.3.

```sh
sudo apt install --no-install-recommends "linux-headers-$(uname -r)" v4l2loopback-dkms
python3 tools/setup-v4l2.py
sudo /usr/bin/python3 -I tools/setup-v4l2.py --install
```

Setup loads the installed module and writes two explicit configuration files
for loading it at boot. It reserves `/dev/video10` with label
`Jieli 720p30 V4L2`, 1280×720 and `exclusive_caps=1`. Occupied device numbers,
unrelated existing module instances and conflicting config are refused.
No module binary is bundled. Installing the DKMS package itself can invoke
distribution hooks that load a default module instance. If setup refuses an
already-loaded instance without this project's exact config, inspect its
devices and users. Only after confirming it is unused, explicitly unload it
with `sudo modprobe -r v4l2loopback`, then run setup again. Never unload another
application's active loopback to satisfy this installer.

The publisher uses **mmap** output. GStreamer 1.24.2's V4L2 `rw` write path is
not implemented; apparently successful sink statistics are insufficient to
prove frames reached the loopback.

For the current session, ensure your desktop user has read/write access to
the virtual device. Do not grant world access to USB or video device nodes.

## 6. Start the bridge

Close camera applications first. Restarting WirePlumber can briefly interrupt
audio/video policy in the current session. If the camera was already left
stuck by an earlier app, unplug and reconnect it once before starting.

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

Run the [validation checklist](validation.md), including suspend and
hibernate only when you are ready to initiate them yourself.

## Updates

The local plugin override is private to the publisher, but it still depends
on installed PipeWire/GStreamer libraries. Recheck compatibility after their
updates. A different PipeWire release needs a reviewed patch port; the current
builder intentionally refuses it. Do not repeatedly restart physical capture
to troubleshoot publisher-only failures. A reboot is not part of installation
validation performed for this release.

See [rollback.md](rollback.md) for full removal.
