//! MAHJONG WARS — Deadpan Tactical First-Person Mahjong Solitaire Shooter.
//!
//! Flat, data-oriented architecture: one `GameState`, genuine solvable 3D Mahjong
//! Solitaire rules, FPS laser carbine viewmodel, SDF alien ghosts with squash & stretch,
//! SVG talisman shockwaves & tile decals, and Chinese pentatonic war music.

use dullahan::App;
use dullahan::assets::{MeshHandle, SoundHandle, TexHandle};
use dullahan::audio::{AudioBus, PlayParams, VoiceId};
use dullahan::data::{DataStore, Snapshot};
use dullahan::geom::{aabb_pushout, ray_sphere};
use dullahan::gfx::{DrawList, PostSettings};
use dullahan::math::{Aabb, Color, Quat, Ray, Rng, Transform, Vec2, Vec3, clamp, lerp};
use dullahan::platform::{InputState, Key, MouseButton};

const TILE_W: f32 = 2.18;
const TILE_H: f32 = 0.86;
const TILE_D: f32 = 2.78;
const NUM_SYMBOLS: usize = 8;

const SYMBOL_NAMES: [&str; NUM_SYMBOLS] = [
    "BAMBOO",
    "RED DRAGON",
    "LOTUS",
    "EAST WIND",
    "GOLD COIN",
    "LANTERN",
    "JADE BLADE",
    "PHOENIX",
];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Title,
    Playing,
    Victory,
    Defeat,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Playing => "playing",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
        }
    }
}

#[derive(Clone, Debug)]
struct Tile {
    /// Grid coordinates (half-step integers so upper layers can center cleanly!)
    gx: i32,
    gz: i32,
    layer: i32,
    symbol: usize,
    removed: bool,
    pulse: f32,
}

impl Tile {
    fn world_pos(&self) -> Vec3 {
        Vec3::new(
            self.gx as f32 * (TILE_W * 0.5),
            self.layer as f32 * TILE_H,
            self.gz as f32 * (TILE_D * 0.5),
        )
    }

