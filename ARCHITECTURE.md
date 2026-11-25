# Dullahan Framework — Architecture Specification

**Status:** Early v0.1 (Linux & Windows)  
**Goal:** A zero-dependency, flat, hot-reloadable, LLM-first Rust game framework designed for sub-second iteration, Git-friendly data-driven development, and closed-loop visual/state verification.

---

## 1. Design Manifesto

Modern game engines optimize for large human teams (deep inheritance trees, ECS archetype scheduling, reflection, GUI editors, and 400-crate dependency graphs). Those abstractions actively fight LLM-driven development by hiding control flow, breaking across crate versions, and slowing builds to a crawl.

Dullahan Framework inverts those priorities around six strict invariants:

1. **Zero Cargo Dependencies (`[dependencies]` is empty).**  
   Pure Rust + minimal `extern "C"` OS FFI. Every line of engine code lives in the repository, compiles offline in under a second, never suffers from crate API rot, and can be read and modified directly by an LLM when a game hits an edge case.
2. **Flat, Explicit Control Flow (No Forced ECS or OOP).**  
   The *game* owns `fn main()`, the main loop, and a single explicit `GameState` struct backed by plain typed arrays / generational pools (`Pool<T>`). Systems are plain functions called in visible top-to-bottom order. Rendering, UI, and debug drawing are immediate-mode—no retained scene graph to drift out of sync.
3. **Standard Formats Only (Decoupled from Asset Tools).**  
   The runtime loads standard `.glb`/`.gltf`, `.png`, `.svg`, `.wav`, and `.glsl` files directly. It pairs effortlessly with Dullahan Artisan, Blender, external asset packs, or hand-authored files without coupling the engine to any generator.
4. **Git-Friendly Data-Driven Design.**  
   Gameplay tuning, entity tables, and level layouts live in compact, line-oriented plaintext files (`.cfg`, `.tbl`, `.lvl`). Every tweak produces a 1-line Git diff and hot-reloads live on the next frame.
5. **Rock-Solid 3-Tier Live Iteration (No Fragile `dlopen`).**  
   Assets/shaders reload in-place, data/tuning reloads in-place, and Rust code changes use sub-second rebuild + schema-tolerant state snapshots—eliminating ABI crashes and UB while preserving instant iteration feel.
6. **Closed-Loop Runtime Observability (Zero MCP).**  
   A running game exposes a tiny local control socket/CLI so an LLM can capture live screenshots, inspect structured state telemetry, step frames deterministically, toggle debug overlays, and run headless playtests.

---

## 2. Repository & Module Layout

Dullahan is designed to be **vendored directly into a game repository** (or cloned as a starter template) alongside a generic companion LLM skill. The total framework footprint targets **~5,000–6,500 LOC** of straightforward, macro-free Rust.

