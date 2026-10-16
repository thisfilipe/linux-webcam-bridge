#!/usr/bin/python3 -I
"""Plan/install/remove optional V4L2 loopback configuration on Debian/Ubuntu.

Default is a read-only plan. Install kernel headers and v4l2loopback-dkms yourself
using your distribution's package manager first. This tool downloads/installs no
packages or prebuilt modules. --install explicitly loads the already-installed
v4l2loopback for the current kernel and writes two exact root-owned config files.
A kernel module increases kernel attack surface; DKMS/Secure Boot compatibility
and real capture/reopen validation remain the administrator's responsibility.

Refuse occupied video10, unrelated loaded loopbacks, other loopback configuration,
symlinks, and modified files. No module is ever unloaded. --remove deletes only
our exact two config files; a loaded device remains until an administrator stops
its users and unloads it manually or reboots. To change this tool's configuration,
remove it using the old release first. No bridge service is changed by this tool.
"""
import argparse
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile

LABEL = "Jieli 720p30 V4L2"
MODULE = Path("/sys/module/v4l2loopback")
NODE = Path("/dev/video10")
CLASS = Path("/sys/class/video4linux/video10")
VIRTUAL = Path("/sys/devices/virtual/video4linux")
OPTIONS = ["devices=1", "video_nr=10", f'card_label="{LABEL}"', "exclusive_caps=1",
           "max_buffers=8", "max_width=1280", "max_height=720"]
MARKER = "# Managed by linux-webcam-bridge setup-v4l2.py.\n"
CONFIG = {
    Path("/etc/modprobe.d/90-jieli-loopback.conf"):
        MARKER + 'options v4l2loopback devices=1 video_nr=10 card_label="Jieli 720p30 V4L2" exclusive_caps=1 max_buffers=8 max_width=1280 max_height=720\n',
    Path("/etc/modules-load.d/90-jieli-loopback.conf"): MARKER + "v4l2loopback\n",
}
CONFIG_DIRS = tuple(Path(prefix) / suffix
                    for prefix in ("/etc", "/run", "/usr/local/lib", "/usr/lib", "/lib")
                    for suffix in ("modprobe.d", "modules-load.d"))


def check_directory_tree(directory):
    for path in (directory, *directory.parents):
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(info.st_mode):
            raise SystemExit(f"Refusing non-directory/symlink in privileged path: {path}")
        if info.st_uid != 0 or info.st_mode & 0o022:
            raise SystemExit(f"Installation path is not exclusively controlled by root: {path}")


def check_existing(path, text):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return
    if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
            or info.st_uid != 0 or info.st_gid != 0 or stat.S_IMODE(info.st_mode) != 0o644):
        raise SystemExit(f"Existing config is not a single root:root regular file mode 0644: {path}")
    if path.read_text() != text:
        raise SystemExit(f"Existing different config preserved: {path}; remove with its original release first")


def check_configs():
    for path, text in CONFIG.items():
        check_directory_tree(path.parent)
        check_existing(path, text)


def check_other_configs():
    for directory in CONFIG_DIRS:
        for path in directory.glob("*.conf"):
            if path in CONFIG:
                continue
            try:
                lines = path.read_text().replace("\\\n", "").splitlines()
            except (OSError, UnicodeError) as error:
                raise SystemExit(f"Cannot audit existing module config {path}: {error}") from None
            if any(re.search(r"(?<![\w-])v4l2loopback(?![\w-])", line.partition("#")[0]) for line in lines):
                raise SystemExit(f"Other v4l2loopback configuration preserved: {path}")