    fn aabb(&self) -> Aabb {
        let p = self.world_pos();
        Aabb::new(
            Vec3::new(p.x - TILE_W * 0.48, p.y, p.z - TILE_D * 0.48),
            Vec3::new(p.x + TILE_W * 0.48, p.y + TILE_H, p.z + TILE_D * 0.48),
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum GhostKind {
    Scout,
    Warden,
}

struct Ghost {
    kind: GhostKind,
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    hp: f32,
    max_hp: f32,
    bob_phase: f32,
    hit_flash: f32,
    squash_timer: f32,
    shoot_cd: f32,
}

struct EctoOrb {
    pos: Vec3,
    vel: Vec3,
    life: f32,
    color: Color,
}

struct LaserBeam {
    start: Vec3,
    end: Vec3,
    life: f32,
    max_life: f32,
    color: Color,
}

struct FxBillboard {
    tex: TexHandle,
    pos: Vec3,
    vel: Vec3,
    size_start: f32,
    size_end: f32,
    life: f32,
    max_life: f32,
    color: Color,
}

struct SoundEvent {
    sound: SoundHandle,
    params: PlayParams,
}

pub struct GameAssets {
    tile: MeshHandle,
    rifle: MeshHandle,
    ghost_scout: MeshHandle,
    ghost_warden: MeshHandle,
    pillar: MeshHandle,
    gate_arch: MeshHandle,
    tile_icons: [TexHandle; NUM_SYMBOLS],
    reticle_tex: TexHandle,
    muzzle_tex: TexHandle,
    seal_tex: TexHandle,
    ecto_ring_tex: TexHandle,
    music_temple: SoundHandle,
    sfx_laser: SoundHandle,
    sfx_select: SoundHandle,
    sfx_match: SoundHandle,
    sfx_err: SoundHandle,
    sfx_ghost_hit: SoundHandle,
    sfx_ghost_pop: SoundHandle,
    sfx_player_hit: SoundHandle,
}

impl GameAssets {
    pub fn load(app: &mut App) -> Self {
        app.data.load_config("mahjong.cfg");
        let tile_icons = [
            app.load_svg("ui/tile_0.svg", 128, 128),
            app.load_svg("ui/tile_1.svg", 128, 128),
            app.load_svg("ui/tile_2.svg", 128, 128),
            app.load_svg("ui/tile_3.svg", 128, 128),
            app.load_svg("ui/tile_4.svg", 128, 128),
            app.load_svg("ui/tile_5.svg", 128, 128),
            app.load_svg("ui/tile_6.svg", 128, 128),
            app.load_svg("ui/tile_7.svg", 128, 128),
        ];
        Self {
            tile: app.load_mesh("models/tile.glb"),
            rifle: app.load_mesh("models/rifle.glb"),
            ghost_scout: app.load_mesh("models/ghost_scout.glb"),
            ghost_warden: app.load_mesh("models/ghost_warden.glb"),
            pillar: app.load_mesh("models/pillar.glb"),
            gate_arch: app.load_mesh("models/gate_arch.glb"),
            tile_icons,
            reticle_tex: app.load_svg("ui/reticle.svg", 64, 64),
            muzzle_tex: app.load_svg("ui/muzzle.svg", 96, 96),
            seal_tex: app.load_svg("ui/seal.svg", 128, 128),
            ecto_ring_tex: app.load_svg("ui/ecto_ring.svg", 96, 96),
            music_temple: app.load_sound("sfx/music_temple.wav"),
            sfx_laser: app.load_sound("sfx/laser.wav"),
            sfx_select: app.load_sound("sfx/tile_select.wav"),
            sfx_match: app.load_sound("sfx/tile_match.wav"),
            sfx_err: app.load_sound("sfx/tile_err.wav"),
            sfx_ghost_hit: app.load_sound("sfx/ghost_hit.wav"),
            sfx_ghost_pop: app.load_sound("sfx/ghost_pop.wav"),
            sfx_player_hit: app.load_sound("sfx/player_hit.wav"),
        }
    }
}

pub struct GameState {
    pub phase: Phase,
    pub paused: bool,
    pub quit_requested: bool,
    pub assets: GameAssets,
    music_voice: Option<VoiceId>,

    // Live Settings (adjustable in Pause Menu)
    pub muted: bool,
    pub master_volume: f32,
    pub mouse_sens_mult: f32,
    pub fov_deg: f32,
    pub invert_y: bool,

    // FPS Player & Parkour State
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub grounded: bool,
    pub sliding: bool,
    slide_timer: f32,
    slide_grace: f32,
    slide_cd: f32,
    slide_spark_cd: f32,
    slide_dir: Vec3,
    coyote_timer: f32,
    jump_buffer: f32,
    eye_height_cur: f32,
    cam_roll: f32,
    fov_kick: f32,
    vault_flash: f32,
    pub hp: f32,
    pub max_hp: f32,
    walk_bob: f32,

    // Tactical Laser Rifle State
    fire_cd: f32,
    recoil: f32,
    muzzle_timer: f32,
    hitmarker_timer: f32,

    // Mahjong Solitaire State
    tiles: Vec<Tile>,
    selected_tile: Option<usize>,
    aimed_tile: Option<(usize, bool, f32)>, // (index, is_free, dist)
    pub pairs_cleared: u32,
    pub total_pairs: u32,
    pub ghosts_purged: u32,
    pub score: u32,
    pub combo: u32,
    combo_timer: f32,

    // Entities & Juice
    ghosts: Vec<Ghost>,
    orbs: Vec<EctoOrb>,
    beams: Vec<LaserBeam>,
    fx: Vec<FxBillboard>,
    spawn_timer: f32,
    shake_trauma: f32,
    flash_timer: f32,
    flash_color: Color,
    radio_msg: String,
    radio_timer: f32,
    rng: Rng,
    queued_sounds: Vec<SoundEvent>,
}

impl GameState {
    pub fn new(app: &mut App) -> Self {
        let assets = GameAssets::load(app);
        let music_voice = app.play_loop(assets.music_temple, 0.36, AudioBus::Music);

        let mut s = Self {
            phase: Phase::Title,
            paused: false,
            quit_requested: false,
            assets,
            music_voice,
            muted: false,
            master_volume: 0.85,
            mouse_sens_mult: 0.75,
            fov_deg: 72.0,
            invert_y: false,
            pos: Vec3::new(0.0, 0.0, 14.5),
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: -0.14,
            grounded: true,
            sliding: false,
            slide_timer: 0.0,
            slide_grace: 0.0,
            slide_cd: 0.0,
            slide_spark_cd: 0.0,
            slide_dir: Vec3::new(0.0, 0.0, -1.0),
            coyote_timer: 0.0,
            jump_buffer: 0.0,
            eye_height_cur: 1.75,
            cam_roll: 0.0,
            fov_kick: 0.0,
            vault_flash: 0.0,
            hp: 100.0,
            max_hp: 100.0,
            walk_bob: 0.0,
            fire_cd: 0.0,
            recoil: 0.0,
            muzzle_timer: 0.0,
            hitmarker_timer: 0.0,
            tiles: Vec::new(),
            selected_tile: None,
            aimed_tile: None,
            pairs_cleared: 0,
            total_pairs: 16,
            ghosts_purged: 0,
            score: 0,
            combo: 0,
            combo_timer: 0.0,
            ghosts: Vec::new(),
            orbs: Vec::new(),
            beams: Vec::new(),
            fx: Vec::new(),
            spawn_timer: 2.0,
            shake_trauma: 0.0,
            flash_timer: 0.0,
            flash_color: Color::WHITE,
            radio_msg: "HQ: CLEAR ALL 16 PAIRS. IGNORE ECTOPLASMIC TAUNTING.".into(),
            radio_timer: 99.0,
            rng: Rng::new(0x8888_4A40_2026),
            queued_sounds: Vec::new(),
        };
        s.generate_solvable_board();
        s.seed_opening_ghosts();
        if let Some(snap) = app.startup_snapshot() {
            s.load_snapshot(&snap);
        }
        s
    }

    pub fn start_run(&mut self) {
        self.phase = Phase::Playing;
        self.paused = false;
        self.pos = Vec3::new(0.0, 0.0, 12.2);
        self.vel = Vec3::ZERO;
        self.yaw = 0.0;
        self.pitch = -0.12;
        self.sliding = false;
        self.slide_timer = 0.0;
        self.slide_grace = 0.0;
        self.slide_cd = 0.0;
        self.coyote_timer = 0.0;
        self.jump_buffer = 0.0;
        self.eye_height_cur = 1.75;
        self.cam_roll = 0.0;
        self.fov_kick = 0.0;
        self.vault_flash = 0.0;
        self.hp = self.max_hp;
        self.fire_cd = 0.0;
        self.recoil = 0.0;
        self.selected_tile = None;
        self.pairs_cleared = 0;
        self.ghosts_purged = 0;
        self.score = 0;
        self.combo = 0;
        self.combo_timer = 0.0;
        self.orbs.clear();
        self.beams.clear();
        self.fx.clear();
        self.shake_trauma = 0.0;
        self.flash_timer = 0.0;
        self.spawn_timer = 3.2;
        self.generate_solvable_board();
        self.seed_opening_ghosts();
        self.set_radio("HQ: TOUCH FREE PAIRS [E/RMB] // SHOOT GHOSTS [LMB]", 5.5);
    }

    /// Generates a 32-tile (16-pair) 3-layer pyramid guaranteed 100% solvable by reverse-peeling.
    fn generate_solvable_board(&mut self) {
        let mut coords: Vec<(i32, i32, i32)> = Vec::new();
        // Layer 0: 6 x 4 base (24 tiles, grid spacing = 2 half-steps)
        for gz in [-3, -1, 1, 3] {
            for gx in [-5, -3, -1, 1, 3, 5] {
                coords.push((gx, gz, 0));
            }
        }
        // Layer 1: 3 x 2 middle tier (6 tiles)
        for gz in [-1, 1] {
            for gx in [-2, 0, 2] {
                coords.push((gx, gz, 1));
            }
        }
        // Layer 2: 2 x 1 summit crown (2 tiles)
        coords.push((-1, 0, 2));
        coords.push((1, 0, 2));

        let mut tiles: Vec<Tile> = coords
            .into_iter()
            .map(|(gx, gz, layer)| Tile { gx, gz, layer, symbol: 0, removed: false, pulse: 0.0 })
            .collect();

        // Reverse-peel pairs of legally free slots so a valid solution ALWAYS exists.
        let mut sym_bag: Vec<usize> = (0..16).map(|i| i % NUM_SYMBOLS).collect();
        for i in (1..sym_bag.len()).rev() {
            let j = self.rng.usize(i + 1);
            sym_bag.swap(i, j);
        }

        let mut peeled = vec![false; tiles.len()];
        let mut assigned = vec![0usize; tiles.len()];
        for &sym in &sym_bag {
            let free_indices: Vec<usize> = (0..tiles.len())
                .filter(|&i| !peeled[i] && is_slot_free(&tiles, &peeled, i))
                .collect();
            if free_indices.len() >= 2 {
                let a_i = self.rng.usize(free_indices.len());
                let a = free_indices[a_i];
                let mut b_i = self.rng.usize(free_indices.len() - 1);
                if b_i >= a_i {
                    b_i += 1;
                }
                let b = free_indices[b_i];
                assigned[a] = sym;
                assigned[b] = sym;
                peeled[a] = true;
                peeled[b] = true;
            }
        }
        for i in 0..tiles.len() {
            tiles[i].symbol = assigned[i];
        }

        self.total_pairs = (tiles.len() / 2) as u32;
        self.tiles = tiles;
    }

    fn seed_opening_ghosts(&mut self) {
        self.ghosts.clear();
        let initial = [
            (GhostKind::Scout, Vec3::new(-5.8, 2.3, 1.8)),
            (GhostKind::Scout, Vec3::new(5.8, 2.5, -1.2)),
            (GhostKind::Warden, Vec3::new(0.4, 3.6, -5.2)),
        ];
        for (idx, (kind, pos)) in initial.into_iter().enumerate() {
            let max_hp = if kind == GhostKind::Warden { 130.0 } else { 65.0 };
            self.ghosts.push(Ghost {
                kind,
                pos,
                vel: Vec3::ZERO,
                yaw: 0.0,
                hp: max_hp,
                max_hp,
                bob_phase: idx as f32 * 2.1,
                hit_flash: 0.0,
                squash_timer: 0.0,
                shoot_cd: 2.0 + idx as f32 * 1.1,
            });
        }
    }

    fn spawn_ghost(&mut self) {
        let angle = self.rng.range(0.0, std::f32::consts::TAU);
        let dist = self.rng.range(13.0, 17.5);
        let kind = if self.rng.f32() < 0.30 { GhostKind::Warden } else { GhostKind::Scout };
        let max_hp = if kind == GhostKind::Warden { 130.0 } else { 65.0 };
        let pos = Vec3::new(angle.sin() * dist, self.rng.range(2.2, 4.0), angle.cos() * dist);
        self.ghosts.push(Ghost {
            kind,
            pos,
            vel: Vec3::ZERO,
            yaw: 0.0,
            hp: max_hp,
            max_hp,
            bob_phase: self.rng.range(0.0, 6.28),
            hit_flash: 0.0,
            squash_timer: 0.0,
            shoot_cd: self.rng.range(2.2, 4.2),
        });
        if kind == GhostKind::Warden {
            self.set_radio("HQ: CLASS-4 ALIEN APPARITION ENTERING COURTYARD PERIMETER.", 3.0);
        }
    }

    pub fn is_tile_free(&self, idx: usize) -> bool {
        if self.tiles[idx].removed {
            return false;
        }
        let removed_flags: Vec<bool> = self.tiles.iter().map(|t| t.removed).collect();
        is_slot_free(&self.tiles, &removed_flags, idx)
    }

    fn eye_pos(&self, _data: &DataStore) -> Vec3 {
        let bob_amp = if self.sliding { 0.0 } else { 0.055 };
        let bob_y = (self.walk_bob * 9.0).sin() * bob_amp;
        Vec3::new(self.pos.x, self.pos.y + self.eye_height_cur + bob_y, self.pos.z)
    }

    pub fn trigger_slide(&mut self, data: &DataStore) {
        let slide_spd = data.config("mahjong.cfg").map(|c| c.f32("player.slide_speed", 16.8)).unwrap_or(16.8);
        let fwd_xz = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos()).norm();
        let horiz = Vec3::new(self.vel.x, 0.0, self.vel.z);
        let dir = if horiz.len_sq() > 1.0 { horiz.norm() } else { fwd_xz };
        self.sliding = true;
        self.slide_timer = 0.65;
        self.slide_grace = 0.18;
        self.slide_cd = 0.35;
        self.slide_dir = dir;
        let boost = horiz.len().max(slide_spd);
        self.vel.x = dir.x * boost;
        self.vel.z = dir.z * boost;
        self.fov_kick = (self.fov_kick + 9.5).min(12.0);
        self.queue_sfx(self.assets.sfx_select, 0.38, 0.62, 0.0);
    }

    fn look_basis(&self) -> (Vec3, Vec3, Vec3) {
        let cp = self.pitch.cos();
        let fwd = Vec3::new(-self.yaw.sin() * cp, self.pitch.sin(), -self.yaw.cos() * cp).norm();
        let right = fwd.cross(Vec3::Y).norm();
        let up = right.cross(fwd).norm();
        (fwd, right, up)
    }

    pub fn fire_laser(&mut self, data: &DataStore) {
        let cfg = data.config("mahjong.cfg");
        let rate = cfg.map(|c| c.f32("rifle.fire_rate", 0.14)).unwrap_or(0.14);
        let dmg = cfg.map(|c| c.f32("rifle.damage", 34.0)).unwrap_or(34.0);
        let range = cfg.map(|c| c.f32("rifle.range", 55.0)).unwrap_or(55.0);
        if self.fire_cd > 0.0 {
            return;
        }
        self.fire_cd = rate;
        self.recoil = (self.recoil + 0.11).min(0.22);
        self.muzzle_timer = 0.075;
        self.shake_trauma = (self.shake_trauma + 0.12).min(1.0);
        let pitch = 0.94 + self.rng.f32() * 0.14;
        self.queue_sfx(self.assets.sfx_laser, 0.58, pitch, 0.08);

        let eye = self.eye_pos(data);
        let (fwd, right, up) = self.look_basis();
        let muzzle_pos = eye + fwd * 1.15 + right * 0.28 - up * 0.22;
        let ray = Ray::new(eye, fwd);

        let mut best_t = range;
        let mut hit_ghost: Option<usize> = None;

        // Check tile occlusion first
        for t in &self.tiles {
            if t.removed {
                continue;
            }
            if let Some(d) = ray.hit_aabb(t.aabb())
                && d < best_t
            {
                best_t = d;
            }
        }
        // Check alien ghosts
        for (i, g) in self.ghosts.iter().enumerate() {
            let rad = if g.kind == GhostKind::Warden { 1.15 } else { 0.92 };
            let center = g.pos + Vec3::new(0.0, 1.1, 0.0);
            if let Some(d) = ray_sphere(ray, center, rad)
                && d < best_t
            {
                best_t = d;
                hit_ghost = Some(i);
            }
        }

        let hit_pos = ray.at(best_t);
        self.beams.push(LaserBeam {
            start: muzzle_pos,
            end: hit_pos,
            life: 0.11,
            max_life: 0.11,
            color: Color::from_hex(0x44FAFF),
        });
        // Luminous plasma pulses along the beam path so laser fire pops in bloom
        for k in 1..=4 {
            let frac = k as f32 / 5.0;
            self.fx.push(FxBillboard {
                tex: self.assets.muzzle_tex,
                pos: muzzle_pos.lerp(hit_pos, frac),
                vel: Vec3::ZERO,
                size_start: 0.34,
                size_end: 0.06,
                life: 0.09,
                max_life: 0.09,
                color: Color::from_hex(0x66FFFF),
            });
        }
        self.fx.push(FxBillboard {
            tex: self.assets.muzzle_tex,
            pos: hit_pos,
            vel: Vec3::ZERO,
            size_start: 0.65,
            size_end: 0.14,
            life: 0.14,
            max_life: 0.14,
            color: Color::from_hex(0x44FAFF),
        });

        if let Some(gi) = hit_ghost {
            self.hitmarker_timer = 0.16;
            let mut died_pos: Option<(Vec3, GhostKind)> = None;
            if let Some(g) = self.ghosts.get_mut(gi) {
                g.hp -= dmg;
                g.hit_flash = 0.14;
                g.squash_timer = 0.28;
                g.vel += fwd * 7.5;
                if g.hp <= 0.0 {
                    died_pos = Some((g.pos + Vec3::new(0.0, 1.1, 0.0), g.kind));
                }
            }
            if let Some((pos, kind)) = died_pos {
                self.ghosts.remove(gi);
                self.ghosts_purged += 1;
                let pts = if kind == GhostKind::Warden { 300 } else { 150 };
                self.score += pts;
                self.shake_trauma = (self.shake_trauma + 0.25).min(1.0);
                let p_pop = 0.95 + self.rng.f32() * 0.15;
                self.queue_sfx(self.assets.sfx_ghost_pop, 0.72, p_pop, 0.0);
                self.spawn_ecto_pop(pos, kind == GhostKind::Warden);
                if self.ghosts_purged % 3 == 0 {
                    self.set_radio("HQ: GHOST EVAPORATED. FILE FORM 104-B LATER.", 3.2);
                }
            } else {
                let p_hit = 0.9 + self.rng.f32() * 0.25;
                self.queue_sfx(self.assets.sfx_ghost_hit, 0.55, p_hit, 0.0);
            }
        }
    }

    /// Touches the currently aimed free Mahjong tile (or nearest free tile within touch range).
    pub fn touch_aimed_tile(&mut self, data: &DataStore) {
        let range = data.config("mahjong.cfg").map(|c| c.f32("player.touch_range", 6.8)).unwrap_or(6.8);
        let Some((idx, is_free, dist)) = self.aimed_tile else {
            return;
        };
        if dist > range {
            self.set_radio("HQ: MOVE CLOSER TO TOUCH THAT TILE.", 2.0);
            return;
        }
        if !is_free {
            self.queue_sfx(self.assets.sfx_err, 0.50, 0.9, 0.0);
            self.set_radio("PROTOCOL BLOCKED: CLEAR FREE EDGES FIRST.", 2.4);
            return;
        }

        if let Some(prev_idx) = self.selected_tile {
            if prev_idx == idx {
                self.selected_tile = None;
                self.queue_sfx(self.assets.sfx_select, 0.45, 0.85, 0.0);
                return;
            }
            if !self.tiles[prev_idx].removed && self.tiles[prev_idx].symbol == self.tiles[idx].symbol {
                self.execute_tile_match(prev_idx, idx);
                return;
            } else {
                // Deadpan mismatch notice, smoothly switch lock to the newly touched free tile
                self.combo = 0;
                self.selected_tile = Some(idx);
                self.tiles[idx].pulse = 1.0;
                self.queue_sfx(self.assets.sfx_err, 0.55, 1.0, 0.0);
                let name = SYMBOL_NAMES[self.tiles[idx].symbol];
                self.set_radio(&format!("MISMATCH LOGGED. RE-LOCKED ONTO FREE {name}."), 2.6);
                return;
            }
        }

        self.selected_tile = Some(idx);
        self.tiles[idx].pulse = 1.0;
        let name = SYMBOL_NAMES[self.tiles[idx].symbol];
        let p = self.tiles[idx].world_pos() + Vec3::new(0.0, 1.2, 0.0);
        self.fx.push(FxBillboard {
            tex: self.assets.seal_tex,
            pos: p,
            vel: Vec3::new(0.0, 0.8, 0.0),
            size_start: 1.0,
            size_end: 2.2,
            life: 0.35,
            max_life: 0.35,
            color: Color::from_hex(0x38F5A8),
        });
        self.queue_sfx(self.assets.sfx_select, 0.65, 1.0, 0.0);
        self.set_radio(&format!("TILE LOCKED: {name}. TOUCH MATCHING FREE {name}."), 3.0);
    }

    pub fn execute_tile_match(&mut self, a: usize, b: usize) {
        let sym_name = SYMBOL_NAMES[self.tiles[a].symbol];
        let pa = self.tiles[a].world_pos() + Vec3::new(0.0, 1.1, 0.0);
        let pb = self.tiles[b].world_pos() + Vec3::new(0.0, 1.1, 0.0);
        self.tiles[a].removed = true;
        self.tiles[b].removed = true;
        self.selected_tile = None;

        self.pairs_cleared += 1;
        self.combo += 1;
        self.combo_timer = 10.0;
        let mult = self.combo.min(5);
        self.score += 500 * mult;
        self.hp = (self.hp + 12.0).min(self.max_hp);
        self.shake_trauma = (self.shake_trauma + 0.38).min(1.0);
        self.flash(Color::from_hex(0x38F5A8), 0.22);

        let pitch = (1.0 + (self.combo.saturating_sub(1) as f32) * 0.05).min(1.35);
        self.queue_sfx(self.assets.sfx_match, 0.85, pitch, 0.0);

        // Connecting jade energy arc + expanding SVG Chinese Talisman Shockwave Seals!
        self.beams.push(LaserBeam {
            start: pa,
            end: pb,
            life: 0.45,
            max_life: 0.45,
            color: Color::from_hex(0x38F5A8),
        });
        for p in [pa, pb] {
            self.fx.push(FxBillboard {
                tex: self.assets.seal_tex,
                pos: p + Vec3::new(0.0, 0.25, 0.0),
                vel: Vec3::new(0.0, 1.15, 0.0),
                size_start: 1.6,
                size_end: 5.8,
                life: 0.85,
                max_life: 0.85,
                color: Color::from_hex(0xFFE466),
            });
            for _ in 0..8 {
                self.fx.push(FxBillboard {
                    tex: self.assets.muzzle_tex,
                    pos: p,
                    vel: Vec3::new(self.rng.range(-5.0, 5.0), self.rng.range(2.5, 7.5), self.rng.range(-5.0, 5.0)),
                    size_start: 0.55,
                    size_end: 0.08,
                    life: 0.48,
                    max_life: 0.48,
                    color: Color::from_hex(0x38F5A8),
                });
            }
        }

        // Harmony Shockwave blasts & damages nearby alien ghosts!
        let mid = (pa + pb) * 0.5;
        for g in &mut self.ghosts {
            let delta = g.pos - mid;
            let dist = delta.len();
            if dist < 11.5 {
                g.hp -= 45.0;
                g.hit_flash = 0.22;
                g.squash_timer = 0.35;
                g.vel += delta.norm() * 11.0;
            }
        }
        let popped: Vec<(Vec3, GhostKind)> = self
            .ghosts
            .iter()
            .filter(|g| g.hp <= 0.0)
            .map(|g| (g.pos + Vec3::new(0.0, 1.0, 0.0), g.kind))
            .collect();
        self.ghosts.retain(|g| g.hp > 0.0);
        for (p, k) in popped {
            self.ghosts_purged += 1;
            self.score += 200;
            self.spawn_ecto_pop(p, k == GhostKind::Warden);
        }

        if self.pairs_cleared >= self.total_pairs {
            self.phase = Phase::Victory;
            self.set_radio("HQ: ALL 16 PAIRS PURGED. TEMPLE HARMONY RESTORED.", 99.0);
        } else {
            let remaining = self.total_pairs - self.pairs_cleared;
            self.set_radio(
                &format!("{sym_name} MATCHED (+{}) // {remaining} PAIRS LEFT", 500 * mult),
                3.6,
            );
        }
    }

    fn spawn_ecto_pop(&mut self, pos: Vec3, is_warden: bool) {
        let col = if is_warden { Color::from_hex(0xFF48B4) } else { Color::from_hex(0x48FF84) };
        self.fx.push(FxBillboard {
            tex: self.assets.ecto_ring_tex,
            pos,
            vel: Vec3::new(0.0, 0.6, 0.0),
            size_start: 1.0,
            size_end: if is_warden { 4.2 } else { 3.1 },
            life: 0.45,
            max_life: 0.45,
            color: col,
        });
        for _ in 0..10 {
            self.fx.push(FxBillboard {
                tex: self.assets.muzzle_tex,
                pos,
                vel: Vec3::new(self.rng.range(-6.0, 6.0), self.rng.range(1.0, 6.0), self.rng.range(-6.0, 6.0)),
                size_start: 0.50,
                size_end: 0.08,
                life: 0.40,
                max_life: 0.40,
                color: col,
            });
        }
    }

    /// Demo helper for `dctl cmd match_demo` — immediately matches the first valid free pair!
    pub fn trigger_demo_match(&mut self) -> Result<String, String> {
        let free: Vec<usize> = (0..self.tiles.len()).filter(|&i| self.is_tile_free(i)).collect();
        for i in 0..free.len() {
            for j in (i + 1)..free.len() {
                if self.tiles[free[i]].symbol == self.tiles[free[j]].symbol {
                    let sym = SYMBOL_NAMES[self.tiles[free[i]].symbol];
                    self.execute_tile_match(free[i], free[j]);
                    return Ok(format!("matched {sym}"));
                }
            }
        }
        Err("no free matching pair".into())
    }

    fn set_radio(&mut self, msg: &str, dur: f32) {
        self.radio_msg = msg.to_string();
        self.radio_timer = dur;
    }

    fn flash(&mut self, color: Color, dur: f32) {
        self.flash_color = color;
        self.flash_timer = dur;
    }

    fn queue_sfx(&mut self, sound: SoundHandle, gain: f32, pitch: f32, pan: f32) {
        self.queued_sounds.push(SoundEvent {
            sound,
            params: PlayParams { gain, pitch, pan, bus: AudioBus::Sfx, looping: false },
        });
    }

    pub fn flush_audio(&mut self, app: &mut App) {
        for ev in self.queued_sounds.drain(..) {
            app.play_sound(ev.sound, ev.params);
        }
        if let Some(vid) = self.music_voice {
            let g = if self.paused {
                0.14
            } else {
                match self.phase {
                    Phase::Playing => 0.36,
                    Phase::Title => 0.24,
                    Phase::Victory | Phase::Defeat => 0.16,
                }
            };
            app.audio.set_voice(vid, g, 1.0, 0.0);
        }
        app.audio.set_bus_gain(AudioBus::Sfx, self.master_volume);
        app.audio.set_bus_gain(AudioBus::Music, self.master_volume * 0.85);
        app.audio.set_muted(self.muted);
    }

    pub fn update(&mut self, input: &InputState, data: &DataStore, dt: f32) {
        if input.key_pressed(Key::M) {
            self.muted = !self.muted;
        }
        if self.paused {
            return;
        }

        let cfg = data.config("mahjong.cfg");
        let move_spd = cfg.map(|c| c.f32("player.move_speed", 9.6)).unwrap_or(9.6);
        let jump_vel = cfg.map(|c| c.f32("player.jump_vel", 10.8)).unwrap_or(10.8);
        let gravity = cfg.map(|c| c.f32("player.gravity", 28.0)).unwrap_or(28.0);
        let stand_eye = cfg.map(|c| c.f32("player.eye_height", 1.75)).unwrap_or(1.75);
        let slide_spd = cfg.map(|c| c.f32("player.slide_speed", 16.8)).unwrap_or(16.8);
        let slide_jump_boost = cfg.map(|c| c.f32("player.slide_jump_boost", 1.24)).unwrap_or(1.24);
        let slide_fric = cfg.map(|c| c.f32("player.slide_friction", 1.65)).unwrap_or(1.65);
        let base_sens = cfg.map(|c| c.f32("player.mouse_sens", 0.0018)).unwrap_or(0.0018);
        let mouse_sens = base_sens * self.mouse_sens_mult;
        let spawn_rate = cfg.map(|c| c.f32("ghost.spawn_rate", 4.6)).unwrap_or(4.6);
        let max_ghosts = cfg.map(|c| c.i32("ghost.max_active", 7)).unwrap_or(7) as usize;
        let scout_spd = cfg.map(|c| c.f32("ghost.scout_speed", 4.4)).unwrap_or(4.4);
        let warden_spd = cfg.map(|c| c.f32("ghost.warden_speed", 3.1)).unwrap_or(3.1);

        // Decay timers & FX
        self.fire_cd = (self.fire_cd - dt).max(0.0);
        self.recoil = lerp(self.recoil, 0.0, (14.0 * dt).min(1.0));
        self.muzzle_timer = (self.muzzle_timer - dt).max(0.0);
        self.hitmarker_timer = (self.hitmarker_timer - dt).max(0.0);
        self.shake_trauma = (self.shake_trauma - dt * 1.8).max(0.0);
        self.flash_timer = (self.flash_timer - dt).max(0.0);
        self.slide_cd = (self.slide_cd - dt).max(0.0);
        self.slide_grace = (self.slide_grace - dt).max(0.0);
        self.slide_spark_cd = (self.slide_spark_cd - dt).max(0.0);
        self.vault_flash = (self.vault_flash - dt).max(0.0);
        self.coyote_timer = (self.coyote_timer - dt).max(0.0);
        self.jump_buffer = (self.jump_buffer - dt).max(0.0);
        if self.radio_timer > 0.0 {
            self.radio_timer -= dt;
        }
        if self.combo_timer > 0.0 {
            self.combo_timer -= dt;
            if self.combo_timer <= 0.0 {
                self.combo = 0;
            }
        }
        for t in &mut self.tiles {
            t.pulse = (t.pulse - dt * 2.5).max(0.0);
        }
        for b in &mut self.beams {
            b.life -= dt;
        }
        self.beams.retain(|b| b.life > 0.0);
        for f in &mut self.fx {
            f.life -= dt;
            f.pos += f.vel * dt;
        }
        self.fx.retain(|f| f.life > 0.0);

        match self.phase {
            Phase::Title => {
                // Slow majestic orbital preview around the temple Mahjong pyramid (higher vantage so painted faces show!)
                self.yaw += 0.16 * dt;
                self.pos = Vec3::new(self.yaw.sin() * 14.2, 4.6, self.yaw.cos() * 14.2);
                self.eye_height_cur = stand_eye;
                self.pitch = -0.44;
                for g in &mut self.ghosts {
                    g.bob_phase += dt * 3.0;
                }
                if input.key_pressed(Key::Space) || input.key_pressed(Key::Enter) {
                    self.start_run();
                }
                return;
            }
            Phase::Victory | Phase::Defeat => {
                if input.key_pressed(Key::Space) || input.key_pressed(Key::R) {
                    self.start_run();
                }
                return;
            }
            Phase::Playing => {}
        }

        // --- FPS Look (fixed 1:1 mouse delta + Invert Y + arrow keys fallback) ---
        let y_sign = if self.invert_y { 1.0 } else { -1.0 };
        self.yaw -= input.mouse_delta.x * mouse_sens;
        self.pitch = clamp(self.pitch + input.mouse_delta.y * mouse_sens * y_sign, -1.32, 1.32);
        if input.key_down(Key::Left) {
            self.yaw += 2.2 * dt;
        }
        if input.key_down(Key::Right) {
            self.yaw -= 2.2 * dt;
        }
        if input.key_down(Key::Up) {
            self.pitch = clamp(self.pitch + 1.5 * dt, -1.32, 1.32);
        }
        if input.key_down(Key::Down) {
            self.pitch = clamp(self.pitch - 1.5 * dt, -1.32, 1.32);
        }

        // --- Parkour Movement: Sprint-Slide, Slide-Jump Vault & Walkable Tile Mantle ---
        let fwd_xz = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos()).norm();
        let right_xz = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin()).norm();
        let mut wish = Vec3::ZERO;
        let mut strafe_in = 0.0f32;
        if input.key_down(Key::W) { wish += fwd_xz; }
        if input.key_down(Key::S) { wish -= fwd_xz; }
        if input.key_down(Key::D) { wish += right_xz; strafe_in += 1.0; }
        if input.key_down(Key::A) { wish -= right_xz; strafe_in -= 1.0; }
        if wish.len_sq() > 1e-4 {
            wish = wish.norm();
            if self.grounded && !self.sliding {
                self.walk_bob += dt;
            }
        }

        let slide_held = input.key_down(Key::LShift)
            || input.key_down(Key::RShift)
            || input.key_down(Key::C)
            || input.key_down(Key::LCtrl);
        let slide_pressed = input.key_pressed(Key::LShift)
            || input.key_pressed(Key::RShift)
            || input.key_pressed(Key::C)
            || input.key_pressed(Key::LCtrl);

        if input.key_pressed(Key::Space) {
            self.jump_buffer = 0.14;
        }
        if self.grounded {
            self.coyote_timer = 0.13;
        }

        // Start a high-speed parkour slide when pressing Shift/C/Ctrl while moving (or landing into a slide!)
        let horiz_speed = (self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt();
        if !self.sliding && self.grounded && self.slide_cd <= 0.0 && (slide_pressed || (slide_held && horiz_speed > move_spd * 0.72)) {
            let dir = if wish.len_sq() > 1e-4 {
                wish
            } else if horiz_speed > 1.5 {
                Vec3::new(self.vel.x, 0.0, self.vel.z).norm()
            } else {
                fwd_xz
            };
            self.sliding = true;
            self.slide_timer = 0.72;
            self.slide_grace = 0.20;
            self.slide_cd = 0.35;
            self.slide_dir = dir;
            let boost = horiz_speed.max(slide_spd);
            self.vel.x = dir.x * boost;
            self.vel.z = dir.z * boost;
            self.fov_kick = (self.fov_kick + 9.0).min(12.0);
            self.queue_sfx(self.assets.sfx_select, 0.38, 0.62, 0.0);
        }

        if self.sliding {
            self.slide_timer -= dt;
            self.slide_grace = 0.18;
            // Subtle steering while gliding across the glossy Mahjong tiles
            let steer = wish * 10.0 * dt;
            self.vel.x += steer.x;
            self.vel.z += steer.z;
            let drag = (1.0 - slide_fric * dt).max(0.0);
            self.vel.x *= drag;
            self.vel.z *= drag;

            // Sparkling jade friction sparks beneath the sliding agent!
            if self.grounded && self.slide_spark_cd <= 0.0 {
                self.slide_spark_cd = 0.055;
                self.fx.push(FxBillboard {
                    tex: self.assets.muzzle_tex,
                    pos: self.pos + Vec3::new(self.rng.range(-0.35, 0.35), 0.18, self.rng.range(-0.35, 0.35)),
                    vel: Vec3::new(-self.vel.x * 0.25, self.rng.range(1.2, 2.8), -self.vel.z * 0.25),
                    size_start: 0.34,
                    size_end: 0.05,
                    life: 0.22,
                    max_life: 0.22,
                    color: Color::from_hex(0x38F5A8),
                });
            }

            let cur_hspd = (self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt();
            if self.slide_timer <= 0.0 || (!slide_held && self.slide_timer < 0.45) || cur_hspd < 4.5 {
                self.sliding = false;
            }
        } else {
            let accel = if self.grounded { 12.5 } else { 4.2 };
            let target_x = wish.x * move_spd;
            let target_z = wish.z * move_spd;
            // Preserve extra parkour air momentum when moving forward after a slide-jump!
            if !self.grounded && horiz_speed > move_spd && wish.len_sq() > 1e-4 {
                self.vel.x = lerp(self.vel.x, wish.x * horiz_speed, (3.2 * dt).min(1.0));
                self.vel.z = lerp(self.vel.z, wish.z * horiz_speed, (3.2 * dt).min(1.0));
            } else {
                self.vel.x = lerp(self.vel.x, target_x, (accel * dt).min(1.0));
                self.vel.z = lerp(self.vel.z, target_z, (accel * dt).min(1.0));
            }
        }

        // Jump & Slide-Jump Vault!
        if self.jump_buffer > 0.0 && self.coyote_timer > 0.0 {
            let was_slide = self.sliding || self.slide_grace > 0.0;
            self.sliding = false;
            self.coyote_timer = 0.0;
            self.jump_buffer = 0.0;
            self.grounded = false;
            if was_slide {
                self.vel.y = jump_vel * slide_jump_boost;
                let dir = if wish.len_sq() > 1e-4 { wish } else { self.slide_dir };
                let launch_spd = (horiz_speed + 2.4).min(slide_spd * 1.15);
                self.vel.x = dir.x * launch_spd;
                self.vel.z = dir.z * launch_spd;
                self.fov_kick = (self.fov_kick + 7.0).min(14.0);
                self.vault_flash = 0.55;
                self.queue_sfx(self.assets.sfx_select, 0.46, 1.32, 0.0);
            } else {
                self.vel.y = jump_vel;
            }
        }

        // Smooth camera duck, parkour roll & FOV speed kick
        let target_eye = if self.sliding { 0.86 } else { stand_eye };
        self.eye_height_cur = lerp(self.eye_height_cur, target_eye, (14.0 * dt).min(1.0));
        let target_roll = if self.sliding {
            if strafe_in.abs() > 0.1 { -strafe_in * 0.085 } else { -0.055 }
        } else {
            -strafe_in * 0.025
        };
        self.cam_roll = lerp(self.cam_roll, target_roll, (10.0 * dt).min(1.0));
        let target_fov_kick = if self.sliding { 8.5 } else if !self.grounded && horiz_speed > move_spd * 1.15 { 5.0 } else { 0.0 };
        self.fov_kick = lerp(self.fov_kick, target_fov_kick, (7.5 * dt).min(1.0));

        self.vel.y -= gravity * dt;
        self.pos += self.vel * dt;

        // Keep inside temple courtyard perimeter
        self.pos.x = clamp(self.pos.x, -16.5, 16.5);
        self.pos.z = clamp(self.pos.z, -15.5, 16.5);

        // Floor + walkable Mahjong tile AABB pushout with smooth Parkour Auto-Vault!
        let was_grounded = self.grounded;
        self.grounded = false;
        if self.pos.y <= 0.0 {
            self.pos.y = 0.0;
            self.vel.y = 0.0;
            self.grounded = true;
        }
        let body_h = if self.sliding { 1.05 } else { 1.75 };
        for t in &self.tiles {
            if t.removed {
                continue;
            }
            let pbox = Aabb::new(
                Vec3::new(self.pos.x - 0.42, self.pos.y, self.pos.z - 0.42),
                Vec3::new(self.pos.x + 0.42, self.pos.y + body_h, self.pos.z + 0.42),
            );
            let tb = t.aabb();
            if let Some(push) = aabb_pushout(pbox, tb) {
                // Parkour tile-edge mantle / smooth step-up (easily climb & slide over Mahjong tiers!)
                let step_delta = tb.max.y - self.pos.y;
                let max_step = if self.sliding || !was_grounded { 0.90 } else { 0.62 };
                if push.y == 0.0 && step_delta > 0.0 && step_delta <= max_step && self.vel.y >= -4.5 {
                    self.pos.y = tb.max.y;
                    if self.vel.y < 0.0 {
                        self.vel.y = 0.0;
                    }
                    self.grounded = true;
                } else {
                    self.pos += push;
                    if push.y > 0.0 {
                        self.vel.y = 0.0;
                        self.grounded = true;
                    } else if push.y < 0.0 && self.vel.y > 0.0 {
                        self.vel.y = 0.0;
                    }
                }
            }
        }

        // Update crosshair aimed tile raycast
        let eye = self.eye_pos(data);
        let (fwd, _, _) = self.look_basis();
        let aim_ray = Ray::new(eye, fwd);
        let mut best_tile: Option<(usize, f32)> = None;
        for (i, t) in self.tiles.iter().enumerate() {
            if t.removed {
                continue;
            }
            if let Some(d) = aim_ray.hit_aabb(t.aabb())
                && d < 18.0
                && best_tile.is_none_or(|(_, bd)| d < bd)
            {
                best_tile = Some((i, d));
            }
        }
        self.aimed_tile = best_tile.map(|(i, d)| (i, self.is_tile_free(i), d));

        // Combat & Tile Touch Actions
        if input.mouse_down(MouseButton::Left) || input.key_down(Key::F) {
            self.fire_laser(data);
        }
        if input.key_pressed(Key::E) || input.mouse_pressed(MouseButton::Right) {
            self.touch_aimed_tile(data);
        }

        // --- Weird Alien Ghost AI & Projectiles ---
        self.spawn_timer -= dt;
        if self.spawn_timer <= 0.0 {
            self.spawn_timer = spawn_rate;
            if self.ghosts.len() < max_ghosts {
                self.spawn_ghost();
            }
        }

        let player_eye = self.eye_pos(data);
        let mut new_orbs = Vec::new();
        let mut melee_hit = false;

        for g in &mut self.ghosts {
            g.bob_phase += dt * if g.kind == GhostKind::Warden { 2.8 } else { 4.2 };
            g.hit_flash = (g.hit_flash - dt).max(0.0);
            g.squash_timer = (g.squash_timer - dt).max(0.0);

            let to_p = player_eye - (g.pos + Vec3::new(0.0, 1.0, 0.0));
            let dist = to_p.len().max(0.01);
            g.yaw = to_p.x.atan2(to_p.z);

            let spd = if g.kind == GhostKind::Warden { warden_spd } else { scout_spd };
            let desired_dist = if g.kind == GhostKind::Warden { 7.5 } else { 4.2 };
            let tangent = Vec3::new(-to_p.z / dist, 0.0, to_p.x / dist);
            let radial = if dist > desired_dist { to_p / dist } else { -(to_p / dist) * 0.4 };
            let wish_v = (radial + tangent * (g.bob_phase * 0.4).sin() * 0.65).norm() * spd;
            g.vel = g.vel.lerp(wish_v, (3.2 * dt).min(1.0));
            g.pos += g.vel * dt;
            g.pos.y = lerp(g.pos.y, 1.9 + (g.bob_phase * 0.9).sin() * 0.55, (2.5 * dt).min(1.0));

            if dist < 1.55 {
                melee_hit = true;
                g.vel -= (to_p / dist) * 10.0;
            }

            g.shoot_cd -= dt;
            if g.shoot_cd <= 0.0 && dist < 24.0 {
                g.shoot_cd = if g.kind == GhostKind::Warden { 2.4 } else { 3.3 };
                g.squash_timer = 0.22;
                let col = if g.kind == GhostKind::Warden {
                    Color::from_hex(0xFF48B4)
                } else {
                    Color::from_hex(0x48FF84)
                };
                new_orbs.push(EctoOrb {
                    pos: g.pos + Vec3::new(0.0, 1.25, 0.0),
                    vel: (to_p / dist) * 12.5,
                    life: 3.5,
                    color: col,
                });
            }
        }
        self.orbs.extend(new_orbs);

        let mut took_dmg = 0.0f32;
        if melee_hit {
            took_dmg += 14.0;
        }
        for o in &mut self.orbs {
            o.life -= dt;
            o.pos += o.vel * dt;
            if (o.pos - player_eye).len_sq() < 0.85 * 0.85 {
                o.life = 0.0;
                took_dmg += 10.0;
            }
        }
        self.orbs.retain(|o| o.life > 0.0);

        if took_dmg > 0.0 {
            self.hp = (self.hp - took_dmg).max(0.0);
            self.shake_trauma = (self.shake_trauma + 0.55).min(1.0);
            self.flash(Color::from_hex(0xFF3860), 0.25);
            self.queue_sfx(self.assets.sfx_player_hit, 0.75, 1.0, 0.0);
            if self.hp <= 0.0 {
                self.phase = Phase::Defeat;
                self.set_radio("HQ: AGENT ECTOPLASMICALLY COMPROMISED. DEPLOYING REPLACEMENT.", 99.0);
            } else {
                self.set_radio("WARNING: ALIEN ECTOPLASM ON STANDARD-ISSUE UNIFORM.", 2.4);
            }
        }
    }

    pub fn render(
        &mut self,
        input: &InputState,
        data: &DataStore,
        draw: &mut DrawList,
        width: u32,
        height: u32,
        elapsed: f32,
    ) {
        let cfg = data.config("mahjong.cfg");

        // Atmospheric Twilight Chinese Temple Sky + Lantern Glow
        draw.clear_color = Color::from_hex(0x14172B);
        draw.sky_gradient = Some((Color::from_hex(0x0A0E22), Color::from_hex(0x3D213B)));
        draw.fog_color = Color::from_hex(0x281B34);
        draw.fog_start = 28.0;
        draw.fog_end = 95.0;
        draw.sun_dir = Vec3::new(0.42, 0.78, 0.45).norm();
        draw.sun_color = Color::from_hex(0xFFE0B8);
        draw.sky_color = Color::from_hex(0x3E4A74);
        draw.ground_color = Color::from_hex(0x261B24);
        draw.shadows_enabled = true;
        draw.shadow_extent = 24.0;

        let eye = self.eye_pos(data);
        let (fwd, right, up) = self.look_basis();
        let shake_amt = self.shake_trauma * self.shake_trauma;
        let shake_off = right * ((elapsed * 52.0).sin() * shake_amt * 0.22)
            + up * ((elapsed * 64.0).cos() * shake_amt * 0.18);

        draw.camera.pos = eye + shake_off;
        draw.camera.target = eye + fwd * 12.0 + shake_off * 0.4;
        draw.camera.up = (Vec3::Y + right * self.cam_roll).norm();
        draw.camera.fov_deg = clamp(self.fov_deg + self.fov_kick, 55.0, 112.0);
        draw.camera.near = 0.08;
        draw.camera.far = 220.0;

        let tint = if self.flash_timer > 0.0 {
            let a = (self.flash_timer * 2.4).min(0.45);
            Color::rgb(
                lerp(1.0, self.flash_color.r * 1.4, a),
                lerp(1.0, self.flash_color.g * 1.4, a),
                lerp(1.0, self.flash_color.b * 1.4, a),
            )
        } else {
            Color::WHITE
        };
        draw.post = PostSettings {
            enabled: true,
            exposure: 1.05,
            contrast: cfg.map(|c| c.f32("post.contrast", 1.05)).unwrap_or(1.05),
            saturation: cfg.map(|c| c.f32("post.saturation", 1.12)).unwrap_or(1.12),
            vignette: cfg.map(|c| c.f32("post.vignette", 0.22)).unwrap_or(0.22),
            bloom_strength: cfg.map(|c| c.f32("post.bloom", 0.48)).unwrap_or(0.48),
            bloom_threshold: cfg.map(|c| c.f32("post.threshold", 0.64)).unwrap_or(0.64),
            chroma: (self.shake_trauma * 0.65 + self.recoil * 0.8).min(0.8),
            time: elapsed,
            tint,
        };

        // --- TEMPLE COURTYARD ENVIRONMENT ---
        // Main inlaid stone courtyard floor + jade center border
        draw.solid_plane(0.0, 22.0, Color::from_hex(0x2F3544));
        draw.solid_box(
            Transform::from_pos_scale(Vec3::new(0.0, -0.02, 0.0), Vec3::new(15.2, 0.08, 13.6)),
            Color::from_hex(0x1E3D36),
        );
        // Subtle inlaid stone grid lines
        draw.debug_grid(14, 2.2, Color::rgba(0.55, 0.48, 0.36, 0.22), Color::rgba(0.9, 0.72, 0.3, 0.35));

        // 4 Ornate Crimson Temple Pillars with Warm Lantern Point Lights!
        let pillars = [
            (Vec3::new(-12.5, 0.0, -10.5), 0.75f32),
            (Vec3::new(12.5, 0.0, -10.5), -0.75f32),
            (Vec3::new(-12.5, 0.0, 10.5), 2.35f32),
            (Vec3::new(12.5, 0.0, 10.5), -2.35f32),
        ];
        for (pos, rot_y) in pillars {
            draw.mesh(
                self.assets.pillar,
                Transform { pos, rot: Quat::from_axis_angle(Vec3::Y, rot_y), scale: Vec3::ONE },
            );
            let l_off = Vec3::new(rot_y.sin() * 1.15, 4.45, rot_y.cos() * 1.15);
            draw.point_light(pos + l_off, Color::from_hex(0xFFAE44), 17.0, 2.2);
        }

        // Majestic Paifang Temple Gate on the North (-Z) edge
        draw.mesh(self.assets.gate_arch, Transform::from_pos(Vec3::new(0.0, 0.0, -15.5)));
        if self.muzzle_timer > 0.0 {
            draw.point_light(eye + fwd * 1.2, Color::from_hex(0x44FAFF), 9.0, 2.5);
        }

        // --- 3D MAHJONG SOLITAIRE PYRAMID ---
        for (i, t) in self.tiles.iter().enumerate() {
            if t.removed {
                continue;
            }
            let pos = t.world_pos();
            let is_free = self.is_tile_free(i);
            let is_selected = self.selected_tile == Some(i);
            let is_aimed = self.aimed_tile.is_some_and(|(ai, _, _)| ai == i);

            let tint = if is_selected {
                Color::from_hex(0x88FFCC)
            } else if is_aimed && is_free {
                Color::from_hex(0xFFF4AA)
            } else if is_free {
                Color::WHITE
            } else {
                Color::rgb(0.62, 0.64, 0.70)
            };

            let y_lift = if is_selected { 0.14 + (elapsed * 6.0).sin() * 0.05 } else { 0.0 };
            draw.mesh_tinted(
                self.assets.tile,
                Transform::from_pos(Vec3::new(pos.x, pos.y + y_lift, pos.z)),
                tint,
            );

            // Crisp SVG Mahjong emblem painted FLAT directly onto the top face of the 3D tile!
            // Climb, jump, and slide across the Mahjong tower to read upper-tier symbols!
            let decal_col = if is_selected {
                Color::from_hex(0xD8FFEE)
            } else if is_free {
                Color::WHITE
            } else {
                Color::rgba(0.66, 0.68, 0.74, 0.85)
            };
            let decal_sz = if is_selected { TILE_W * 0.84 } else { TILE_W * 0.78 };
            draw.decal_flat(
                Some(self.assets.tile_icons[t.symbol]),
                Vec3::new(pos.x, pos.y + TILE_H + 0.018 + y_lift, pos.z),
                Vec2::new(decal_sz, decal_sz),
                0.0,
                decal_col,
            );

            // Glowing vertical selection beacon + jade outline on selected tile!
            if is_selected {
                let top = Vec3::new(pos.x, pos.y + TILE_H + 0.2, pos.z);
                draw.debug_line(top, top + Vec3::new(0.0, 8.5, 0.0), Color::from_hex(0x38F5A8));
                draw.debug_box(
                    Vec3::new(pos.x, pos.y + TILE_H * 0.5 + y_lift, pos.z),
                    Vec3::new(TILE_W * 0.51, TILE_H * 0.52, TILE_D * 0.51),
                    Color::from_hex(0x38F5A8),
                );
            } else if is_aimed && is_free {
                draw.debug_box(
                    Vec3::new(pos.x, pos.y + TILE_H * 0.5, pos.z),
                    Vec3::new(TILE_W * 0.50, TILE_H * 0.51, TILE_D * 0.50),
                    Color::from_hex(0xFFD848),
                );
            }
            draw.collider_box(t.aabb().center(), t.aabb().half_extents(), Color::YELLOW);
        }

        // --- WEIRD ALIEN FLOATING GHOSTS (SQUASH & STRETCH!) ---
        for g in &self.ghosts {
            let idle_sq = (g.bob_phase * 2.0).sin() * 0.11;
            let hit_sq = if g.squash_timer > 0.0 {
                (g.squash_timer * 14.0).sin().abs() * 0.34
            } else {
                0.0
            };
            let sx = 1.0 + idle_sq + hit_sq;
            let sy = (1.0 - idle_sq - hit_sq * 0.75).max(0.52);
            let sz = 1.0 + idle_sq + hit_sq;

            let mesh = if g.kind == GhostKind::Warden {
                self.assets.ghost_warden
            } else {
                self.assets.ghost_scout
            };
            let hp_frac = clamp(g.hp / g.max_hp.max(1.0), 0.0, 1.0);
            let tint = if g.hit_flash > 0.0 {
                Color::rgb(2.4, 2.4, 2.4)
            } else {
                // Subtle red-hot destabilization tint as ghost armor drops
                let dmg_w = (1.0 - hp_frac) * 0.45;
                Color::rgb(1.0 + dmg_w * 0.9, 1.0 - dmg_w * 0.4, 1.0 - dmg_w * 0.4)
            };
            draw.mesh_tinted(
                mesh,
                Transform {
                    pos: g.pos,
                    rot: Quat::from_axis_angle(Vec3::Y, g.yaw),
                    scale: Vec3::new(sx, sy, sz),
                },
                tint,
            );
            // World-space tactical HP bar above wounded ghosts
            if hp_frac < 0.99 {
                let bar_c = g.pos + Vec3::new(0.0, 2.65, 0.0);
                let half_w = 0.65;
                let left_p = bar_c - right * half_w;
                let right_p = bar_c + right * half_w;
                let fill_p = left_p.lerp(right_p, hp_frac);
                let fill_col = if hp_frac > 0.45 {
                    Color::from_hex(0x38F5A8)
                } else {
                    Color::from_hex(0xFF3860)
                };
                for dy in [-0.03f32, 0.0, 0.03] {
                    let lift = Vec3::new(0.0, dy, 0.0);
                    draw.debug_line(left_p + lift, right_p + lift, Color::rgba(0.15, 0.12, 0.2, 0.9));
                    draw.debug_line(left_p + lift, fill_p + lift, fill_col);
                }
            }
            draw.collider_box(g.pos + Vec3::new(0.0, 1.1, 0.0), Vec3::splat(0.95), Color::from_hex(0x48FF84));
        }

        // Alien Plasma Ecto-Orbs
        for o in &self.orbs {
            draw.billboard(Some(self.assets.ecto_ring_tex), o.pos, Vec2::new(0.85, 0.85), o.color);
        }

        // --- 3D LASER BEAMS & SVG EFFECTS ---
        for b in &self.beams {
            let a = clamp(b.life / b.max_life, 0.0, 1.0);
            let col = Color::rgba(b.color.r, b.color.g, b.color.b, a);
            draw.debug_line(b.start, b.end, Color::rgba(1.0, 1.0, 1.0, a));
            for (ox, oy) in [(0.025, 0.0), (-0.025, 0.0), (0.0, 0.025), (0.0, -0.025)] {
                let off = right * ox + up * oy;
                draw.debug_line(b.start + off, b.end + off, col);
            }
        }

        for f in &self.fx {
            let t = 1.0 - clamp(f.life / f.max_life, 0.0, 1.0);
            let sz = lerp(f.size_start, f.size_end, t);
            let alpha = clamp(1.0 - t * t, 0.0, 1.0);
            draw.billboard(
                Some(f.tex),
                f.pos,
                Vec2::new(sz, sz),
                Color::rgba(f.color.r, f.color.g, f.color.b, alpha),
            );
        }

        // --- FIRST-PERSON TACTICAL LASER CARBINE VIEWMODEL ---
        if self.phase == Phase::Playing {
            let bob_x = if self.sliding { 0.0 } else { (self.walk_bob * 4.5).cos() * 0.018 };
            let bob_y = if self.sliding { 0.0 } else { (self.walk_bob * 9.0).sin().abs() * -0.02 };
            let slide_drop = if self.sliding { 0.04 } else { 0.0 };
            let gun_pos = draw.camera.pos
                + fwd * (0.64 - self.recoil)
                + right * (0.30 + bob_x)
                - up * (0.24 - bob_y + slide_drop);
            let gun_roll = if self.sliding { 0.24 } else { self.cam_roll * 1.4 };
            let gun_rot = Quat::from_euler_yxz(self.yaw, self.pitch + self.recoil * 0.9, gun_roll);
            draw.mesh(
                self.assets.rifle,
                Transform { pos: gun_pos, rot: gun_rot, scale: Vec3::splat(0.82) },
            );
            if self.muzzle_timer > 0.0 {
                let muzzle_w = gun_pos + fwd * 0.68 + up * 0.02;
                draw.billboard(
                    Some(self.assets.muzzle_tex),
                    muzzle_w,
                    Vec2::new(0.46, 0.46),
                    Color::from_hex(0x44FAFF),
                );
            }
        }

        // --- DEADPAN TACTICAL HUD ---
        self.render_hud(input, data, draw, width as f32, height as f32);
    }

    fn render_hud(&mut self, input: &InputState, data: &DataStore, draw: &mut DrawList, w: f32, h: f32) {
        let bg = Color::rgba(0.06, 0.07, 0.11, 0.86);
        let jade = Color::from_hex(0x38F5A8);
        let gold = Color::from_hex(0xF0BE3C);
        let crimson = Color::from_hex(0xE03038);
        let cyan = Color::from_hex(0x44FAFF);
        let white = Color::rgb(0.96, 0.97, 0.98);
        let dim = Color::rgb(0.68, 0.74, 0.80);

        // Top-Right: Settings + Audio quick buttons
        let mlbl = if self.muted { "AUDIO OFF" } else { "AUDIO ON" };
        if draw.ui_button(input, w - 154.0, 16.0, 138.0, 36.0, mlbl) {
            self.muted = !self.muted;
        }
        if !self.paused && draw.ui_button(input, w - 302.0, 16.0, 138.0, 36.0, "MENU [ESC]") {
            self.paused = true;
        }

        // Active Combat HUD (hidden on Title so the temple courtyard breathes cleanly)
        if self.phase != Phase::Title {
            // Top-Left: Bureau Tactical Status
            let left_w = 356.0;
            draw.ui_panel(16.0, 16.0, left_w, 84.0, bg, jade, 2.0);
            draw.ui_text(28.0, 26.0, 2.0, jade, "MAHJONG WARS // OPS");
            let pairs_left = self.total_pairs.saturating_sub(self.pairs_cleared);
            draw.ui_text(
                28.0,
                50.0,
                2.0,
                white,
                &format!("PAIRS LEFT {:>2}  SCORE {:05}", pairs_left, self.score),
            );
            draw.ui_text(
                28.0,
                72.0,
                2.0,
                gold,
                &format!("GHOSTS PURGED {:>2}  x{} COMBO", self.ghosts_purged, self.combo.max(1)),
            );

            // Top-Center: Deadpan HQ Radio Dispatch Banner (neatly centered in open top gap)
            if self.radio_timer > 0.0 && !self.radio_msg.is_empty() && self.phase == Phase::Playing && !self.paused {
                let (rw, _) = draw.ui_text_measure(2.0, &self.radio_msg);
                let pw = rw + 28.0;
                let left_edge = 16.0 + left_w + 16.0;
                let right_edge = w - 316.0;
                let (px, py) = if pw <= (right_edge - left_edge) {
                    (left_edge + ((right_edge - left_edge) - pw) * 0.5, 16.0)
                } else {
                    ((w - pw) * 0.5, 108.0)
                };
                draw.ui_panel(px, py, pw, 36.0, bg, gold, 2.0);
                draw.ui_text(px + 14.0, py + 11.0, 2.0, gold, &self.radio_msg);
            }

            // Bottom-Left: Held Mahjong Tile Tactical Slot (with SVG icon!) + Armor Bar
            draw.ui_panel(16.0, h - 102.0, 324.0, 86.0, bg, gold, 2.0);
            if let Some(sel_i) = self.selected_tile
                && !self.tiles[sel_i].removed
            {
                let sym = self.tiles[sel_i].symbol;
                draw.ui_sprite(self.assets.tile_icons[sym], 26.0, h - 92.0, 52.0, 52.0, Color::WHITE);
                draw.ui_text(88.0, h - 90.0, 2.0, jade, "LOCKED TILE:");
                draw.ui_text(88.0, h - 68.0, 2.0, white, SYMBOL_NAMES[sym]);
            } else {
                draw.ui_text(28.0, h - 88.0, 2.0, dim, "NO TILE LOCKED [E/RMB]");
                draw.ui_text(28.0, h - 66.0, 2.0, dim, "CLIMB & TOUCH FREE EDGES");
            }
            draw.ui_text(28.0, h - 34.0, 2.0, white, &format!("ARMOR {:>3.0}%", self.hp));
            draw.ui_bar(
                158.0,
                h - 32.0,
                168.0,
                12.0,
                self.hp / self.max_hp,
                Color::rgba(0.18, 0.16, 0.22, 1.0),
                if self.hp > 35.0 { jade } else { crimson },
            );

            // Parkour Momentum Badge above bottom-left panel when sliding or slide-vaulting!
            let hspd = (self.vel.x * self.vel.x + self.vel.z * self.vel.z).sqrt();
            if self.sliding || self.vault_flash > 0.0 {
                let (tag, col) = if self.vault_flash > 0.0 {
                    (format!("PARKOUR VAULT // {:>4.1} M/S", hspd), gold)
                } else {
                    (format!("POWER SLIDE // {:>4.1} M/S", hspd), cyan)
                };
                let (tw, _) = draw.ui_text_measure(2.0, &tag);
                draw.ui_panel(16.0, h - 140.0, tw + 24.0, 30.0, bg, col, 2.0);
                draw.ui_text(28.0, h - 132.0, 2.0, col, &tag);
            }
        }

        // Center-Screen Dynamic SVG Tactical Reticle + Context Touch Prompt
        if self.phase == Phase::Playing && !self.paused {
            let ret_sz = 34.0 + self.recoil * 70.0;
            let ret_col = if self.hitmarker_timer > 0.0 {
                crimson
            } else if self.aimed_tile.is_some_and(|(_, free, d)| free && d < 7.2) {
                gold
            } else {
                cyan
            };
            draw.ui_sprite(
                self.assets.reticle_tex,
                (w - ret_sz) * 0.5,
                (h - ret_sz) * 0.5,
                ret_sz,
                ret_sz,
                ret_col,
            );

            let range = data.config("mahjong.cfg").map(|c| c.f32("player.touch_range", 7.2)).unwrap_or(7.2);
            if let Some((ti, is_free, dist)) = self.aimed_tile
                && dist <= range + 2.5
            {
                let sym = SYMBOL_NAMES[self.tiles[ti].symbol];
                let (prompt, col) = if !is_free {
                    (format!("{sym} [OBSTRUCTED - CLEAR EDGES]"), crimson)
                } else if dist > range {
                    (format!("{sym} [MOVE CLOSER TO TOUCH]"), dim)
                } else {
                    (format!("[E / RMB] TOUCH {sym}"), gold)
                };
                let (pw, _) = draw.ui_text_measure(2.0, &prompt);
                let bx = (w - pw) * 0.5;
                draw.ui_panel(bx - 14.0, h * 0.5 + 34.0, pw + 28.0, 32.0, bg, col, 2.0);
                draw.ui_text(bx, h * 0.5 + 43.0, 2.0, col, &prompt);
            }
        }

        // Title Screen Overlay
        if self.phase == Phase::Title && !self.paused {
            let bw = 684.0;
            let bh = 252.0;
            let bx = (w - bw) * 0.5;
            let by = h * 0.07;
            draw.ui_panel(bx, by, bw, bh, Color::rgba(0.05, 0.06, 0.10, 0.88), gold, 3.0);

            let t = "MAHJONG WARS";
            let (tw, _) = draw.ui_text_measure(5.0, t);
            draw.ui_text((w - tw) * 0.5 + 2.0, by + 22.0, 5.0, crimson, t);
            draw.ui_text((w - tw) * 0.5, by + 20.0, 5.0, gold, t);

            let sub = "DEPT. OF PARANORMAL TILE OPERATIONS";
            let (sw, _) = draw.ui_text_measure(2.0, sub);
            draw.ui_text((w - sw) * 0.5, by + 70.0, 2.0, jade, sub);

            let dir = "DIRECTIVE 404: CLIMB THE TOWER. CLEAR 16 PAIRS.";
            let (dw, _) = draw.ui_text_measure(2.0, dir);
            draw.ui_text((w - dw) * 0.5, by + 98.0, 2.0, gold, dir);

            let c1 = "WASD MOVE  SPACE JUMP  SHIFT/C SLIDE  LMB LASER";
            let (c1w, _) = draw.ui_text_measure(2.0, c1);
            draw.ui_text((w - c1w) * 0.5, by + 132.0, 2.0, white, c1);

            let c2 = "E / RMB TOUCH FREE TILE   ESC PAUSE & SETTINGS";
            let (c2w, _) = draw.ui_text_measure(2.0, c2);
            draw.ui_text((w - c2w) * 0.5, by + 158.0, 2.0, dim, c2);

            if draw.ui_button(input, (w - 240.0) * 0.5, by + 194.0, 240.0, 42.0, "DEPLOY AGENT [SPC]") {
                self.start_run();
            }
        }

        // Deadpan Pause & Settings Menu Overlay (interactive sliders & toggles!)
        if self.paused {
            draw.ui_rect(0.0, 0.0, w, h, Color::rgba(0.03, 0.04, 0.08, 0.62));
            let bw = 540.0;
            let bh = 412.0;
            let bx = (w - bw) * 0.5;
            let by = (h - bh) * 0.5;
            draw.ui_panel(bx, by, bw, bh, Color::rgba(0.05, 0.06, 0.11, 0.96), gold, 3.0);

            let heading = if self.phase == Phase::Playing {
                "OPERATION PAUSED"
            } else {
                "BUREAU SETTINGS"
            };
            let (hw, _) = draw.ui_text_measure(3.0, heading);
            draw.ui_text((w - hw) * 0.5, by + 22.0, 3.0, gold, heading);

            let sub = "PARANORMAL TILE OPS // FIELD CALIBRATION";
            let (sw, _) = draw.ui_text_measure(2.0, sub);
            draw.ui_text((w - sw) * 0.5, by + 56.0, 2.0, jade, sub);

            let lx = bx + 28.0;
            let sx = bx + 258.0;
            let sw_bar = 186.0;
            let vx = bx + 456.0;

            // 1. Mouse Sensitivity Slider
            let y1 = by + 98.0;
            draw.ui_text(lx, y1 + 4.0, 2.0, white, "MOUSE SENSITIVITY");
            draw.ui_slider(input, sx, y1, sw_bar, 22.0, 0.10, 2.50, &mut self.mouse_sens_mult);
            draw.ui_text(vx, y1 + 4.0, 2.0, cyan, &format!("{:.2}X", self.mouse_sens_mult));

            // 2. Master Volume Slider
            let y2 = by + 144.0;
            draw.ui_text(lx, y2 + 4.0, 2.0, white, "MASTER VOLUME");
            draw.ui_slider(input, sx, y2, sw_bar, 22.0, 0.0, 1.0, &mut self.master_volume);
            draw.ui_text(vx, y2 + 4.0, 2.0, cyan, &format!("{:>3.0}%", self.master_volume * 100.0));

            // 3. Field of View Slider
            let y3 = by + 190.0;
            draw.ui_text(lx, y3 + 4.0, 2.0, white, "FIELD OF VIEW");
            draw.ui_slider(input, sx, y3, sw_bar, 22.0, 58.0, 100.0, &mut self.fov_deg);
            draw.ui_text(vx, y3 + 4.0, 2.0, cyan, &format!("{:>3.0}", self.fov_deg));

            // 4. Toggle Buttons Row (Invert Y + Mute)
            let y4 = by + 236.0;
            let inv_lbl = if self.invert_y { "INVERT Y: ON" } else { "INVERT Y: OFF" };
            if draw.ui_button(input, lx, y4, 232.0, 36.0, inv_lbl) {
                self.invert_y = !self.invert_y;
            }
            let aud_lbl = if self.muted { "AUDIO: MUTED" } else { "AUDIO: ACTIVE" };
            if draw.ui_button(input, bx + bw - 260.0, y4, 232.0, 36.0, aud_lbl) {
                self.muted = !self.muted;
            }

            // 5. Action Buttons
            let resume_lbl = if self.phase == Phase::Playing {
                "RESUME OPERATION [ESC]"
            } else {
                "CLOSE SETTINGS [ESC]"
            };
            if draw.ui_button(input, lx, by + 294.0, bw - 56.0, 42.0, resume_lbl) {
                self.paused = false;
            }
            if draw.ui_button(input, lx, by + 350.0, 232.0, 38.0, "RESTART BOARD") {
                self.start_run();
            }
            if draw.ui_button(input, bx + bw - 260.0, by + 350.0, 232.0, 38.0, "QUIT TO DESKTOP") {
                self.quit_requested = true;
            }
        }

        // Victory / Defeat Overlay
        if matches!(self.phase, Phase::Victory | Phase::Defeat) {
            let bw = 480.0;
            let bh = 210.0;
            let bx = (w - bw) * 0.5;
            let by = (h - bh) * 0.45;
            let accent = if self.phase == Phase::Victory { jade } else { crimson };
            draw.ui_panel(bx, by, bw, bh, Color::rgba(0.06, 0.06, 0.10, 0.92), accent, 3.0);

            let heading = if self.phase == Phase::Victory {
                "HARMONY RESTORED"
            } else {
                "AGENT COMPROMISED"
            };
            let (hw, _) = draw.ui_text_measure(4.0, heading);
            draw.ui_text((w - hw) * 0.5, by + 24.0, 4.0, accent, heading);

            let s1 = format!("FINAL SCORE {}   PAIRS {}/{}", self.score, self.pairs_cleared, self.total_pairs);
            let (s1w, _) = draw.ui_text_measure(2.0, &s1);
            draw.ui_text((w - s1w) * 0.5, by + 78.0, 2.0, white, &s1);

            let s2 = format!("ALIEN GHOSTS PURGED: {}", self.ghosts_purged);
            let (s2w, _) = draw.ui_text_measure(2.0, &s2);
            draw.ui_text((w - s2w) * 0.5, by + 108.0, 2.0, gold, &s2);

            if draw.ui_button(input, (w - 240.0) * 0.5, by + 146.0, 240.0, 42.0, "NEW ASSIGNMENT [R]") {
                self.start_run();
            }
        }
    }

    pub fn save_snapshot(&self, snap: &mut Snapshot) {
        snap.set_str("game", "phase", self.phase.as_str());
        snap.set_vec3("player", "pos", self.pos);
        snap.set_f32("player", "yaw", self.yaw);
        snap.set_f32("player", "pitch", self.pitch);
        snap.set_f32("player", "hp", self.hp);
        snap.set_i32("game", "score", self.score as i32);
        snap.set_i32("game", "pairs_cleared", self.pairs_cleared as i32);
    }

    pub fn load_snapshot(&mut self, snap: &Snapshot) {
        self.phase = match snap.get_str("game", "phase", "title") {
            "playing" => Phase::Playing,
            "victory" => Phase::Victory,
            "defeat" => Phase::Defeat,
            _ => Phase::Title,
        };
        self.pos = snap.get_vec3("player", "pos", self.pos);
        self.yaw = snap.get_f32("player", "yaw", self.yaw);
        self.pitch = snap.get_f32("player", "pitch", self.pitch);
        self.hp = snap.get_f32("player", "hp", self.hp);
        self.score = snap.get_i32("game", "score", self.score as i32).max(0) as u32;
        self.pairs_cleared = snap.get_i32("game", "pairs_cleared", self.pairs_cleared as i32).max(0) as u32;
    }
}

/// Genuine Mahjong Solitaire legality check:
/// Free iff no remaining tile sits on `layer + 1` overlapping `(gx, gz)`, AND at least one of
/// left (`gx - 2`) or right (`gx + 2`) on the same layer is open.
fn is_slot_free(tiles: &[Tile], removed: &[bool], idx: usize) -> bool {
    let t = &tiles[idx];
    let mut blocked_left = false;
    let mut blocked_right = false;
    for (j, o) in tiles.iter().enumerate() {
        if j == idx || removed[j] {
            continue;
        }
        if o.layer == t.layer + 1 && (o.gx - t.gx).abs() <= 1 && (o.gz - t.gz).abs() <= 1 {
            return false;
        }
        if o.layer == t.layer && (o.gz - t.gz).abs() <= 1 {
            if o.gx == t.gx - 2 {
                blocked_left = true;
            }
            if o.gx == t.gx + 2 {
                blocked_right = true;
            }
        }
    }
    !(blocked_left && blocked_right)
}
