# Privileges, privacy and trust

## User service

Capture and publishing run as the desktop user, not root. Images remain in
memory and are exposed to local camera clients via PipeWire/V4L2. The bridge
has no image upload, network listener or recording feature. Camera apps can
still record or transmit video according to their own behavior and permissions.

Continuous physical capture means the LED can stay on and consumes CPU/power
even with no consumers. Stopping the service ends capture. On the affected
camera, a later physical stream may require recovery. There is no claim that
the service provides a camera privacy switch.

The plugin is installed under `~/.local/lib/jieli-gstreamer/` and selected
only in the publisher's child environment. System GStreamer and PipeWire
binaries are untouched. A locally built shared library is executable code;
build only source you trust. The source archive checksum pins what is built
but does not replace source review or authenticate a maintainer by itself.

## Restricted reset permission

Automatic recovery requires exactly two privileged files:

| File | Owner / mode | Purpose |
| --- | --- | --- |
| `/usr/local/libexec/jieli-usb-reset` | root:root 0755 | Isolated Rust reset helper (reference installation) |
| `/etc/sudoers.d/90-jieli-camera-reset` | root:root 0440 | Authorizes one selected user |

The generated sudoers command specification is:

```sudoers
SELECTED_USER ALL=(root) NOPASSWD: /usr/local/libexec/jieli-usb-reset ""
```

`SELECTED_USER` is replaced with the explicitly supplied validated account.
The trailing `""` restricts this command to **no arguments**. The helper also
rejects arguments itself. The installed helper is the reproducible Rust binary;
the bridge invokes the fixed absolute helper through `/usr/bin/sudo -n` and
never supplies a USB path or ID.

The root helper:

- Requires exactly one sysfs device matching `1224:2a25`.
- Opens the current USB node without following a final symlink.
- Verifies it is USB character major 189 with the expected bus/address minor.
- Reads and verifies the USB device descriptor **from that same open FD**
  before issuing `USBDEVFS_RESET` on it.
- Uses a root-owned regular single-link lock file, refuses group/other write
  access, serializes calls and enforces an eight-second cooldown.
- Handles reset-induced re-enumeration by rechecking identity.

It cannot reset arbitrary devices or run arbitrary root commands. A permitted
user **can intentionally reset this camera** within the rate limit. That also
interrupts its composite USB microphone. The helper's eight-second alarm is
best effort: a kernel-blocked ioctl may outlast it. The bridge's timeout and
reset barrier do not authorize reopening over a failed or pending reset.

The root installation steps are intentionally explicit: build the helper with
`tools/build-helper.sh build`, verify its SHA-256 manifest, install it as
root:root 0755, stage the sudoers rule and validate it with `visudo -cf` before
installing it. On rollback, remove the rule first and then the helper. Do not
replace this permission with broad passwordless sudo, a user-writable root
service, or world-writable USB nodes.

## Optional kernel module

`v4l2loopback` is optional. It increases kernel code in use and depends on
distribution DKMS/module-signing support. Installation is a separate explicit
step. The documented configuration reserves `/dev/video10`; review an existing
module instance before changing or unloading it, and never unload another
application's active loopback.

## Reporting

For a security issue, contact the maintainer through a private channel listed
on the GitHub profile before publishing exploit details. If no private channel
is available, open an issue asking for one without including sensitive details.
Never attach credentials, full USB captures or camera images to a public issue.
