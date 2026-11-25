#!/usr/bin/env bash
# Tier 3 sub-second rebuild + schema-tolerant state snapshot restart.
set -euo pipefail

SOCK="${DULLAHAN_SOCK:-/tmp/dullahan.sock}"
SNAP="${DULLAHAN_SNAP:-/tmp/dullahan_state.snap}"

if [ -S "$SOCK" ]; then
  ./target/debug/dctl --sock "$SOCK" snapshot save "$SNAP" || true
fi

cargo build --bin dullahan --bin dctl

if [ -S "$SOCK" ]; then
  ./target/debug/dctl --sock "$SOCK" quit || true
  for _ in $(seq 1 20); do
    [ ! -S "$SOCK" ] && break
    sleep 0.02
  done
fi

exec ./target/debug/dullahan --sock "$SOCK" --restore "$SNAP" "$@"