def verify_runtime():
    """Conservative idempotency: exactly the named virtual node and parameters."""
    try:
        info = NODE.stat()
        device_number = (CLASS / "dev").read_text().strip()
        if not re.fullmatch(r"[0-9]+:[0-9]+", device_number, flags=re.ASCII):
            raise ValueError("invalid sysfs V4L2 device number")
        major, minor = map(int, device_number.split(":"))
        # video_nr controls the basename; the allocated kernel minor may differ.
        if (not stat.S_ISCHR(info.st_mode) or major != 81
                or info.st_rdev != os.makedev(major, minor)):
            raise ValueError("video10 does not match the sysfs V4L2 character device")
        if CLASS.resolve(strict=True) != VIRTUAL / "video10":
            raise ValueError("video10 is not the expected virtual device")
        if (CLASS / "name").read_text().strip() != LABEL:
            raise ValueError("video10 has a different label")
        if sorted(path.name for path in VIRTUAL.glob("video*")) != ["video10"]:
            raise ValueError("other virtual video devices are present")
        def values(name):
            return (MODULE / "parameters" / name).read_text().strip().split(",")
        numbers = [int(value) for value in values("video_nr")]
        exclusive = values("exclusive_caps")
        if not numbers or numbers[0] != 10 or any(value != -1 for value in numbers[1:]):
            raise ValueError("different video_nr parameters")
        if not exclusive or exclusive[0] not in ("Y", "1") or any(value not in ("N", "0") for value in exclusive[1:]):
            raise ValueError("different exclusive_caps parameters")
        for name, expected in (("max_buffers", 8), ("max_width", 1280), ("max_height", 720)):
            if int(values(name)[0]) != expected:
                raise ValueError(f"different {name} parameter")
    except (OSError, ValueError, RuntimeError, OverflowError) as error:
        raise SystemExit(f"Loaded module/device is not the exact managed loopback: {error}") from None


def preflight():
    check_configs()
    check_other_configs()
    if MODULE.exists():
        if not all(path.exists() for path in CONFIG):
            raise SystemExit("v4l2loopback already loaded without our exact configs; leave it untouched and review manually")
        verify_runtime()
        return True
    if NODE.exists() or NODE.is_symlink() or CLASS.exists() or CLASS.is_symlink():
        raise SystemExit("/dev/video10 is occupied; no device will be replaced")
    return False


def atomic_config(path, text):
    for directory in reversed((path.parent, *path.parent.parents)):
        if not directory.exists():
            directory.mkdir(mode=0o755)
    check_directory_tree(path.parent)
    descriptor, name = tempfile.mkstemp(prefix=".jieli-config-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as file:
            file.write(text.encode("ascii"))
            file.flush()
            os.fchmod(file.fileno(), 0o644)
            os.fchown(file.fileno(), 0, 0)
            os.fsync(file.fileno())
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--install", action="store_true")
    mode.add_argument("--remove", action="store_true")
    args = parser.parse_args()
    print("Optional output: /dev/video10, Jieli 720p30 V4L2, I420 1280x720 at 30 fps.")
    print("Prerequisite: distribution kernel headers and v4l2loopback-dkms installed for the running kernel.")
    print("Install: load the installed v4l2loopback module and write these exact root:root 0644 configs:")
    for path in CONFIG:
        print(" ", path)
    print("Risk: extra kernel module; DKMS rebuilds and Secure Boot signing may require administrator work.")
    print("Remove: only these exact configs; no package removal, module unload, or service operation.")
    if not (args.install or args.remove):
        return
    if os.geteuid() != 0 or not sys.flags.isolated:
        raise SystemExit("Run the selected mode with sudo /usr/bin/python3 -I tools/setup-v4l2.py")
    if args.remove:
        check_configs()
        for path in CONFIG:
            path.unlink(missing_ok=True)
        print("Exact configs removed; any loaded loopback remains active until manual unload or reboot.")
        return
    loaded = preflight()
    if not loaded:
        try:
            result = subprocess.run(["/usr/sbin/modinfo", "-F", "name", "v4l2loopback"],
                                    capture_output=True, text=True, check=True)
        except (OSError, subprocess.CalledProcessError) as error:
            raise SystemExit(f"No usable installed v4l2loopback for this kernel; install distribution DKMS/headers first: {error}") from None
        if result.stdout.strip() != "v4l2loopback":
            raise SystemExit("modinfo did not identify the installed v4l2loopback module")
        subprocess.run(["/usr/sbin/modprobe", "--ignore-install", "v4l2loopback", *OPTIONS], check=True)
        verify_runtime()
    for path, text in CONFIG.items():
        atomic_config(path, text)
    print("Configured. Validate real capture and reopening separately; no bridge service was started.")


if __name__ == "__main__":
    main()