```text
DullahanFramework/
├── Cargo.toml                     # Zero external crates ([dependencies] empty)
├── ARCHITECTURE.md                # This specification
├── skills/
│   └── dullahan-framework/
│       └── SKILL.md               # Generic LLM guidelines, idioms & closed-loop workflow
├── src/
│   ├── main.rs                    # Game entry point (owns loop & GameState)
│   ├── game/                      # Game-specific flat code (update.rs, render.rs, etc.)
│   ├── bin/
│   │   └── dctl.rs                # Tiny CLI client for the runtime control socket (~120 LOC)
│   └── dullahan/                  # Vendored framework (~5–6.5k LOC total)
│       ├── mod.rs                 # Public facade & App frame lifecycle
│   │   ├── platform/
│   │   │   ├── mod.rs             # Thin OS-neutral Platform + OpenGL 3.3 Core table
│   │   │   ├── linux.rs           # X11 + GLX (GL 3.3 Core) + EGL headless + FPS cursor lock
│   │   │   └── windows.rs         # Win32 + WGL (GL 3.3 Core) + hidden-window FBO + FPS cursor lock
│   │   ├── math.rs                # Plain-field Vec2/3/4, Mat4, Quat, Transform, Aabb, Ray
│   │   ├── formats/
│   │   │   ├── mod.rs
│   │   │   ├── json.rs            # Pure-Rust RFC 8259 JSON parser
│   │   │   ├── png.rs             # RFC 1950/1951 PNG decode (8-bit RGB/RGBA + 16-bit gray) & encode
│   │   │   ├── wav.rs             # PCM 16-bit & float32 WAV loader + linear resampler
│   │   │   ├── gltf.rs            # glTF 2.0 (.glb & .gltf+.bin) mesh/PBR/node/collider parser
│   │   │   └── svg.rs             # Game-UI SVG subset -> 4x4 AA RGBA rasterizer
│   │   ├── font.rs                # Built-in 5x7 ASCII bitmap font atlas
│   │   ├── gfx.rs                 # OpenGL 3.3 Core PBR, shadows, post-FX, 3D decals, 2D & UI batcher
│   │   ├── audio.rs               # 48 kHz stereo mixer (ALSA on Linux, WinMM on Windows, null fallback)
│   │   ├── data.rs                # .cfg/.tbl/.lvl parser, Pool<T>, schema-tolerant Snapshot
│   │   ├── assets.rs              # Handle registry, file watcher, hot-reload & error fallbacks
│   │   ├── geom.rs                # Optional pure collision/raycast/heightmap/rigid-body queries
│   │   └── dev.rs                 # Local command bridge (Unix socket / Windows TCP), headless runner, scripts
├── demos/                         # Self-contained showcase games (neon_rush, mahjong_wars, cozy_golf)
└── scripts/                       # fast_restart.sh & build_windows_bundle.sh
```

### Non-Constraining Ownership Rule

The framework **never** hijacks control flow. A game is always a plain loop you can read in 15 lines:

```rust
fn main() {
    let mut app = App::from_args("Demo", 1280, 720); // handles --headless, --restore, --sock
    let mut assets = Assets::new("assets/", "data/");
    let mut state = GameState::load_or_init(&app, &mut assets);

    while app.begin_frame() {
        assets.hot_reload(&mut app);                  // Tier 1 & 2: assets, shaders, .cfg/.tbl/.lvl
        dev::poll_commands(&mut app, &mut state);     // screenshot, inspect, pause/step, snapshot

        if app.should_simulate() {
            game::update(&mut state, &app.input, &assets, app.dt);
        }
        game::render(&state, &assets, &mut app.draw);

        app.end_frame();
    }
}
```

---

## 3. Platform Layer & Zero-Dependency FFI (`platform/`)

Dullahan targets **Linux and Windows** directly via runtime-linked OS libraries—zero Cargo dependencies, zero `build.rs`, zero external SDKs required.

| Subsystem | Linux (`platform/linux.rs`) | Windows (`platform/windows.rs`) |
| :--- | :--- | :--- |
| Window & Input | `libX11.so.6` (keyboard, mouse, FPS cursor lock, resize, focus) | `user32.dll` (DPI-aware Win32 window, keyboard, mouse, FPS cursor lock, maximize/resize) |
| GPU Context | `libGL.so.1` (GLX `glXCreateContextAttribsARB` -> OpenGL 3.3 Core) | `gdi32.dll` + `opengl32.dll` (WGL `wglCreateContextAttribsARB` -> OpenGL 3.3 Core + VSync) |
| Offscreen / Headless | `libEGL.so.1` surfaceless/pbuffer + offscreen FBO (works with `DISPLAY` unset) | Hidden Win32 window + OpenGL 3.3 Core offscreen FBO |
| Audio Output | `libasound.so.2` (ALSA via `dlopen`, automatic silent null fallback) | `winmm.dll` (`waveOut` 4-buffer 48 kHz stereo ring, automatic silent null fallback) |
| Runtime Dev Bridge | Unix domain socket (`std::os::unix::net`, default `/tmp/dullahan.sock`) | Loopback TCP (`std::net`, default `127.0.0.1:42420`) |

