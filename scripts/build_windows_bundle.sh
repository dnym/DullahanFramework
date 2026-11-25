#!/usr/bin/env bash
# Cross-compiles Dullahan workspace binaries as portable Windows x64 `.exe`
# bundles inside `dist/dullahan-win64/`, ready to copy onto a USB stick.
#
# Usage:
#   ./scripts/build_windows_bundle.sh            # auto-discovers all crates in demos/*/
#   ./scripts/build_windows_bundle.sh my_game    # packages specific package name(s)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "Missing Windows cross-linker: x86_64-w64-mingw32-gcc"
    echo "Install it once:"
    echo "  Arch / CachyOS:  sudo pacman -S --needed mingw-w64-gcc"
    echo "  Ubuntu / Debian: sudo apt install gcc-mingw-w64-x86-64"
    exit 1
fi

rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true

TARGETS=()
if [ $# -gt 0 ]; then
    TARGETS=("$@")
elif [ -d "$ROOT/demos" ]; then
    for dir in "$ROOT"/demos/*/; do
        [ -f "${dir}Cargo.toml" ] && TARGETS+=("$(basename "$dir")")
    done
fi

if [ ${#TARGETS[@]} -eq 0 ]; then
    echo "No target packages specified or found under demos/." >&2
    echo "Usage: $0 <package_name> [...]" >&2
    exit 1
fi

echo "[1/2] Building release Windows x64 binaries (0 external Cargo crates)..."
BUILD_ARGS=(--release --target x86_64-pc-windows-gnu --bin dctl)
for pkg in "${TARGETS[@]}"; do
    BUILD_ARGS+=(-p "$pkg")
done
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc cargo build "${BUILD_ARGS[@]}"

OUT="$ROOT/dist/dullahan-win64"
rm -rf "$OUT"
mkdir -p "$OUT"

echo "[2/2] Packaging portable bundle into $OUT ..."
for pkg in "${TARGETS[@]}"; do
    mkdir -p "$OUT/$pkg"
    cp "target/x86_64-pc-windows-gnu/release/${pkg}.exe" "$OUT/$pkg/"
    SRC_DIR="$ROOT/demos/$pkg"
    [ ! -d "$SRC_DIR" ] && SRC_DIR="$ROOT"
    [ -d "$SRC_DIR/assets" ] && cp -r "$SRC_DIR/assets" "$OUT/$pkg/assets"
    [ -d "$SRC_DIR/data" ] && cp -r "$SRC_DIR/data" "$OUT/$pkg/data"
done
cp "target/x86_64-pc-windows-gnu/release/dctl.exe" "$OUT/"

echo "Done! Copy '$OUT' onto a USB stick and double-click any .exe on Windows."
du -sh "$OUT"
