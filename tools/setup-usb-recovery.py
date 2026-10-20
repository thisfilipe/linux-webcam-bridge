#!/usr/bin/python3 -I
"""Plan/install/remove one Jieli-only privileged Rust reset helper and sudo rule.

The default only prints a plan. Mutations require root, Python isolated mode
(-I), and an explicit --install or --remove. --user is always required and must
name an existing non-root local account. The no-argument helper can interrupt
both video and audio on exactly one USB 1224:2a25; it grants no general USB access.

Existing different files are never overwritten or removed. To upgrade a changed
helper or transfer permission to another account, stop the bridge, use the OLD
release and OLD --user to --remove, review the new release, then --install it.
Keep the old checkout until removal completes. Removing permission does not stop
an already-running helper or bridge. No services are started by this tool.
"""
import argparse
import hashlib
import os
from pathlib import Path
import pwd
import re
import stat
import subprocess
import sys
import tempfile

SOURCE = Path(__file__).resolve().parents[1] / "dist/jieli-usb-reset"
HELPER = Path("/usr/local/libexec/jieli-usb-reset")
HELPER_SHA256 = "8b82d909212be61ab7c148869012623688b4212d483c1dabee53d03c5499684b"
MANIFEST = Path("/usr/local/libexec/jieli-usb-reset.sha256")
RULE = Path("/etc/sudoers.d/90-jieli-camera-reset")


def validated_user(name):
    # Deliberately narrower than every Linux account name: no sudoers syntax.
    if not re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", name, flags=re.ASCII):
        raise SystemExit("--user must be a conservative local login name: [a-z_][a-z0-9_-]{0,31}")
    try:
        account = pwd.getpwnam(name)
    except KeyError:
        raise SystemExit(f"Unknown local account: {name}") from None
    if account.pw_uid == 0 or account.pw_name != name:
        raise SystemExit("--user must identify the exact existing non-root account")
    return name


def rule_text(user):
    return ("# Jieli bridge: root-owned VID/PID-checked helper, no arguments.\n"
            f'{user} ALL=(root) NOPASSWD: /usr/local/libexec/jieli-usb-reset ""\n')


def check_directory_tree(directory):
    for path in (directory, *directory.parents):
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue
        if not stat.S_ISDIR(info.st_mode):
            raise SystemExit(f"Refusing non-directory/symlink in installation path: {path}")
        if info.st_uid != 0 or info.st_mode & 0o022:
            raise SystemExit(f"Installation path is not exclusively controlled by root: {path}")


def ensure_directory(directory):
    # Explicit modes for every newly created ancestor, even with permissive umask.
    for path in reversed((directory, *directory.parents)):
        if not path.exists():
            path.mkdir(mode=0o755)
    check_directory_tree(directory)


def check_existing(path, expected, mode):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return
    if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
            or info.st_uid != 0 or info.st_gid != 0 or stat.S_IMODE(info.st_mode) != mode):
        raise SystemExit(f"Existing file is not a single root:root regular file with mode {mode:04o}: {path}")
    if path.read_bytes() != expected:
        raise SystemExit(f"Existing different file preserved: {path}; remove with its original release/user first")


def require_root_isolated():
    if os.geteuid() != 0 or not sys.flags.isolated:
        raise SystemExit("Run the selected mode with sudo /usr/bin/python3 -I tools/setup-usb-recovery.py --user LOGIN")


def manifest_text():
    return f"{HELPER_SHA256}  {HELPER}\n".encode("ascii")


def install(source, rule):
    for directory in (HELPER.parent, RULE.parent):
        ensure_directory(directory)
    with tempfile.TemporaryDirectory(prefix=".jieli-setup-", dir=RULE.parent) as temporary:
        staged_rule = Path(temporary) / RULE.name
        staged_rule.write_bytes(rule)
        staged_rule.chmod(0o440)
        os.chown(staged_rule, 0, 0)
        subprocess.run(["/usr/sbin/visudo", "-cf", str(staged_rule)], check=True)
        descriptor, name = tempfile.mkstemp(prefix=".jieli-reset-", dir=HELPER.parent)
        manifest_descriptor, manifest_name = tempfile.mkstemp(prefix=".jieli-reset-manifest-", dir=HELPER.parent)
        try:
            with os.fdopen(descriptor, "wb") as file:
                file.write(source)
                file.flush()
                os.fchmod(file.fileno(), 0o755)
                os.fchown(file.fileno(), 0, 0)
                os.fsync(file.fileno())
            with os.fdopen(manifest_descriptor, "wb") as file:
                file.write(manifest_text())
                file.flush()
                os.fchmod(file.fileno(), 0o644)
                os.fchown(file.fileno(), 0, 0)
                os.fsync(file.fileno())
            os.replace(name, HELPER)
            os.replace(staged_rule, RULE)
            os.replace(manifest_name, MANIFEST)
        finally:
            Path(name).unlink(missing_ok=True)
            Path(manifest_name).unlink(missing_ok=True)
    subprocess.run(["/usr/sbin/visudo", "-cf", str(RULE)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--user", required=True, help="existing non-root local login receiving only the exact reset permission")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--install", action="store_true")
    modes.add_argument("--remove", action="store_true")
    args = parser.parse_args()
    user = validated_user(args.user)
    print("Purpose: bounded USB recovery for exactly one Jieli 1224:2a25.")
    print(f"Helper: {HELPER} root:root 0755; SHA256={HELPER_SHA256}")
    print(f"Manifest: {MANIFEST} root:root 0644")
    print("Build the binary first with tools/build-helper.sh build.")
    print(f"Rule: {RULE} root:root 0440")
    print(f"Permission: {user} may run only the installed no-argument helper without a password.")
    print("Risk: reset briefly interrupts both video and microphone; no service or device is touched in this plan.")
    print("Rollback/update: stop bridge; use this release and same --user with --remove before installing changed files.")
    if not (args.install or args.remove):
        return
    require_root_isolated()
    source = SOURCE.read_bytes()
    if hashlib.sha256(source).hexdigest() != HELPER_SHA256:
        raise SystemExit("Staged helper hash changed; review required")
    rule = rule_text(user).encode("ascii")
    for directory in (HELPER.parent, RULE.parent):
        check_directory_tree(directory)
    check_existing(HELPER, source, 0o755)
    check_existing(MANIFEST, manifest_text(), 0o644)
    check_existing(RULE, rule, 0o440)
    if args.remove:
        RULE.unlink(missing_ok=True)
        HELPER.unlink(missing_ok=True)
        MANIFEST.unlink(missing_ok=True)
        print("Exact reset permission and Rust helper removed; running processes were not stopped.")
    else:
        install(source, rule)
        print("Installed the exact Rust reset helper, manifest and rule; validate bridge recovery separately.")


if __name__ == "__main__":
    main()