### Portability Seam
All OS specifics live behind `platform/mod.rs`, `audio.rs`, and `dev.rs`. Game code, math, formats, renderer, physics, and data layers are 100% OS-agnostic; additional OS backends can be added later without changing any other module.

---

## 4. Standard Asset Format Contracts (`formats/`)

Every loader is pure Rust, panic-free on malformed input, and returns clear `file:line` / offset errors so hot-reload can fall back safely.

### 4.1 glTF 2.0 (`.glb` and `.gltf` + external `.bin`/textures)
- **Geometry:** Indexed triangle meshes (`POSITION`, `NORMAL`, `TEXCOORD_0`, optional `COLOR_0` and `TANGENT`). Missing normals are auto-computed; missing tangents are handled cleanly.
- **PBR Metallic-Roughness Materials:**
  - `baseColorFactor` + `baseColorTexture` (sRGB)
  - `metallicFactor`, `roughnessFactor` + `metallicRoughnessTexture` (linear, glTF spec: `G = Roughness`, `B = Metallic`)
  - `occlusionTexture` (`R = AO`, supports packed ORM textures automatically when pointing to the same image!)
  - `emissiveFactor` + `emissiveTexture` (sRGB)
  - `alphaMode`: `OPAQUE`, `MASK` (with `alphaCutoff`), `BLEND` (sorted transparent pass), and `doubleSided`
  - External texture deduplication: multiple `.glb` files referencing the same external `.png` texture share a single GPU texture handle and batch cleanly.
- **Named Node Hierarchy:** Every glTF node preserves its name, parent index, and rest TRS (`translation`, `rotation`, `scale`). Games can query `mesh.node_id("turret_yaw")` and pass per-draw local overrides to animate wheels, doors, weapons, or limbs without needing a skeletal animation engine on day one.
- **Background Loading & Unloading:** `load_mesh_async` returns a stable handle at once while 1-4 `std::thread` workers read and parse the file (CPU only). `App::begin_frame` uploads finished models on the main thread within `App::upload_budget_ms` (default 2 ms, at least one mesh per frame), so streaming worlds never stall on disk or parsing. Until then `assets.mesh(h)` is `None` and draws are skipped. `unload_mesh` frees GPU buffers and embedded textures and recycles the slot; a per-slot generation discards results for meshes unloaded mid-load.
- **Collider Convention:** Nodes named `col`, `col_*`, `*_col`, or `*-colonly` are extracted into `Mesh::colliders` (local AABBs / convex vertex clouds / triangle soups) and excluded from visual rendering.

### 4.2 PNG (`.png`)
- **Decoder:** Pure Rust zlib/DEFLATE + PNG filter reconstruction supporting:
  - 8-bit Grayscale, Grayscale+Alpha, RGB, RGBA, and Indexed Palette (`PLTE`/`tRNS`).
  - **16-bit Grayscale** (essential for lossless terrain heightmaps!).
- **Encoder:** Minimal valid RGBA PNG writer for `screenshot <path>` capture from the OpenGL framebuffer (flipped vertically to top-left origin).

### 4.3 WAV (`.wav`)
- Standard RIFF/WAVE parser supporting 16-bit signed PCM and 32-bit IEEE float, mono or stereo.
- Auto-resampled at load time to the mixer's native rate (48,000 Hz stereo `f32`) so runtime mixing stays trivial and fast.

### 4.4 SVG (`.svg`)
- Built-in pure-Rust Game-UI SVG rasterizer supporting `viewBox`, `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`, `<path>` (`M/L/H/V/C/S/Q/T/A/Z`), `<g>`, `transform`, fill/stroke/opacity, and linear gradients.
- Rasterized on load (or hot-reload) to an anti-aliased RGBA texture with transparent-edge alpha bleed at requested pixel size/scale—so both pre-baked `.png` UI assets and raw `.svg` icons work seamlessly.

---

## 5. Renderer (`gfx.rs`) & Audio (`audio.rs`)

