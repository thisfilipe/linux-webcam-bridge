#!/usr/bin/python3
"""Build the isolated patched GStreamer plugin, never install system libraries.

Requires PipeWire development files exactly 1.6.8 and GStreamer >= 1.24.
Only the pinned upstream archive is accepted, including with --archive.
"""
import argparse
import ctypes
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
VERSION = "1.6.8"
URL = f"https://github.com/PipeWire/pipewire/archive/refs/tags/{VERSION}.tar.gz"
SHA256 = "8181172a1d95131f6af8bbc0b98f90b2a33349b042b84c3ce57dd5d11348cc58"
DEPENDENCIES = ["gstreamer-1.0", "gstreamer-base-1.0", "gstreamer-video-1.0",
                "gstreamer-audio-1.0", "gstreamer-net-1.0",
                "gstreamer-allocators-1.0", "libpipewire-0.3", "gio-unix-2.0"]
NAMES = ["gstpipewire", "gstpipewirecore", "gstpipewireclock", "gstpipewireformat",
         "gstpipewirepool", "gstpipewiresink", "gstpipewiresrc",
         "gstpipewirestream", "gstpipewiredeviceprovider"]


def pkg(*arguments):
    return subprocess.check_output(["pkg-config", *arguments], text=True).strip()


def extract_archive(archive, destination):
    """Reject links, special files and paths outside the expected source tree."""
    with tarfile.open(archive) as bundle:
        for member in bundle.getmembers():
            path = Path(member.name)
            if (path.is_absolute() or ".." in path.parts or not path.parts
                    or path.parts[0] != f"pipewire-{VERSION}"
                    or not (member.isfile() or member.isdir())):
                raise RuntimeError(f"Unexpected archive member: {member.name}")
        bundle.extractall(destination, filter="data")


def prepare_source(build, archive=None):
    build.mkdir(parents=True, exist_ok=True)
    archive = Path(archive) if archive else build / f"pipewire-{VERSION}.tar.gz"
    if not archive.exists():
        if archive != build / f"pipewire-{VERSION}.tar.gz":
            raise RuntimeError(f"Archive does not exist: {archive}")
        temporary = archive.with_suffix(".download")
        try:
            with urllib.request.urlopen(URL, timeout=60) as source, temporary.open("wb") as output:
                shutil.copyfileobj(source, output)
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise RuntimeError("Upstream archive checksum mismatch; source was not extracted")
    # Extract into a fresh temporary tree; never mutate a user's existing source.
    with tempfile.TemporaryDirectory(prefix=".source-", dir=build) as temporary:
        extract_archive(archive, temporary)
        pristine = Path(temporary) / f"pipewire-{VERSION}"
        source = build / f"pipewire-{VERSION}"
        if source.exists():
            raise RuntimeError(f"Build source already exists: {source}; use a fresh --build-dir")
        pristine.rename(source)
    (build / "gstpipewirepool.c.original").write_bytes((source / "src/gst/gstpipewirepool.c").read_bytes())
    subprocess.run(["patch", "--batch", "--forward", "-p1", "-i",
                    str(ROOT / "patches/gstpipewirepool-busy-slot.patch")], cwd=source, check=True)
    return source


def build_plugin(build, source):
    if pkg("--modversion", "libpipewire-0.3") != VERSION:
        raise RuntimeError(f"This patch/build supports PipeWire {VERSION}; port and validate other versions first")
    library = ctypes.CDLL("libpipewire-0.3.so.0")
    library.pw_get_library_version.restype = ctypes.c_char_p
    if library.pw_get_library_version().decode() != VERSION:
        raise RuntimeError("Installed PipeWire runtime and pinned source version do not match")
    subprocess.run(["pkg-config", "--atleast-version=1.24", "gstreamer-1.0"], check=True)
    flags = shlex.split(pkg("--cflags", "--libs", *DEPENDENCIES))
    compiler = os.environ.get("CC", "cc")
    defines = [f'#define PACKAGE "pipewire"', f'#define PACKAGE_VERSION "{VERSION}"',
               "#define HAVE_GSTREAMER_DEVICE_PROVIDER 1"]
    for feature, header in (("DMA_DRM", "gst/video/video-info-dma-drm.h"),
                            ("SHM_ALLOCATOR", "gst/allocators/gstshmallocator.h")):
        probe = subprocess.run([compiler, "-x", "c", "-fsyntax-only", "-", *flags],
                               input=f"#include <{header}>\n", text=True, capture_output=True)
        if probe.returncode == 0:
            defines.append(f"#define HAVE_GSTREAMER_{feature} 1")
    (build / "config.h").write_text("\n".join(defines) + "\n")
    gst = source / "src/gst"
    output = build / "libgstpipewire.so"
    command = [compiler, "-shared", "-fPIC", "-O2", "-std=gnu11", "-D_GNU_SOURCE",
               "-Wall", "-Wextra", "-Wno-unused-parameter", "-Wno-missing-field-initializers",
               "-Wl,-z,defs", "-I" + str(build), "-I" + str(gst), "-I" + str(gst.parent),
               *[str(gst / f"{name}.c") for name in NAMES], *flags, "-lm", "-o", str(output)]
    (build / "compile-command.txt").write_text(shlex.join(command) + "\n")
    subprocess.run(command, check=True)
    result = subprocess.run(["ldd", "-r", str(output)], text=True, capture_output=True, check=True)
    (build / "link-check.txt").write_text(result.stdout + result.stderr)
    if "undefined symbol:" in result.stdout + result.stderr or "not found" in result.stdout + result.stderr:
        raise RuntimeError("Plugin link validation failed")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, help="Existing pinned upstream tar.gz (offline build)")
    parser.add_argument("--build-dir", type=Path, default=ROOT / "build/plugin")
    parser.add_argument("--source-only", action="store_true", help="Prepare source for deterministic tests without development headers")
    args = parser.parse_args()
    if os.geteuid() == 0:
        parser.error("Build as your regular user, without sudo")
    build = args.build_dir.resolve()
    source = prepare_source(build, args.archive)
    if not args.source_only:
        print(build_plugin(build, source))
    else:
        print(source)


if __name__ == "__main__":
    main()
