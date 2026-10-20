#!/bin/sh
# Reproducible release build for the root USB reset helper.
#
# build:  build twice from independent target directories, compare the bytes,
#         then copy the binary and its SHA-256 manifest to OUT (default dist/).
# verify: build twice and fail unless the two binaries are byte-identical.
#
# No system paths are written to the repository. Paths that Rust embeds are
# remapped to stable virtual prefixes so independent builders can reproduce the
# same bytes with the pinned toolchain.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MODE=${1:-build}
OUT=${2:-"$ROOT/dist"}
PINNED_RUSTC=1.99.0
VIRTUAL_ROOT=/build/linux-webcam-bridge
VIRTUAL_CARGO=/cargo
VIRTUAL_RUST=/rust
VIRTUAL_TARGET=/target

usage() {
    echo "usage: $0 [build|verify] [output-directory]" >&2
    exit 2
}

case "$MODE" in
    build|verify) ;;
    *) usage ;;
esac

if ! command -v rustc >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
    echo "error: rustc and cargo are required" >&2
    exit 1
fi

TOOLCHAIN=$(rustc --version)
case "$TOOLCHAIN" in
    "rustc $PINNED_RUSTC "*) ;;
    *)
        echo "error: reproducible build requires rustc $PINNED_RUSTC; found: $TOOLCHAIN" >&2
        exit 1
        ;;
esac

SYSROOT=$(rustc --print sysroot)
CARGO_HOME_DIR=${CARGO_HOME:-"$HOME/.cargo"}
TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/jieli-usb-repro.XXXXXX")
trap 'rm -rf "$TMP_ROOT"' EXIT HUP INT TERM

build_once() {
    target_dir=$1
    CARGO_TARGET_DIR="$target_dir" \
    CARGO_HOME="$CARGO_HOME_DIR" \
    SOURCE_DATE_EPOCH=0 \
    CARGO_INCREMENTAL=0 \
    RUSTFLAGS="--remap-path-prefix=$ROOT=$VIRTUAL_ROOT \
--remap-path-prefix=$CARGO_HOME_DIR=$VIRTUAL_CARGO \
--remap-path-prefix=$SYSROOT=$VIRTUAL_RUST \
--remap-path-prefix=$target_dir=$VIRTUAL_TARGET" \
    cargo build --locked --release -p jieli-usb
}

echo "building with $TOOLCHAIN" >&2
build_once "$TMP_ROOT/a"
build_once "$TMP_ROOT/b"

FIRST="$TMP_ROOT/a/release/jieli-usb-reset"
SECOND="$TMP_ROOT/b/release/jieli-usb-reset"
if ! cmp -s "$FIRST" "$SECOND"; then
    echo "error: independent builds differ; reproducibility is broken" >&2
    sha256sum "$FIRST" "$SECOND" >&2
    exit 1
fi

if [ "$MODE" = "verify" ]; then
    echo "reproducible: $(sha256sum "$FIRST" | cut -d' ' -f1)"
    exit 0
fi

mkdir -p "$OUT"
cp "$FIRST" "$OUT/jieli-usb-reset"
chmod 0755 "$OUT/jieli-usb-reset"
(
    cd "$OUT"
    sha256sum jieli-usb-reset > jieli-usb-reset.sha256
)
echo "wrote $OUT/jieli-usb-reset"
cat "$OUT/jieli-usb-reset.sha256"
