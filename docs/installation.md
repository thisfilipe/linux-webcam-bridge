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

The project does not replace the PipeWire daemon, system GStreamer libraries,
UVC driver, boot arguments, GPU configuration or hibernation configuration.

## 1. Obtain and review the source

```sh
git clone https://github.com/thisfilipe/linux-webcam-bridge.git
cd linux-webcam-bridge
```

Read [security.md](security.md) and the installer scripts before executing
privileged steps. No one-line download-and-run installer is provided.

## 2. Build the isolated plugin

```sh
python3 tools/build-plugin.py
python3 tests/check_pool.py
python3 -m unittest discover -s tests -p 'test_*.py'
```

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

First preview the exact destinations:

```sh
python3 tools/setup-user.py
python3 tools/setup-user.py --install
```

This copies the two Python programs, user service, WirePlumber rules and
built plugin. It does **not** start or stop services. It writes an installation
manifest and archives previous copies in
`${XDG_STATE_HOME:-$HOME/.local/state}/jieli-camera/backups/`.
Differing existing files require explicit `--replace` after review. A running
bridge must be stopped before replacement; this ends physical capture and
may require a reconnect on the affected device.

The service uses `%h`, so it does not depend on your checkout directory or
account name. Runtime status uses `$XDG_RUNTIME_DIR`; caches and flags follow
`XDG_CACHE_HOME` / `XDG_CONFIG_HOME`.

## 4. Enable automatic USB recovery

This is needed for unattended recovery after suspend/hibernate or a capture
failure. Without it, the initial stream may work, but later recovery can
remain on a placeholder until you physically reconnect the camera.

```sh
python3 tools/setup-usb-recovery.py --user "$(id -un)"
sudo /usr/bin/python3 -I tools/setup-usb-recovery.py --user "$(id -un)" --install
```

The first command is a preview. The second installs a root-owned no-argument
helper and a sudoers entry for the explicitly selected non-root user. It
validates the rule with `visudo`. It does not reset the camera during setup.
The helper only resets a positively identified Jieli `1224:2a25` and also
briefly interrupts its microphone. Review [the precise permission](security.md).

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
