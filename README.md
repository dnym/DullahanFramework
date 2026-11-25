# Dullahan Framework

A **zero-dependency, flat, hot-reloadable, LLM-first Rust game framework** (Linux-first) built for sub-second iteration, Git-friendly data-driven design, and closed-loop visual/state verification.

> **Status:** Early experimental / active dogfooding.

## Why Dullahan?

Traditional engines optimize for large human studios with deep OOP hierarchies, ECS scheduling, reflection, and 400-crate dependency trees—all of which slow down and confuse agentic coding workflows. Dullahan takes the opposite approach:

- **Zero Cargo dependencies (`[dependencies]` is empty):** Pure Rust + minimal `extern "C"` Linux OS FFI (`libX11`, `libGL`, `libEGL`, `libasound`). Compiles offline in **~1s clean / ~0.25s incremental** with zero crate API drift.
- **Flat, explicit, non-constraining:** The game owns `fn main()`, the loop, and a single flat `GameState` backed by plain typed arrays or generational `Pool<T>` arenas. No forced ECS, scene graph, or physics engine.
- **Standard asset formats only:** Pure-Rust loaders for `.glb`/`.gltf` (PBR metallic-roughness + ORM + emissive + named child nodes + `col` colliders), `.png` (8-bit & 16-bit heightmaps + PNG encoder), `.svg` (4×4 anti-aliased UI rasterizer), `.wav`, and `.glsl`. Works seamlessly with [Dullahan Artisan](https://github.com/jayzone80) or any external assets.
- **Git-friendly data & 3-tier live iteration:**
  1. **Assets & shaders** (`.glb`, `.png`, `.svg`, `.wav`, `pbr.vert/.frag`, `post.frag`) hot-reload live in-place with safe fallbacks.
  2. **Tuning, tables & levels** (`.cfg`, `.tbl`, `.lvl`) hot-reload live with 1-line Git diffs.
  3. **Rust logic** uses sub-second rebuild + schema-tolerant plaintext state snapshots (`./scripts/fast_restart.sh`) instead of fragile `dlopen`.
- **Closed-loop runtime verification (zero MCP):** Control and inspect live or `--headless` runs via the tiny `dctl` Unix-socket CLI (`state`, `screenshot`, `pause`, `step`, `overlay`, `set`, `reload`, `snapshot`, `input`).

## Quick Start (Linux)

```bash
# Run unit tests (0 external crates)
cargo test

# Launch interactive desktop window (WASD/mouse drag to orbit, F1 overlays, Space bounces ball)
cargo run --bin dullahan

# In another terminal, inspect or control the live game:
cargo run --bin dctl -- state
cargo run --bin dctl -- overlay colliders
cargo run --bin dctl -- set ground.color "#C2A670"
cargo run --bin dctl -- screenshot /tmp/live.png

# Headless deterministic render (works without X11 / DISPLAY):
./target/debug/dullahan --headless --size 960x540 --frames 45 --snap /tmp/qa.png
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — Full design spec, module layout, and asset/data contracts.
- [skills/dullahan-framework/SKILL.md](skills/dullahan-framework/SKILL.md) — Companion LLM skill with flat Rust idioms and closed-loop QA practices.