### 5.1 Immediate-Mode OpenGL 3.3 Core Renderer (`gfx.rs`)
No scene graph, no retained entity handles inside the renderer. Each frame, the game pushes draw commands into `DrawList`, and `end_frame()` sorts and executes them in minimal passes:

1. **Shadow Pass (optional single directional shadow map, e.g. 2048×2048 PCF):** Casts crisp grounding shadows for 3D readability.
2. **Opaque PBR Pass:**
   - Standard metallic-roughness PBR + hemisphere ambient + 1 directional sun + up to 8 dynamic point lights + linear/exp distance fog.
   - Draw calls sorted by `(shader, material/texture, mesh)` so shared-palette models automatically minimize state switches.
   - **Instancing:** `draw.mesh_instances(mesh, instances, transform, tint)` draws a static GPU buffer of per-instance `Mat4`s (`app.create_instances`) with one `glDrawElementsInstanced` per primitive, in both the shadow and main pass (default shaders compiled a second time with `#define INSTANCED`; custom `pbr.vert` overrides need the same `#ifdef INSTANCED` block). For forests, crowds, debris.
   - **Frustum culling:** with `draw.frustum_culling` (default on), mesh draws and instance batches whose bounds are outside the camera frustum skip the main pass, and those outside the light frustum skip the shadow pass. Draws with node overrides are never culled. `FrameStats::culled` / `instances` report it.
   - Supports per-draw `Transform`, optional `node_overrides` slice, and color tint/emissive boost (great for hit-flash!).
3. **Terrain Pass (optional helper):**
   - Renders either a regular `.glb` terrain mesh *or* a heightmap + 4-channel RGBA splatmap + normal map terrain using the exact same PBR lighting model.
4. **Transparent (`BLEND`) Pass:** Back-to-front sorted by camera distance.
5. **3D Debug Pass:** Immediate-mode lines, wire boxes, spheres, capsules, colliders, ground grid, and RGB axis triads (`draw.debug_box(...)`, `draw.debug_line(...)`)—always toggleable live via the dev bridge.
6. **2D / UI Overlay Pass:**
   - Batched textured quads, solid rectangles, **9-slice scalable panels**, and a built-in crisp bitmap font (zero FreeType dependency) rendered in pixel coordinates.
   - Lightweight optional immediate-mode UI helpers (`ui.panel`, `ui.bar`, `ui.label`, `ui.button`).

All default shaders are embedded as clean GLSL 330 strings *and* can be overridden from `assets/shaders/*.vert|.frag` with instant hot-reload (compile errors print exact GLSL line numbers and keep the previous shader program active).

### 5.2 Audio Mixer (`audio.rs`)
- Dedicated background audio thread pulling frames from a lock-free command ringbuffer (zero locks in the audio callback, zero allocations per frame).
- Simple, expressive API:
  - `audio.play(sound, PlayParams { gain, pitch, pan, bus })`
  - `audio.play_variant(&[hit_1, hit_2, hit_3], params)` (effortless variation playback with slight pitch jitter)
  - `audio.play_loop(sound, bus) -> VoiceId` + `audio.set_voice(id, gain, pitch, pan)` + `audio.stop(id)`
  - `SFX` and `Music` master bus gains + mute toggle.

---

## 6. Git-Friendly Data Layer & 3-Tier Hot-Reload (`data.rs`, `assets.rs`)

### 6.1 Plaintext Data Formats
Designed for 1-line diffs, zero syntactic clutter, and effortless LLM generation:

**Config (`.cfg`) — key/value tuning & settings:**
```ini
# data/tuning.cfg
player.move_speed   = 7.5
player.jump_impulse = 11.2
player.gravity      = 28.0
camera.fov          = 60.0
camera.smooth       = 10.0
```

**Tables (`.tbl`) — typed entity/item definitions (one record per line):**
```text
# data/weapons.tbl
@cols id       damage cooldown speed  mesh              sfx
sword          25     0.35     0.0    weapons/sword.glb sfx/slash.wav
fireball       40     0.80     18.0   fx/orb.glb        sfx/fire.wav
crossbow       18     0.25     32.0   weapons/bolt.glb  sfx/twang.wav
```

