---
name: dullahan-framework
description: >-
  Architecture rules, flat Rust game-dev idioms, Git-friendly data patterns,
  3-tier hot-reload workflow, and closed-loop visual/state verification for
  games built with the zero-dependency Dullahan Framework (`src/dullahan/`).
  Use whenever creating, modifying, tuning, or debugging a Dullahan game or
  extending the vendored framework modules.
---

# Dullahan Framework — LLM-First Game Development Guide

Dullahan is a **vendored, zero-dependency, flat Rust game framework**. The game owns `fn main()`, the main loop, and `GameState`; `src/dullahan/` provides OS/GPU/Audio/Asset/Data/Physics plumbing that you can read and extend directly when a game hits an edge case.

---

## 1. Core Architectural Rules

1. **Zero Cargo dependencies.** Keep `Cargo.toml` `[dependencies]` empty. Incremental builds stay ~0.2–0.5s, offline, with zero crate API drift.
2. **Flat `GameState`, never ECS or deep OOP.** Store entities in plain typed `Vec<T>` or generational `Pool<T>` arenas inside one top-level `GameState` struct.
3. **Explicit top-to-bottom control flow.** Call `update(...)` steps linearly in visible order. Keep `update()` deterministic and free of GL calls; keep `render()` read-only over `&GameState`.
4. **Immediate-mode rendering, debug draw, and UI.** Push meshes, boxes, billboards, debug lines, and UI quads into `app.draw` every frame—never mirror game state in a retained scene graph.
5. **Standard formats only.** Load `.glb`/`.gltf`, `.png`, `.svg`, `.wav`, and `.glsl` directly from `assets/`. Never couple runtime code to external asset compilers.
6. **Git-friendly plaintext data.** Put tweakable feel parameters in `data/*.cfg`, item/entity stats in `data/*.tbl`, and spatial layouts in `data/*.lvl`—never bury tuning numbers as hardcoded magic floats.

---

## 2. Quick API Surface (`src/dullahan/`)

| Need | Module / API |
| :--- | :--- |
| App loop & CLI | `App::from_args(title, w, h)` (`--headless`, `--size WxH`, `--frames N`, `--dt S`, `--snap out.png`, `--sock path`, `--script file`, `--restore file.snap`) |
| Assets (Tier 1 hot-reload) | `app.load_mesh("x.glb")`, `app.load_texture("x.png", true)`, `app.load_svg("icon.svg", 64, 64)`, `app.load_sound("x.wav")`, `app.create_terrain_mesh(&hf, tex, 64, 64)` |
| 3D PBR & Lighting | `draw.camera`, `draw.sun_dir`, `draw.shadows_enabled`, `draw.point_light(pos, col, rad, int)`, `draw.mesh(h, tr)`, `draw.mesh_nodes(h, tr, &[(node_idx, local_tr)])`, `draw.billboard(tex, pos, size, col)` |
| Post-Processing | `draw.post` (`enabled`, `exposure`, `contrast`, `saturation`, `vignette`, `bloom_strength`, `bloom_threshold`, `tint`) + hot-reloadable `assets/shaders/post.frag` & `pbr.vert/.frag` |
| 2D & Interactive UI | `Camera2D`, `draw.ui_rect`, `draw.ui_panel`, `draw.ui_bar`, `draw.ui_sprite`, `draw.ui_sprite_rot`, `draw.ui_nine_slice`, `draw.ui_text`, `draw.ui_text_measure`, `draw.ui_button(&app.input, x, y, w, h, label)` |
| Audio | `app.play_sound(h, PlayParams { gain, pitch, pan, bus, looping })`, `app.play_loop(h, gain, bus)`, `app.audio.play_variant(...)`, `app.audio.set_voice(...)`, `app.audio.set_bus_gain(...)` |
| Data & Arenas (Tier 2) | `Pool<T>` / `Handle<T>`, `app.data.load_config("tuning.cfg")`, `load_table("items.tbl")`, `load_level("lvl.lvl")`, `Snapshot` (`app.startup_snapshot()`, `app.handle_snapshots(...)`) |
| Physics & Collision | `geom::{Rect2D, circle_overlaps, circle_rect_pushout, sphere_overlaps, ray_sphere, ray_plane, ray_triangle, aabb_pushout, sphere_aabb_pushout, capsule_aabb_pushout, HeightField, RigidBody, resolve_body_pair}` |
| Math & PRNG | `math::{Vec2, Vec3, Vec4, Quat, Mat4, Transform, Aabb, Ray, Color, Rng, smoothstep, ease_out_cubic}` |

---

## 3. 3-Tier Live Iteration Workflow

Choose the lightest tier for every change:

1. **Tier 1 (Instant, ~10ms):** Edit `.glb`, `.gltf`, `.png`, `.svg`, `.wav`, or `assets/shaders/{pbr.vert,pbr.frag,post.frag}`. The running game swaps GPU/audio resources in-place behind stable handles; syntax/parse errors log cleanly and keep the last valid resource live.
2. **Tier 2 (Instant, <1ms):** Edit `data/*.cfg`, `data/*.tbl`, or `data/*.lvl`—or test a value live with `dctl set <key> <value>` before committing it to `.cfg`.
3. **Tier 3 (~0.5s):** For `.rs` logic edits, use snapshot-preserving fast restart (`./scripts/fast_restart.sh`): saves `Snapshot` → `cargo build` (~0.3–0.8s) → relaunches with `--restore`. Schema-tolerant plaintext snapshots survive added/removed fields with zero ABI crashes.

---

## 4. Closed-Loop Verification (Always Verify Visually!)

Never guess whether lighting, shadows, collisions, UI alignment, or shaders look right.

### Interactive Instance (`dctl`)
```bash
cargo run --bin dctl -- state
cargo run --bin dctl -- screenshot /tmp/qa.png
cargo run --bin dctl -- overlay colliders    # none | grid | wireframe | colliders
cargo run --bin dctl -- pause && cargo run --bin dctl -- step 10
cargo run --bin dctl -- set player.jump_impulse 13.5
cargo run --bin dctl -- reload
```

### Headless Deterministic QA (Works Without X11 / `DISPLAY`)
```bash
./target/debug/dullahan --headless --size 960x540 --frames 30 --snap /tmp/qa.png
```
Inspect `/tmp/qa.png` to verify:
- No clipped HUD text or overlapping panels (use `draw.ui_text_measure` to size badges dynamically).
- Grounding shadows, readable contrast, and accurate collider fit (`overlay colliders`).
- `SUMMARY` line reports expected entity/prop counts, zero NaNs, and zero reload errors.

---

## 5. Extending the Vendored Framework

When a game needs a new engine primitive (e.g., decals, custom shader uniform, gamepad axis, instanced grass):
- Add the smallest flat primitive directly inside `src/dullahan/`—avoid generic trait frameworks or external crates.
- Pair every pure-Rust module change with a fast unit test (`cargo test` runs in < 0.3s) and a headless visual check if rendering is affected.
