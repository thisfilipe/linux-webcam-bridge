#!/usr/bin/python3
"""Install user files, or remove a previous installation. Default: print plan.

Installation copies files only. Start/stop services explicitly as documented.
Existing different files are refused unless --replace is supplied; replaced
and removed files are archived in the user's state directory.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
HOME = Path.home()
CONFIG = Path(os.environ.get("XDG_CONFIG_HOME", HOME / ".config"))
STATE = Path(os.environ.get("XDG_STATE_HOME", HOME / ".local/state")) / "jieli-camera"
MANIFEST = STATE / "installed-files.json"
PLUGIN = HOME / ".local/lib/jieli-gstreamer/libgstpipewire.so"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def files(plugin):
    return {
        HOME / ".local/libexec/jieli-camera-bridge.py": ROOT / "src/jieli-camera-bridge.py",
        HOME / ".local/libexec/jieli-pipewire-publisher.py": ROOT / "src/jieli-pipewire-publisher.py",
        CONFIG / "systemd/user/jieli-camera-bridge.service": ROOT / "config/jieli-camera-bridge.service",
        CONFIG / "wireplumber/wireplumber.conf.d/90-jieli-bridge.conf": ROOT / "config/90-jieli-bridge.conf",
        PLUGIN: plugin,
    }


def atomic_write(path, data, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=".jieli-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            os.fchmod(output.fileno(), mode)
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def backup_directory():
    result = STATE / "backups" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    result.mkdir(parents=True)
    return result


def archive(path, directory):
    relative = path.relative_to(HOME) if path.is_relative_to(HOME) else Path("external") / path.as_posix().lstrip("/")
    target = directory / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(path, target)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--install", action="store_true")
    mode.add_argument("--remove", action="store_true")
    parser.add_argument("--replace", action="store_true", help="Archive and replace differing existing files")
    parser.add_argument("--plugin", type=Path, default=ROOT / "build/plugin/libgstpipewire.so")
    args = parser.parse_args()
    if os.geteuid() == 0:
        parser.error("Run as your regular desktop user, without sudo")
    destinations = files(args.plugin.resolve())
    print("User installation (no service changes):")
    for target in destinations:
        print(" ", target)
    print("The bridge continuously captures video while connected and running.")
    print("Review docs/installation.md before starting it; USB recovery is a separate root setup.")
    if not (args.install or args.remove):
        return
    # Avoid replacing code underneath a running bridge and stopping physical
    # capture by surprise. The administrator explicitly controls service changes.
    active = subprocess.run(["systemctl", "--user", "is-active", "--quiet", "jieli-camera-bridge.service"])
    if active.returncode == 0:
        parser.error("Stop jieli-camera-bridge.service before changing its files")
    if active.returncode not in (3, 4):
        parser.error("Cannot establish service state; run from your desktop user session")
    if MANIFEST.is_symlink():
        parser.error("Refusing a symlink installation manifest")
    if args.remove:
        if not MANIFEST.exists():
            parser.error("No installation manifest found; follow manual rollback documentation")
        installed = json.loads(MANIFEST.read_text())
        if set(installed) != {str(path) for path in destinations}:
            parser.error("Manifest paths do not match this user's installation layout")
        for target in destinations:
            if target.is_symlink() or (target.exists() and digest(target.read_bytes()) != installed[str(target)]):
                parser.error(f"Modified file preserved: {target}")
        backup = backup_directory()
        for target in destinations:
            if target.exists():
                archive(target, backup)
                target.unlink()
        archive(MANIFEST, backup)
        MANIFEST.unlink()
        print("Removed; archived at", backup)
        return
    data = {target: source.read_bytes() for target, source in destinations.items()}
    for target, content in data.items():
        if target.is_symlink():
            parser.error(f"Refusing existing symlink: {target}")
        if target.exists() and target.read_bytes() != content and not args.replace:
            parser.error(f"Existing different file preserved: {target}; review before using --replace")
    # The build is local and trusted; link-check before copying its plugin.
    linkage = subprocess.run(["ldd", "-r", str(args.plugin.resolve())], text=True, capture_output=True, check=True)
    if "undefined symbol:" in linkage.stdout + linkage.stderr or "not found" in linkage.stdout + linkage.stderr:
        parser.error("Plugin has unresolved runtime dependencies; rebuild against installed libraries")
    backup = backup_directory()
    for target, content in data.items():
        if target.exists():
            archive(target, backup)
        atomic_write(target, content, 0o755 if target == PLUGIN else 0o644)
    atomic_write(MANIFEST, (json.dumps({str(path): digest(content) for path, content in data.items()}, indent=2) + "\n").encode(), 0o600)
    print("Installed. Previous files archived at", backup)
    print("Next: follow the explicit service and USB recovery steps in docs/installation.md")


if __name__ == "__main__":
    main()