**Levels / Scenes (`.lvl`) — declarative spatial placements:**
```text
# data/level_01.lvl
terrain assets/island.glb
spawn   player  0.0 1.0 0.0   yaw=0
prop    crate   4.0 0.0 -2.5  yaw=45 scale=1.2
enemy   goblin  9.0 0.0 -6.0  patrol=4.0
light   point   4.0 3.0 -2.5  color=#ffaa44 radius=8.0 intensity=2.0
```

### 6.2 Core Data Structures
- **`Pool<T>` + `Handle<T>` (index + generation):** Flat `Vec`-backed arena with O(1) insert, remove, lookup, and dense iteration. Prevents dangling references without `Rc`/`RefCell` or ECS overhead.
- **Schema-Tolerant `Snapshot`:** Plaintext line-oriented serializer (`section` + `key = value` + indexed pool records). On load:
  - Unknown fields (removed in a code edit) are silently ignored with a log note.
  - Missing fields (newly added in a code edit) fall back to their Rust default value.
  - This guarantees Tier 3 fast-restarts survive struct refactors cleanly.

### 6.3 The 3-Tier Live Iteration Workflow

| Tier | What Changed | Mechanism | Latency | Failure Mode Safety |
| :--- | :--- | :--- | :--- | :--- |
| **Tier 1** | `.glb`, `.gltf`, `.png`, `.svg`, `.wav`, `.glsl` | File watcher detects mtime change → reloads into existing handle slot | ~5–30 ms | Parse/compile error logs diagnostics and retains last valid GPU/audio resource (or bright magenta checkerboard placeholder on initial failure) |
| **Tier 2** | `.cfg`, `.tbl`, `.lvl` | File watcher detects mtime change → re-parses tables & bumps generation counter | < 1 ms | Syntax error logs `file:line`, retains previous valid table; recovers automatically on next save |
| **Tier 3** | `.rs` game logic | Dev runner triggers `snapshot save` → `cargo build` (~0.5s) → relaunch `--restore .dullahan_state.snap` | ~0.5–1.0 s | Compile error keeps current running process alive; schema-tolerant snapshot survives struct changes |

---

## 7. Closed-Loop Runtime Bridge (`dev.rs` & `dctl`)

Inspired by Artisan's closed-loop CLI workflow, every Dullahan app in dev mode binds a local control endpoint (Unix socket `/tmp/dullahan.sock` on Linux, loopback TCP `127.0.0.1:42420` on Windows, configurable via `--sock <path-or-addr>`).

An LLM (or human) interacts with the live game using a single tiny CLI binary (`dctl`, or plain `socat`/`nc -U`)—no MCP server, no browser tooling, no polling boilerplate:

```bash
dctl state                              # Structured plaintext dump: FPS, frame, draw calls, player/entity summary
dctl screenshot /tmp/view.png           # Captures next rendered frame to PNG and blocks until written
dctl pause                              # Freezes simulation while keeping rendering & inspection live
dctl step 10                            # Advances simulation by exactly 10 fixed-dt frames
dctl resume                             # Resumes real-time simulation
dctl overlay colliders                  # Toggles: none | colliders | wireframe | grid | bounds | hud
dctl set player.jump_impulse 14.0       # Overrides a live tuning value in memory (for instant feel tests)
dctl reload                             # Forces immediate hot-reload check of all assets & data
dctl snapshot save /tmp/dev.snap        # Saves current GameState snapshot
dctl snapshot load /tmp/dev.snap        # Restores GameState snapshot in-place
dctl input key W 0.5                    # Injects synthetic input for automated testing (hold W for 0.5s)
dctl cmd <game-specific-command>        # Dispatches to optional GameState::handle_dev_cmd(&mut self, args)
dctl quit                               # Clean shutdown
```

