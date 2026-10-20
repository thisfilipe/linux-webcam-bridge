#!/bin/sh
# Build the isolated patched GStreamer PipeWire plugin.
#
# This is deliberately a shell build wrapper around the C plugin and the
# upstream build dependencies; the camera runtime and installers are Rust.
# The pinned archive is checksum-verified, extracted into a fresh directory,
# patched, compiled and link-checked. No system library is replaced.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VERSION=1.6.8
URL="https://github.com/PipeWire/pipewire/archive/refs/tags/$VERSION.tar.gz"
SHA256=8181172a1d95131f6af8bbc0b98f90b2a33349b042b84c3ce57dd5d11348cc58
DEPENDENCIES="gstreamer-1.0 gstreamer-base-1.0 gstreamer-video-1.0 gstreamer-audio-1.0 gstreamer-net-1.0 gstreamer-allocators-1.0 libpipewire-0.3 gio-unix-2.0"
NAMES="gstpipewire gstpipewirecore gstpipewireclock gstpipewireformat gstpipewirepool gstpipewiresink gstpipewiresrc gstpipewirestream gstpipewiredeviceprovider"
BUILD="$ROOT/build/plugin"
ARCHIVE=
SOURCE_ONLY=0

usage() {
    echo "usage: $0 [--archive PATH] [--build-dir DIR] [--source-only]" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --archive)
            [ "$#" -ge 2 ] || usage
            ARCHIVE=$2
            shift 2
            ;;
        --build-dir)
            [ "$#" -ge 2 ] || usage
            BUILD=$2
            shift 2
            ;;
        --source-only)
            SOURCE_ONLY=1
            shift
            ;;
        -h|--help) usage ;;
        *) usage ;;
    esac
done

if [ "$(id -u)" -eq 0 ]; then
    echo "error: build as your regular user, without sudo" >&2
    exit 1
fi
if ! command -v sha256sum >/dev/null 2>&1 || ! command -v tar >/dev/null 2>&1 \
    || ! command -v patch >/dev/null 2>&1 || ! command -v pkg-config >/dev/null 2>&1; then
    echo "error: sha256sum, tar, patch and pkg-config are required" >&2
    exit 1
fi

mkdir -p "$BUILD"
if [ -z "$ARCHIVE" ]; then
    ARCHIVE="$BUILD/pipewire-$VERSION.tar.gz"
fi
if [ ! -f "$ARCHIVE" ]; then
    temporary="$ARCHIVE.download"
    trap 'rm -f "$temporary"' EXIT HUP INT TERM
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 3 --connect-timeout 20 -o "$temporary" "$URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$temporary" "$URL"
    else
        echo "error: curl or wget is required to download $URL" >&2
        exit 1
    fi
    mv "$temporary" "$ARCHIVE"
    trap - EXIT HUP INT TERM
fi
actual=$(sha256sum "$ARCHIVE" | cut -d' ' -f1)
if [ "$actual" != "$SHA256" ]; then
    echo "error: upstream archive checksum mismatch; source was not extracted" >&2
    exit 1
fi

source="$BUILD/pipewire-$VERSION"
if [ -e "$source" ]; then
    echo "error: build source already exists: $source; use a fresh --build-dir" >&2
    exit 1
fi
temporary=$(mktemp -d "$BUILD/.source.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
tar -xzf "$ARCHIVE" -C "$temporary" --no-same-owner --no-same-permissions
if [ ! -d "$temporary/pipewire-$VERSION" ]; then
    echo "error: archive did not contain pipewire-$VERSION" >&2
    exit 1
fi
mv "$temporary/pipewire-$VERSION" "$source"
rmdir "$temporary"
trap - EXIT HUP INT TERM

cp "$source/src/gst/gstpipewirepool.c" "$BUILD/gstpipewirepool.c.original"
if ! (cd "$source" && patch --batch --forward -p1 -i "$ROOT/patches/gstpipewirepool-busy-slot.patch"); then
    echo "error: the pinned patch did not apply" >&2
    exit 1
fi

if [ "$SOURCE_ONLY" -eq 1 ]; then
    printf '%s\n' "$source"
    exit 0
fi

runtime=$(pkg-config --modversion libpipewire-0.3)
if [ "$runtime" != "$VERSION" ]; then
    echo "error: this patch/build supports PipeWire $VERSION; found $runtime" >&2
    exit 1
fi
if command -v pw-cli >/dev/null 2>&1; then
    runtime_cli=$(pw-cli --version 2>/dev/null | awk 'END {print $NF}')
    if [ "$runtime_cli" != "$VERSION" ]; then
        echo "error: installed PipeWire runtime is $runtime_cli; expected $VERSION" >&2
        exit 1
    fi
fi
pkg-config --atleast-version=1.24 gstreamer-1.0 || {
    echo "error: GStreamer >= 1.24 is required" >&2
    exit 1
}

# Expand pkg-config output into the argument list without invoking a shell.
set -- $(pkg-config --cflags --libs $DEPENDENCIES)
flags=$*
compiler=${CC:-cc}
config="$BUILD/config.h"
: >"$config"
printf '#define PACKAGE "pipewire"\n#define PACKAGE_VERSION "%s"\n#define HAVE_GSTREAMER_DEVICE_PROVIDER 1\n' "$VERSION" >>"$config"
for probe in \
    "DMA_DRM:gst/video/video-info-dma-drm.h" \
    "SHM_ALLOCATOR:gst/allocators/gstshmallocator.h"; do
    feature=${probe%%:*}
    header=${probe#*:}
    if printf '#include <%s>\n' "$header" | "$compiler" -x c -fsyntax-only - $flags >/dev/null 2>&1; then
        printf '#define HAVE_GSTREAMER_%s 1\n' "$feature" >>"$config"
    fi
done

gst="$source/src/gst"
output="$BUILD/libgstpipewire.so"
set -- "$compiler" -shared -fPIC -O2 -std=gnu11 -D_GNU_SOURCE \
    -Wall -Wextra -Wno-unused-parameter -Wno-missing-field-initializers \
    -Wl,-z,defs -I"$BUILD" -I"$gst" -I"$gst/.."
for name in $NAMES; do
    set -- "$@" "$gst/$name.c"
done
set -- "$@" $flags -lm -o "$output"
# The arguments were assembled as shell words on purpose: pkg-config emits
# trusted compiler flags, never user input.
"$@"
ldd -r "$output" >"$BUILD/link-check.txt" 2>&1
if grep -Eq 'undefined symbol:|not found' "$BUILD/link-check.txt"; then
    cat "$BUILD/link-check.txt" >&2
    echo "error: plugin link validation failed" >&2
    exit 1
fi
printf '%s\n' "$output"
