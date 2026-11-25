#!/usr/bin/env bash
# Builds all 3 Dullahan showcase demos + dctl as standalone Windows x64 `.exe`
# bundles inside `dist/dullahan-win64/`, ready to copy straight onto a USB stick.
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

echo "[1/2] Building release Windows x64 binaries (0 external Cargo crates)..."
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    cargo build --release --workspace --target x86_64-pc-windows-gnu

OUT="$ROOT/dist/dullahan-win64"
rm -rf "$OUT"
mkdir -p "$OUT"

echo "[2/2] Packaging portable USB bundle into $OUT ..."
for demo in neon_rush mahjong_wars cozy_golf; do
    mkdir -p "$OUT/$demo"
    cp "target/x86_64-pc-windows-gnu/release/${demo}.exe" "$OUT/$demo/"
    cp -r "demos/${demo}/assets" "$OUT/$demo/assets"
    cp -r "demos/${demo}/data" "$OUT/$demo/data"
done
cp "target/x86_64-pc-windows-gnu/release/dctl.exe" "$OUT/"

cat << 'EOF' > "$OUT/README_PLAY.txt"
DULLAHAN FRAMEWORK — WINDOWS PORTABLE SHOWCASE BUNDLE
=====================================================
Double-click any game .exe inside its folder!

1. neon_rush\neon_rush.exe
   A/D: Switch lanes | W/Space: Jump | S: Slide/Slam | M: Mute | F1: Debug

2. mahjong_wars\mahjong_wars.exe
   WASD: Move | Mouse: Look | Space: Jump/Vault | Shift/C: Power Slide
   LMB/F: Laser Rifle | E/RMB: Touch Free Tile | Esc: Pause & Settings

3. cozy_golf\cozy_golf.exe
   LMB Drag: Aim & Putt | RMB: Cancel | R: Reset Hole | Esc: Pause & Settings
EOF

echo "Done! Copy '$OUT' onto a USB stick and double-click any .exe on Windows 11."
du -sh "$OUT"
