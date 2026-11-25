# Dullahan Framework

A **zero-dependency, flat, hot-reloadable, LLM-first Rust game framework** (Linux-first) built for sub-second iteration, Git-friendly data-driven design, and closed-loop visual/state verification.

![NEON RUSH — Synthwave Cyber-Runner built on Dullahan Framework](docs/showcase_action.png)

> **Status:** Active dogfooding showcase — built with **0 external Cargo crates** (`[dependencies]` is empty).

---

## Showcase Demo: NEON RUSH

Included out of the box is **NEON RUSH**, a juiced-up Synthwave endless cyber-runner dogfooding every subsystem of Dullahan Framework:

- **3D PBR + Shadows + Post-Processing:** Shared 0.94 KB PBR palette across 11 `.glb` models, 2048×2048 PCF shadows, dynamic hoverboard/headlight point lights, twilight sky gradient, translucent orbital Holo-Shield, and a custom hot-reloadable synthwave bloom + radial chromatic aberration shader (`assets/shaders/post.frag`).
- **Gameplay & Juice:** 3-lane cyber-highway with launch ramps onto train roofs, oncoming cyber-trains, low plasma barriers, slide-under laser gates, coyote time + jump buffering + fast-fall slam, near-miss callouts (`CLOSE CALL! +50`), camera roll/shake/FOV kick, hitstop, and 3D billboard particle bursts.
- **Procedural Synthwave Audio:** Seamless 124 BPM synthwave loop (`seam=0.0001`) + 7 synth SFX mixed on a 48 kHz lock-free ALSA thread (with combo-escalating coin pitch).
- **Pure-Rust Vector UI:** Anti-aliased `.svg` HUD icons & Outrun sunset disc rasterized directly at runtime + interactive `ui_button` controls and built-in bitmap font.

### Controls
| Key | Action |
| :--- | :--- |
| `A` / `D` or `Left` / `Right` | Switch lanes (hover-bank + camera tilt) |
| `W` / `Up` / `Space` | Jump (coyote time + input buffer; ride train roofs!) |
| `S` / `Down` | Cyber-slide under laser gates / fast-fall slam in mid-air |
| `M` | Toggle audio mute |
| `F1` | Cycle debug overlays (`colliders`, `wireframe`, `grid`, `none`) |
| `R` / `Space` | Instant restart on wipeout |

---

## Why Dullahan?

- **Zero Cargo dependencies:** Pure Rust + minimal `extern "C"` Linux OS FFI (`libX11`, `libGL`, `libEGL`, `libasound`). Clean builds in **~1.1s**, incremental rebuilds in **~0.25s**, zero crate API drift.
- **Flat, explicit, non-constraining:** The game owns `fn main()`, the loop, and one flat `GameState` backed by plain `Vec`s or generational `Pool<T>` arenas. No forced ECS, OOP scene graph, or monolithic physics engine.
- **Standard formats only:** Pure-Rust loaders for `.glb`/`.gltf` (PBR + named child nodes + `col` colliders), `.png` (8-bit & 16-bit heightmaps + PNG encoder), `.svg` (4×4 AA rasterizer), `.wav`, and `.glsl`. Works with Dullahan Artisan or any external assets.
- **Git-friendly data & 3-tier live iteration:**
  1. **Assets & shaders** (`.glb`, `.png`, `.svg`, `.wav`, `pbr.vert/.frag`, `post.frag`) hot-reload live in-place with safe error fallbacks.
  2. **Tuning, tables & levels** (`data/runner.cfg`, `.tbl`, `.lvl`) hot-reload live with 1-line Git diffs.
  3. **Rust logic** uses sub-second rebuild + schema-tolerant plaintext state snapshots (`./scripts/fast_restart.sh`) instead of fragile `dlopen`.
- **Closed-loop runtime verification (zero MCP):** Control and inspect live or `--headless` runs via the tiny `dctl` Unix-socket CLI.

---

## Quick Start (Linux)

```bash
# Run unit tests (0 external crates)
cargo test

# Play NEON RUSH on desktop
cargo run --bin dullahan

# Live-tweak & inspect from another terminal while playing:
cargo run --bin dctl -- state
cargo run --bin dctl -- set post.bloom 0.9
cargo run --bin dctl -- set player.jump_vel 17.0
cargo run --bin dctl -- overlay colliders
cargo run --bin dctl -- screenshot /tmp/live.png

# Headless deterministic render (works without X11 / DISPLAY):
./target/debug/dullahan --headless --size 1280x720 --frames 15 --snap /tmp/title.png
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — Full framework specification, module boundaries, and format contracts.
- [skills/dullahan-framework/SKILL.md](skills/dullahan-framework/SKILL.md) — Companion generic LLM skill for building & verifying games on Dullahan.
