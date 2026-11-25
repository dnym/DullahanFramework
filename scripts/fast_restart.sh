#!/usr/bin/env bash
# Tier 3 sub-second rebuild + schema-tolerant state snapshot restart.
# Usage:
#   ./scripts/fast_restart.sh <binary_name> [extra app args...]
#   DULLAHAN_BIN=my_game ./scripts/fast_restart.sh [extra app args...]
set -euo pipefail

SOCK="${DULLAHAN_SOCK:-/tmp/dullahan.sock}"
SNAP="${DULLAHAN_SNAP:-/tmp/dullahan_state.snap}"

if [ -n "${DULLAHAN_BIN:-}" ]; then
  BIN="$DULLAHAN_BIN"
elif [ $# -ge 1 ]; then
  BIN="$1"
  shift
else
  echo "Usage: $0 <binary_name> [extra args...]" >&2
  echo "   or: DULLAHAN_BIN=<binary_name> $0 [extra args...]" >&2
  exit 1
fi

if [ -S "$SOCK" ]; then
  ./target/debug/dctl --sock "$SOCK" snapshot save "$SNAP" || true
fi

cargo build --bin "$BIN" --bin dctl

if [ -S "$SOCK" ]; then
  ./target/debug/dctl --sock "$SOCK" quit || true
  for _ in $(seq 1 20); do
    [ ! -S "$SOCK" ] && break
    sleep 0.02
  done
fi

exec "./target/debug/$BIN" --sock "$SOCK" --restore "$SNAP" "$@"