### Automated Headless Verification Mode
For CI or autonomous LLM bug-fixing without touching the desktop:

```bash
./target/debug/my_game \
  --headless \
  --size 960x540 \
  --seed 42 \
  --dt 0.0166667 \
  --script tests/jump_gap.inp \
  --snap /tmp/jump_result.png
```

Outputs a single-line structured summary to stdout (e.g., `frames=120 dt=0.0167 draws=14 pos=12.40,1.00,-3.20 nan=0 errors=0 snap=/tmp/jump_result.png`) plus the rendered PNG for immediate visual inspection.

---

## 8. Companion Generic LLM Skill (`skills/dullahan-framework/SKILL.md`)

Because the framework is intentionally vendored and editable, we will ship a concise, genre-neutral `SKILL.md` that teaches any coding agent how to work cleanly inside a Dullahan project:

- **Architecture rules:** Keep `GameState` flat; put tunable numbers in `data/*.cfg` or `data/*.tbl`, never buried as magic constants; keep `update()` deterministic and free of GL calls; keep `render()` read-only over `&GameState`.
- **Safe modification rules:** When adding a missing engine feature (e.g., fog, decals, camera shake, custom shader uniform), add the minimal flat primitive in `src/dullahan/` rather than introducing abstractions or external crates.
- **Closed-loop debugging workflow:** Whenever changing visuals, layout, physics, or feel:
  1. Edit data/asset/code.
  2. Trigger reload or fast-restart.
  3. Run `dctl state` and `dctl screenshot /tmp/qa.png` (or `--headless --script ...`).
  4. Visually inspect the PNG and verify zero warnings/NaNs before declaring the task done.

---

## 9. Phased Implementation Roadmap

Each phase produces a runnable, visually verifiable milestone:

| Phase | Scope | Exit Criterion |
| :--- | :--- | :--- |
| **Phase 0: Core Spine** | `platform/linux.rs` (X11 + GL 3.3 Core), `math.rs`, minimal `gfx.rs` (debug 3D lines/grid + camera), `formats/png.rs` (encoder), `dev.rs` (`screenshot` + `state` + `--headless`) | Opens a 60 FPS Linux window with orbiting 3D grid/cube; `dctl screenshot` captures a verified PNG; `cargo build` has 0 dependencies and builds in < 1s. |
| **Phase 1: Standard Assets & Hot-Reload** | `formats/png.rs` (full decoder incl. 16-bit), `formats/gltf.rs` (`.glb`/`.gltf` + PBR + nodes + `col` colliders), `formats/wav.rs`, `assets.rs` (handles, mtime watcher, safe fallbacks) | Loads external/Artisan `.glb`, `.png`, `.wav`; editing an asset file swaps it live in the running window without restarting; corrupt files fall back safely. |
| **Phase 2: PBR Renderer, UI & Audio** | Full `gfx.rs` (PBR ORM/emissive, shadow map, transparent pass, 2D sprite & 9-slice batcher, built-in font, hot-reloadable GLSL), `audio.rs` (ALSA mixer + null fallback) | Renders lit PBR scenes with shadows, animated glTF child nodes, crisp HUD, and spatialized/pitched WAV SFX & looping music. |
| **Phase 3: Data Layer, Snapshots & Geometry** | `data.rs` (`.cfg`/`.tbl`/`.lvl`, `Pool<T>`, schema-tolerant `Snapshot`), `geom.rs` (raycast, AABB/capsule collision, heightmap sampling), Tier 3 fast-restart script | Editing `.cfg`/`.tbl`/`.lvl` updates gameplay live; code edit triggers <1s snapshot-preserving restart; scripted headless playtest passes. |
| **Phase 4: SVG, Terrain & Companion Skill** | `formats/svg.rs` (UI SVG rasterizer), heightmap/splatmap terrain helper, `skills/dullahan-framework/SKILL.md`, and vertical-slice demo game | Full demo game built & iterated end-to-end using Artisan/external assets + closed-loop `dctl` workflow. |
