//! COZY GOLF — Warm, Inviting 2D Mini Golf built on Dullahan Framework.
//!
//! Pure 2D showcase exercising `Camera2D`, 4x4 AA SVG sprites, rotated 2D quads & lines,
//! sub-stepped 2D circle-vs-segment/bumper/windmill physics, bounce trajectory preview,
//! warm storybook parchment UI, interactive `ui_slider` settings, and procedural acoustic audio.

use dullahan::App;
use dullahan::assets::{SoundHandle, TexHandle};
use dullahan::audio::{AudioBus, PlayParams, VoiceId};
use dullahan::data::{DataStore, Snapshot};
use dullahan::gfx::{Camera2D, DebugOverlay, DrawList};
use dullahan::math::{Color, Rng, Vec2, clamp, lerp};
use dullahan::platform::{InputState, Key, MouseButton};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Title,
    Playing,
    HoleClear,
    Scorecard,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Playing => "playing",
            Self::HoleClear => "hole_clear",
            Self::Scorecard => "scorecard",
        }
    }
}

#[derive(Clone, Copy)]
struct RectZone {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectZone {
    fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    fn contains(self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }
}

#[derive(Clone, Copy)]
struct WallSeg {
    a: Vec2,
    b: Vec2,
    half_thick: f32,
}

impl WallSeg {
    fn new(ax: f32, ay: f32, bx: f32, by: f32) -> Self {
        Self { a: Vec2::new(ax, ay), b: Vec2::new(bx, by), half_thick: 8.0 }
    }
}

#[derive(Clone, Copy)]
struct Bumper {
    pos: Vec2,
    radius: f32,
    pulse: f32,
}

#[derive(Clone, Copy)]
struct Windmill {
    center: Vec2,
    half_len: f32,
    half_thick: f32,
    angle: f32,
    speed: f32,
}

#[derive(Clone, Copy)]
struct SandTrap {
    pos: Vec2,
    rx: f32,
    ry: f32,
}

impl SandTrap {
    fn contains(self, p: Vec2) -> bool {
        let dx = (p.x - self.pos.x) / self.rx.max(1.0);
        let dy = (p.y - self.pos.y) / self.ry.max(1.0);
        dx * dx + dy * dy <= 1.0
    }
}

#[derive(Clone, Copy)]
struct BoostPad {
    pos: Vec2,
    size: Vec2,
    dir: Vec2,
}

impl BoostPad {
    fn contains(self, p: Vec2) -> bool {
        (p.x - self.pos.x).abs() <= self.size.x * 0.5 && (p.y - self.pos.y).abs() <= self.size.y * 0.5
    }
}

#[derive(Clone, Copy)]
enum SceneryKind {
    Tree,
    Flower,
    Lilypad,
}

#[derive(Clone, Copy)]
struct Scenery {
    kind: SceneryKind,
    pos: Vec2,
    size: f32,
    phase: f32,
}

#[derive(Clone)]
struct HoleDef {
    name: &'static str,
    par: u32,
    tee: Vec2,
    cup: Vec2,
    fairways: Vec<RectZone>,
    bridges: Vec<RectZone>,
    waters: Vec<RectZone>,
    walls: Vec<WallSeg>,
    bumpers: Vec<Bumper>,
    windmills: Vec<Windmill>,
    sands: Vec<SandTrap>,
    boosts: Vec<BoostPad>,
    scenery: Vec<Scenery>,
}

struct Particle2D {
    pos: Vec2,
    vel: Vec2,
    size_start: f32,
    size_end: f32,
    life: f32,
    max_life: f32,
    color: Color,
    is_star: bool,
    rot: f32,
    rot_spd: f32,
}

struct Ripple2D {
    pos: Vec2,
    r_start: f32,
    r_end: f32,
    life: f32,
    max_life: f32,
    color: Color,
}

struct SoundEvent {
    sound: SoundHandle,
    params: PlayParams,
}

pub struct GameAssets {
    ball: TexHandle,
    hole: TexHandle,
    flag: TexHandle,
    bumper: TexHandle,
    tree: TexHandle,
    flower: TexHandle,
    lilypad: TexHandle,
    sand: TexHandle,
    boost: TexHandle,
    star: TexHandle,
    disc: TexHandle,
    music: SoundHandle,
    sfx_putt: SoundHandle,
    sfx_wall: SoundHandle,
    sfx_bumper: SoundHandle,
    sfx_boost: SoundHandle,
    sfx_sand: SoundHandle,
    sfx_splash: SoundHandle,
    sfx_hole: SoundHandle,
}

impl GameAssets {
    pub fn load(app: &mut App) -> Self {
        app.data.load_config("cozy.cfg");
        Self {
            ball: app.load_svg("ui/ball.svg", 64, 64),
            hole: app.load_svg("ui/hole.svg", 64, 64),
            flag: app.load_svg("ui/flag.svg", 96, 96),
            bumper: app.load_svg("ui/bumper.svg", 96, 96),
            tree: app.load_svg("ui/tree.svg", 128, 128),
            flower: app.load_svg("ui/flower.svg", 80, 80),
            lilypad: app.load_svg("ui/lilypad.svg", 80, 80),
            sand: app.load_svg("ui/sand.svg", 128, 128),
            boost: app.load_svg("ui/boost.svg", 96, 96),
            star: app.load_svg("ui/star.svg", 64, 64),
            disc: app.load_svg("ui/disc.svg", 48, 48),
            music: app.load_sound("sfx/music_cozy.wav"),
            sfx_putt: app.load_sound("sfx/putt.wav"),
            sfx_wall: app.load_sound("sfx/wall_hit.wav"),
            sfx_bumper: app.load_sound("sfx/bumper.wav"),
            sfx_boost: app.load_sound("sfx/boost.wav"),
            sfx_sand: app.load_sound("sfx/sand.wav"),
            sfx_splash: app.load_sound("sfx/splash.wav"),
            sfx_hole: app.load_sound("sfx/hole_in.wav"),
        }
    }
}

pub struct GameState {
    pub phase: Phase,
    pub paused: bool,
    pub quit_requested: bool,
    pub assets: GameAssets,
    music_voice: Option<VoiceId>,

    // Settings
    pub muted: bool,
    pub master_volume: f32,
    pub show_aim_guide: bool,

    // Course & Scoring
    holes: Vec<HoleDef>,
    pub hole_idx: usize,
    pub strokes: u32,
    pub hole_scores: Vec<u32>,

    // Ball & Physics State (2D world coordinates centered around (0, 0), ~1040 x 560)
    pub ball_pos: Vec2,
    pub ball_vel: Vec2,
    last_safe_pos: Vec2,
    ball_sink: f32,
    ball_squash: f32,
    ball_squash_angle: f32,
    sand_sfx_cd: f32,
    boost_sfx_cd: f32,
    flag_lift: f32,

    // Drag-to-Putt Aiming
    pub dragging: bool,
    drag_start: Vec2,
    drag_cur: Vec2,
    forced_aim: Option<Vec2>,

    // Camera2D & Juice
    cam: Camera2D,
    cam_shake: f32,
    toast_text: String,
    toast_color: Color,
    toast_timer: f32,
    clear_timer: f32,
    particles: Vec<Particle2D>,
    ripples: Vec<Ripple2D>,
    motes: Vec<(Vec2, f32)>,
    rng: Rng,
    queued_sounds: Vec<SoundEvent>,
}

impl GameState {
    pub fn new(app: &mut App) -> Self {
        let assets = GameAssets::load(app);
        let music_voice = app.play_loop(assets.music, 0.34, AudioBus::Music);
        let holes = build_course();
        let mut rng = Rng::new(0xC027_601F_2026);
        let mut motes = Vec::with_capacity(28);
        for _ in 0..28 {
            motes.push((
                Vec2::new(rng.range(-640.0, 640.0), rng.range(-360.0, 360.0)),
                rng.range(0.0, 6.28),
            ));
        }

        let tee = holes[0].tee;
        let mut s = Self {
            phase: Phase::Title,
            paused: false,
            quit_requested: false,
            assets,
            music_voice,
            muted: false,
            master_volume: 0.85,
            show_aim_guide: true,
            holes,
            hole_idx: 0,
            strokes: 0,
            hole_scores: Vec::new(),
            ball_pos: tee,
            ball_vel: Vec2::ZERO,
            last_safe_pos: tee,
            ball_sink: 0.0,
            ball_squash: 0.0,
            ball_squash_angle: 0.0,
            sand_sfx_cd: 0.0,
            boost_sfx_cd: 0.0,
            flag_lift: 0.0,
            dragging: false,
            drag_start: Vec2::ZERO,
            drag_cur: Vec2::ZERO,
            forced_aim: None,
            cam: Camera2D { pos: Vec2::ZERO, zoom: 1.0 },
            cam_shake: 0.0,
            toast_text: String::new(),
            toast_color: Color::WHITE,
            toast_timer: 0.0,
            clear_timer: 0.0,
            particles: Vec::new(),
            ripples: Vec::new(),
            motes,
            rng,
            queued_sounds: Vec::new(),
        };
        if let Some(snap) = app.startup_snapshot() {
            s.load_snapshot(&snap);
        }
        s
    }

    pub fn hole_count(&self) -> usize {
        self.holes.len()
    }

    pub fn current_par(&self) -> u32 {
        self.holes[self.hole_idx].par
    }

    pub fn start_course(&mut self) {
        self.hole_scores.clear();
        self.paused = false;
        self.load_hole(0);
    }

    pub fn load_hole(&mut self, idx: usize) {
        self.hole_idx = idx.min(self.holes.len().saturating_sub(1));
        self.phase = Phase::Playing;
        self.paused = false;
        self.strokes = 0;
        let tee = self.holes[self.hole_idx].tee;
        self.ball_pos = tee;
        self.ball_vel = Vec2::ZERO;
        self.last_safe_pos = tee;
        self.ball_sink = 0.0;
        self.ball_squash = 0.0;
        self.dragging = false;
        self.forced_aim = None;
        self.particles.clear();
        self.ripples.clear();
        self.clear_timer = 0.0;
        let h = &self.holes[self.hole_idx];
        self.show_toast(
            &format!("HOLE {} - {} (PAR {})", self.hole_idx + 1, h.name, h.par),
            Color::from_hex(0xFAF2DF),
            2.4,
        );
    }

    pub fn next_hole(&mut self) {
        if self.hole_idx + 1 < self.holes.len() {
            self.load_hole(self.hole_idx + 1);
        } else {
            self.phase = Phase::Scorecard;
        }
    }

    pub fn set_demo_aim(&mut self, aim_vel: Vec2) {
        self.forced_aim = Some(aim_vel);
    }

    pub fn execute_putt(&mut self, launch_vel: Vec2, data: &DataStore) {
        let max_p = data.config("cozy.cfg").map(|c| c.f32("ball.max_power", 840.0)).unwrap_or(840.0);
        let spd = launch_vel.len().min(max_p);
        if spd < 25.0 {
            return;
        }
        let dir = launch_vel.norm();
        self.last_safe_pos = self.ball_pos;
        self.ball_vel = dir * spd;
        self.strokes += 1;
        self.forced_aim = None;
        self.dragging = false;
        self.ball_squash = 0.28 * (spd / max_p).clamp(0.35, 1.0);
        self.ball_squash_angle = dir.y.atan2(dir.x);
        self.cam_shake = (self.cam_shake + (spd / max_p) * 3.5).min(6.0);

        let pitch = 0.88 + (spd / max_p) * 0.28;
        let gain = 0.42 + (spd / max_p) * 0.36;
        self.queue_sfx(self.assets.sfx_putt, gain, pitch, 0.0);
        self.spawn_burst(self.ball_pos, 7, Color::from_hex(0xD7ED9B), 75.0, false);
    }

    fn show_toast(&mut self, text: &str, color: Color, dur: f32) {
        self.toast_text = text.to_string();
        self.toast_color = color;
        self.toast_timer = dur;
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
            let g = if self.paused { 0.14 } else if self.phase == Phase::Title { 0.26 } else { 0.34 };
            app.audio.set_voice(vid, g, 1.0, 0.0);
        }
        app.audio.set_bus_gain(AudioBus::Sfx, self.master_volume);
        app.audio.set_bus_gain(AudioBus::Music, self.master_volume * 0.85);
        app.audio.set_muted(self.muted);
    }

    fn spawn_burst(&mut self, pos: Vec2, count: usize, color: Color, speed: f32, is_star: bool) {
        for _ in 0..count {
            let a = self.rng.range(0.0, std::f32::consts::TAU);
            let sp = self.rng.range(speed * 0.35, speed);
            let life = self.rng.range(0.35, if is_star { 1.1 } else { 0.65 });
            self.particles.push(Particle2D {
                pos,
                vel: Vec2::new(a.cos() * sp, a.sin() * sp),
                size_start: if is_star { self.rng.range(18.0, 28.0) } else { self.rng.range(7.0, 12.0) },
                size_end: if is_star { 4.0 } else { 1.5 },
                life,
                max_life: life,
                color,
                is_star,
                rot: self.rng.range(0.0, 6.28),
                rot_spd: self.rng.range(-5.0, 5.0),
            });
        }
    }

    pub fn update(&mut self, input: &InputState, data: &DataStore, dt: f32, width: u32, height: u32) {
        if input.key_pressed(Key::M) {
            self.muted = !self.muted;
        }
        // Fit the 1080x620 world nicely inside any viewport size with warm breathing room
        let fit_x = width as f32 / 1180.0;
        let fit_y = height as f32 / 680.0;
        self.cam.zoom = fit_x.min(fit_y).clamp(0.45, 2.0);
        self.cam.pos = Vec2::new(0.0, -8.0);

        if self.paused {
            return;
        }

        // Gentle drifting meadow pollen motes (even on Title!)
        for (mp, ph) in &mut self.motes {
            *ph += dt * 1.1;
            mp.x += (12.0 + ph.sin() * 6.0) * dt;
            mp.y += (ph.cos() * 7.0 - 3.0) * dt;
            if mp.x > 640.0 { mp.x = -640.0; }
            if mp.y < -360.0 { mp.y = 360.0; }
            if mp.y > 360.0 { mp.y = -360.0; }
        }

        // Animate windmills & bumper pulses
        for wm in &mut self.holes[self.hole_idx].windmills {
            wm.angle += wm.speed * dt;
        }
        for b in &mut self.holes[self.hole_idx].bumpers {
            b.pulse = (b.pulse - dt * 3.5).max(0.0);
        }

        // Update particles & pond ripples
        for p in &mut self.particles {
            p.life -= dt;
            p.pos += p.vel * dt;
            p.vel *= (1.0 - 2.6 * dt).max(0.0);
            p.rot += p.rot_spd * dt;
        }
        self.particles.retain(|p| p.life > 0.0);
        for r in &mut self.ripples {
            r.life -= dt;
        }
        self.ripples.retain(|r| r.life > 0.0);
        self.cam_shake = (self.cam_shake - dt * 16.0).max(0.0);
        self.ball_squash = (self.ball_squash - dt * 2.2).max(0.0);
        self.sand_sfx_cd = (self.sand_sfx_cd - dt).max(0.0);
        self.boost_sfx_cd = (self.boost_sfx_cd - dt).max(0.0);
        if self.toast_timer > 0.0 {
            self.toast_timer -= dt;
        }

        match self.phase {
            Phase::Title => {
                if input.key_pressed(Key::Space) || input.key_pressed(Key::Enter) {
                    self.start_course();
                }
                return;
            }
            Phase::HoleClear => {
                self.ball_sink = (self.ball_sink + dt * 3.5).min(1.0);
                let cup = self.holes[self.hole_idx].cup;
                self.ball_pos = self.ball_pos.lerp(cup, (10.0 * dt).min(1.0));
                self.clear_timer += dt;
                if input.key_pressed(Key::Space) || input.key_pressed(Key::Enter) {
                    self.next_hole();
                }
                return;
            }
            Phase::Scorecard => {
                if input.key_pressed(Key::Space) || input.key_pressed(Key::R) {
                    self.start_course();
                }
                return;
            }
            Phase::Playing => {}
        }

        if input.key_pressed(Key::R) {
            self.load_hole(self.hole_idx);
            return;
        }

        let cfg = data.config("cozy.cfg");
        let ball_r = cfg.map(|c| c.f32("ball.radius", 13.0)).unwrap_or(13.0);
        let friction = cfg.map(|c| c.f32("ball.friction", 1.45)).unwrap_or(1.45);
        let sand_fric = cfg.map(|c| c.f32("ball.sand_friction", 5.8)).unwrap_or(5.8);
        let stop_spd = cfg.map(|c| c.f32("ball.stop_speed", 14.0)).unwrap_or(14.0);
        let max_power = cfg.map(|c| c.f32("ball.max_power", 840.0)).unwrap_or(840.0);
        let drag_scale = cfg.map(|c| c.f32("ball.drag_scale", 4.4)).unwrap_or(4.4);
        let wall_bounce = cfg.map(|c| c.f32("ball.wall_bounce", 0.78)).unwrap_or(0.78);
        let bumper_bounce = cfg.map(|c| c.f32("ball.bumper_bounce", 1.25)).unwrap_or(1.25);
        let bumper_min = cfg.map(|c| c.f32("ball.bumper_min_spd", 290.0)).unwrap_or(290.0);
        let boost_accel = cfg.map(|c| c.f32("ball.boost_accel", 920.0)).unwrap_or(920.0);
        let hole_r = cfg.map(|c| c.f32("hole.radius", 21.0)).unwrap_or(21.0);
        let capture_spd = cfg.map(|c| c.f32("hole.capture_speed", 430.0)).unwrap_or(430.0);

        let is_resting = self.ball_vel.len_sq() < 1.0;

        // --- Slingshot Drag-to-Putt Input (ignores top UI bar clicks) ---
        let world_mouse = self.cam.screen_to_world(input.mouse_pos, width as f32, height as f32);
        let in_top_ui = input.mouse_pos.y < 68.0;
        if is_resting {
            if input.mouse_pressed(MouseButton::Left) && !in_top_ui {
                self.dragging = true;
                self.forced_aim = None;
                self.drag_start = world_mouse;
                self.drag_cur = world_mouse;
            }
            if self.dragging {
                self.drag_cur = world_mouse;
                if input.mouse_pressed(MouseButton::Right) {
                    self.dragging = false;
                } else if input.mouse_released(MouseButton::Left) {
                    let pull = self.drag_start - self.drag_cur;
                    let launch = pull * drag_scale;
                    self.dragging = false;
                    if launch.len() >= 32.0 {
                        self.execute_putt(launch, data);
                    }
                }
            }
        } else {
            self.dragging = false;
        }

        // Lift flag smoothly as the ball approaches the cup
        let cup = self.holes[self.hole_idx].cup;
        let dist_cup = (self.ball_pos - cup).len();
        let target_lift = if dist_cup < 95.0 { 1.0 } else { 0.0 };
        self.flag_lift = lerp(self.flag_lift, target_lift, (9.0 * dt).min(1.0));

        // --- Sub-Stepped 2D Mini Golf Physics ---
        if !is_resting {
            const SUBSTEPS: usize = 8;
            let sub_dt = dt / SUBSTEPS as f32;
            let mut hit_wall_spd = 0.0f32;
            let mut hit_bumper_pos: Option<Vec2> = None;
            let mut in_sand = false;
            let mut on_boost = false;
            let mut sank = false;
            let mut splashed = false;

            for _ in 0..SUBSTEPS {
                self.ball_pos += self.ball_vel * sub_dt;

                // Boost pads
                for pad in &self.holes[self.hole_idx].boosts {
                    if pad.contains(self.ball_pos) {
                        self.ball_vel += pad.dir * (boost_accel * sub_dt);
                        let s = self.ball_vel.len();
                        if s > max_power * 1.15 {
                            self.ball_vel = self.ball_vel.norm() * (max_power * 1.15);
                        }
                        on_boost = true;
                    }
                }

                // Wall segments (circle vs capsule segment)
                for w in &self.holes[self.hole_idx].walls {
                    if let Some((n, pen)) = circle_vs_segment(self.ball_pos, ball_r, w.a, w.b, w.half_thick) {
                        self.ball_pos += n * pen;
                        let vn = self.ball_vel.dot(n);
                        if vn < 0.0 {
                            self.ball_vel -= n * ((1.0 + wall_bounce) * vn);
                            hit_wall_spd = hit_wall_spd.max(-vn);
                        }
                    }
                }

                // Rotating wooden windmill paddles
                for wm in &self.holes[self.hole_idx].windmills {
                    let dir = Vec2::new(wm.angle.cos(), wm.angle.sin());
                    let a = wm.center - dir * wm.half_len;
                    let b = wm.center + dir * wm.half_len;
                    if let Some((n, pen)) = circle_vs_segment(self.ball_pos, ball_r, a, b, wm.half_thick) {
                        self.ball_pos += n * pen;
                        let vn = self.ball_vel.dot(n);
                        if vn < 0.0 {
                            self.ball_vel -= n * ((1.0 + wall_bounce) * vn);
                            hit_wall_spd = hit_wall_spd.max(-vn);
                        }
                        // Add slight rotational push from moving wooden blade
                        let arm = self.ball_pos - wm.center;
                        let tangent = Vec2::new(-arm.y, arm.x) * wm.speed;
                        self.ball_vel += tangent * 0.35;
                    }
                }

                // Bouncy garden toadstool bumpers
                for b in &mut self.holes[self.hole_idx].bumpers {
                    let delta = self.ball_pos - b.pos;
                    let d = delta.len();
                    let min_d = ball_r + b.radius;
                    if d < min_d && d > 1e-4 {
                        let n = delta / d;
                        self.ball_pos = b.pos + n * min_d;
                        let vn = self.ball_vel.dot(n);
                        if vn < 0.0 {
                            self.ball_vel -= n * ((1.0 + bumper_bounce) * vn);
                        }
                        let cur_s = self.ball_vel.len().max(bumper_min).min(max_power * 1.1);
                        self.ball_vel = n * cur_s;
                        b.pulse = 1.0;
                        hit_bumper_pos = Some(b.pos + n * b.radius);
                    }
                }

                // Sand bunker drag
                let sand_here = self.holes[self.hole_idx].sands.iter().any(|s| s.contains(self.ball_pos));
                if sand_here {
                    in_sand = true;
                }
                let f = if sand_here { sand_fric } else { friction };
                let decay = (1.0 - f * sub_dt).max(0.0);
                self.ball_vel *= decay;

                // Subtle gravitational pull near cup lip & capture check!
                let to_cup = cup - self.ball_pos;
                let cd = to_cup.len();
                if cd < hole_r * 1.18 {
                    let spd = self.ball_vel.len();
                    if cd > 1.0 {
                        self.ball_vel += (to_cup / cd) * (340.0 * sub_dt);
                    }
                    if cd < hole_r * 0.78 && spd < capture_spd {
                        sank = true;
                        break;
                    }
                }

                // Water hazard check (safe if riding on a wooden bridge!)
                let on_bridge = self.holes[self.hole_idx].bridges.iter().any(|br| br.contains(self.ball_pos));
                let in_water = !on_bridge && self.holes[self.hole_idx].waters.iter().any(|wt| wt.contains(self.ball_pos));
                if in_water {
                    splashed = true;
                    break;
                }
            }

            // Trigger audio & particle feedback outside substep loop
            if hit_wall_spd > 40.0 {
                let g = (hit_wall_spd / 520.0).clamp(0.18, 0.72);
                let p = 0.90 + self.rng.f32() * 0.22;
                self.queue_sfx(self.assets.sfx_wall, g, p, 0.0);
                self.ball_squash = (self.ball_squash + 0.16).min(0.28);
                self.cam_shake = (self.cam_shake + g * 2.5).min(4.5);
                self.spawn_burst(self.ball_pos, 4, Color::from_hex(0xF6DEC1), 65.0, false);
            }
            if let Some(bp) = hit_bumper_pos {
                let p = 0.95 + self.rng.f32() * 0.15;
                self.queue_sfx(self.assets.sfx_bumper, 0.78, p, 0.0);
                self.cam_shake = (self.cam_shake + 4.0).min(6.0);
                self.spawn_burst(bp, 10, Color::from_hex(0xFFE588), 140.0, true);
            }
            if on_boost && self.boost_sfx_cd <= 0.0 {
                self.boost_sfx_cd = 0.35;
                self.queue_sfx(self.assets.sfx_boost, 0.58, 1.0, 0.0);
                self.spawn_burst(self.ball_pos, 6, Color::from_hex(0xFFF099), 110.0, true);
            }
            if in_sand && self.ball_vel.len() > 35.0 && self.sand_sfx_cd <= 0.0 {
                self.sand_sfx_cd = 0.22;
                let p = 0.92 + self.rng.f32() * 0.16;
                self.queue_sfx(self.assets.sfx_sand, 0.42, p, 0.0);
                self.spawn_burst(self.ball_pos, 4, Color::from_hex(0xEFE0AE), 55.0, false);
            }

            if splashed {
                self.queue_sfx(self.assets.sfx_splash, 0.75, 1.0, 0.0);
                let sp = self.ball_pos;
                self.ripples.push(Ripple2D {
                    pos: sp,
                    r_start: 8.0,
                    r_end: 58.0,
                    life: 0.75,
                    max_life: 0.75,
                    color: Color::from_hex(0xE8FAFF),
                });
                self.spawn_burst(sp, 10, Color::from_hex(0xB8F0FA), 120.0, false);
                self.ball_pos = self.last_safe_pos;
                self.ball_vel = Vec2::ZERO;
                self.strokes += 1;
                self.show_toast("WATER HAZARD! +1 STROKE", Color::from_hex(0xF6C36B), 2.0);
            } else if sank {
                self.ball_vel = Vec2::ZERO;
                self.phase = Phase::HoleClear;
                self.hole_scores.push(self.strokes);
                self.queue_sfx(self.assets.sfx_hole, 0.85, 1.0, 0.0);
                self.spawn_burst(cup, 22, Color::from_hex(0xFFE169), 210.0, true);
                self.ripples.push(Ripple2D {
                    pos: cup,
                    r_start: 14.0,
                    r_end: 82.0,
                    life: 0.65,
                    max_life: 0.65,
                    color: Color::from_hex(0xFFE680),
                });
                let par = self.holes[self.hole_idx].par;
                let callout = if self.strokes == 1 {
                    "HOLE IN ONE!"
                } else if self.strokes + 2 <= par {
                    "EAGLE!"
                } else if self.strokes + 1 == par {
                    "BIRDIE!"
                } else if self.strokes == par {
                    "RIGHT ON PAR!"
                } else if self.strokes == par + 1 {
                    "BOGEY - NICE TRY!"
                } else {
                    "IN THE CUP!"
                };
                self.show_toast(callout, Color::from_hex(0xFFE578), 4.0);
            } else if self.ball_vel.len() < stop_spd {
                self.ball_vel = Vec2::ZERO;
            }
        }
    }

    /// Computes a smooth 1-bounce predicted trajectory preview for aiming!
    fn predict_trajectory(&self, start_vel: Vec2, data: &DataStore) -> Vec<Vec2> {
        let cfg = data.config("cozy.cfg");
        let ball_r = cfg.map(|c| c.f32("ball.radius", 13.0)).unwrap_or(13.0);
        let wall_bounce = cfg.map(|c| c.f32("ball.wall_bounce", 0.78)).unwrap_or(0.78);
        let mut pts = Vec::with_capacity(28);
        let mut p = self.ball_pos;
        let mut v = start_vel;
        let step_dt = 0.014;
        let mut bounces = 0u32;
        for step in 0..38 {
            p += v * step_dt;
            v *= (1.0 - 1.45 * step_dt).max(0.0);
            for w in &self.holes[self.hole_idx].walls {
                if let Some((n, pen)) = circle_vs_segment(p, ball_r, w.a, w.b, w.half_thick) {
                    p += n * pen;
                    let vn = v.dot(n);
                    if vn < 0.0 {
                        v -= n * ((1.0 + wall_bounce) * vn);
                        bounces += 1;
                    }
                }
            }
            for b in &self.holes[self.hole_idx].bumpers {
                let d = p - b.pos;
                let dist = d.len();
                if dist < ball_r + b.radius && dist > 1e-4 {
                    let n = d / dist;
                    p = b.pos + n * (ball_r + b.radius);
                    let vn = v.dot(n);
                    if vn < 0.0 {
                        v -= n * ((1.0 + 1.2) * vn);
                        bounces += 1;
                    }
                }
            }
            if step % 2 == 1 {
                pts.push(p);
            }
            if bounces >= 2 || v.len() < 45.0 {
                break;
            }
        }
        pts
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
        let wf = width as f32;
        let hf = height as f32;

        // Pure 2D mode: warm golden-hour sunlit meadow clear color, zero 3D overhead!
        draw.clear_color = Color::from_hex(0x6D9F5D);
        draw.sky_gradient = None;
        draw.shadows_enabled = false;
        draw.post.enabled = false;

        let shake = if self.cam_shake > 0.05 {
            Vec2::new((elapsed * 65.0).sin() * self.cam_shake, (elapsed * 78.0).cos() * self.cam_shake)
        } else {
            Vec2::ZERO
        };
        let cam = Camera2D { pos: self.cam.pos + shake, zoom: self.cam.zoom };
        let z = cam.zoom;
        let w2s = |p: Vec2| cam.world_to_screen(p, wf, hf);

        // Subtle sunlit diagonal meadow bands in the background
        for i in -12..=12 {
            if i % 2 == 0 {
                let c = w2s(Vec2::new(i as f32 * 96.0, 0.0));
                draw.ui_rect(c.x - 48.0 * z, 0.0, 96.0 * z, hf, Color::rgba(0.47, 0.68, 0.38, 0.32));
            }
        }

        let hole = &self.holes[self.hole_idx];

        // --- 1. FAIRWAY TURF (warm drop shadow + mowed checkerboard green!) ---
        for fw in &hole.fairways {
            let tl = w2s(Vec2::new(fw.x, fw.y));
            // Soft meadow drop shadow
            draw.ui_rect(tl.x + 10.0 * z, tl.y + 12.0 * z, fw.w * z, fw.h * z, Color::rgba(0.18, 0.31, 0.17, 0.42));
            // Rich outer terracotta edge trim
            draw.ui_rect(tl.x - 4.0 * z, tl.y - 4.0 * z, (fw.w + 8.0) * z, (fw.h + 8.0) * z, Color::from_hex(0x8C5332));
            // Base warm sunlit fairway
            draw.ui_rect(tl.x, tl.y, fw.w * z, fw.h * z, Color::from_hex(0x8CC76A));
            // Mowed checkerboard turf tiles!
            let tile = 48.0;
            let cols = (fw.w / tile).ceil() as i32;
            let rows = (fw.h / tile).ceil() as i32;
            for r in 0..rows {
                for c in 0..cols {
                    if (r + c) % 2 == 0 {
                        let tx = fw.x + c as f32 * tile;
                        let ty = fw.y + r as f32 * tile;
                        let tw = (fw.x + fw.w - tx).min(tile);
                        let th = (fw.y + fw.h - ty).min(tile);
                        let p = w2s(Vec2::new(tx, ty));
                        draw.ui_rect(p.x, p.y, tw * z, th * z, Color::from_hex(0x9BD376));
                    }
                }
            }
        }

        // --- 2. WATER PONDS (rendered over fairways & under bridges so interior garden ponds shine!) ---
        for wt in &hole.waters {
            let tl = w2s(Vec2::new(wt.x, wt.y));
            draw.ui_rect(tl.x - 6.0 * z, tl.y - 6.0 * z, (wt.w + 12.0) * z, (wt.h + 12.0) * z, Color::from_hex(0xD8C28F));
            draw.ui_rect(tl.x, tl.y, wt.w * z, wt.h * z, Color::from_hex(0x4DA8B8));
            draw.ui_rect(tl.x + 8.0 * z, tl.y + 8.0 * z, (wt.w - 16.0).max(0.0) * z, (wt.h - 16.0).max(0.0) * z, Color::from_hex(0x61BAC8));
            // Gentle animated water gleam lines
            let gleam_x = tl.x + (22.0 + (elapsed * 1.4).sin() * 14.0) * z;
            draw.ui_rect(gleam_x, tl.y + 22.0 * z, 42.0 * z, 4.0 * z, Color::rgba(0.92, 0.99, 1.0, 0.42));
            draw.ui_rect(gleam_x + 36.0 * z, tl.y + wt.h * z - 30.0 * z, 54.0 * z, 4.0 * z, Color::rgba(0.92, 0.99, 1.0, 0.36));
        }

        // --- 3. WOODEN BRIDGES ---
        for br in &hole.bridges {
            let tl = w2s(Vec2::new(br.x, br.y));
            draw.ui_rect(tl.x + 6.0 * z, tl.y + 8.0 * z, br.w * z, br.h * z, Color::rgba(0.12, 0.25, 0.28, 0.45));
            draw.ui_rect(tl.x, tl.y, br.w * z, br.h * z, Color::from_hex(0xBE8856));
            let planks = (br.w / 22.0).max(2.0) as i32;
            for i in 0..planks {
                let px = tl.x + i as f32 * 22.0 * z;
                draw.ui_rect(px, tl.y, 2.0 * z, br.h * z, Color::from_hex(0x8A5B34));
            }
            draw.ui_rect(tl.x, tl.y, br.w * z, 6.0 * z, Color::from_hex(0x8A5B34));
            draw.ui_rect(tl.x, tl.y + (br.h - 6.0) * z, br.w * z, 6.0 * z, Color::from_hex(0x8A5B34));
        }

        // --- 4. GOLDEN SAND BUNKERS ---
        for s in &hole.sands {
            let c = w2s(s.pos);
            draw.ui_sprite(
                self.assets.sand,
                c.x - s.rx * z,
                c.y - s.ry * z,
                s.rx * 2.0 * z,
                s.ry * 2.0 * z,
                Color::WHITE,
            );
        }

        // --- 5. CLOVER BOOST PADS ---
        for pad in &hole.boosts {
            let c = w2s(pad.pos);
            let ang = pad.dir.y.atan2(pad.dir.x);
            let pulse = 1.0 + (elapsed * 5.0).sin() * 0.04;
            draw.ui_sprite_rot(
                self.assets.boost,
                c.x,
                c.y,
                pad.size.x * z * pulse,
                pad.size.y * z * pulse,
                ang,
                Color::WHITE,
            );
        }

        // --- 6. TEE MARKER & CUP ---
        let tee_s = w2s(hole.tee);
        draw.ui_sprite_rot(self.assets.disc, tee_s.x, tee_s.y, 14.0 * z, 14.0 * z, 0.0, Color::rgba(1.0, 0.96, 0.84, 0.65));

        let cup_s = w2s(hole.cup);
        let hole_r = data.config("cozy.cfg").map(|c| c.f32("hole.radius", 21.0)).unwrap_or(21.0);
        draw.ui_sprite_rot(self.assets.hole, cup_s.x, cup_s.y, hole_r * 2.15 * z, hole_r * 2.15 * z, 0.0, Color::WHITE);

        // --- 7. TERRACOTTA WOOD-TRIMMED WALLS ---
        for w in &hole.walls {
            let sa = w2s(w.a);
            let sb = w2s(w.b);
            let sh_off = Vec2::new(4.0 * z, 6.0 * z);
            draw.ui_line(sa + sh_off, sb + sh_off, w.half_thick * 2.1 * z, Color::rgba(0.18, 0.29, 0.16, 0.45));
            draw.ui_line(sa, sb, w.half_thick * 2.0 * z, Color::from_hex(0x9E5936));
            draw.ui_line(sa, sb, w.half_thick * 1.15 * z, Color::from_hex(0xCC8252));
            // Rounded endcaps for crisp corners!
            let cap = w.half_thick * 2.0 * z;
            draw.ui_sprite_rot(self.assets.disc, sa.x, sa.y, cap, cap, 0.0, Color::from_hex(0x9E5936));
            draw.ui_sprite_rot(self.assets.disc, sb.x, sb.y, cap, cap, 0.0, Color::from_hex(0x9E5936));
            draw.ui_sprite_rot(self.assets.disc, sa.x, sa.y, cap * 0.58, cap * 0.58, 0.0, Color::from_hex(0xD69160));
            draw.ui_sprite_rot(self.assets.disc, sb.x, sb.y, cap * 0.58, cap * 0.58, 0.0, Color::from_hex(0xD69160));
        }

        // --- 8. ROTATING WOODEN WINDMILL PADDLES ---
        for wm in &hole.windmills {
            let c = w2s(wm.center);
            let dir = Vec2::new(wm.angle.cos(), wm.angle.sin());
            let sa = w2s(wm.center - dir * wm.half_len);
            let sb = w2s(wm.center + dir * wm.half_len);
            let sh = Vec2::new(5.0 * z, 7.0 * z);
            draw.ui_line(sa + sh, sb + sh, wm.half_thick * 2.1 * z, Color::rgba(0.15, 0.25, 0.14, 0.45));
            draw.ui_line(sa, sb, wm.half_thick * 2.0 * z, Color::from_hex(0x8D5631));
            draw.ui_line(sa, sb, wm.half_thick * 1.1 * z, Color::from_hex(0xD49B63));
            draw.ui_sprite_rot(self.assets.disc, c.x, c.y, 26.0 * z, 26.0 * z, 0.0, Color::from_hex(0x704124));
            draw.ui_sprite_rot(self.assets.disc, c.x, c.y, 16.0 * z, 16.0 * z, 0.0, Color::from_hex(0xF2C55A));
        }

        // --- 9. BOUNCY GARDEN TOADSTOOL BUMPERS ---
        for b in &hole.bumpers {
            let c = w2s(b.pos);
            let sc = 1.0 + (b.pulse * std::f32::consts::PI).sin() * 0.24;
            let sz = b.radius * 2.18 * z * sc;
            draw.ui_sprite_rot(self.assets.disc, c.x + 4.0 * z, c.y + 6.0 * z, sz * 0.92, sz * 0.92, 0.0, Color::rgba(0.16, 0.27, 0.15, 0.42));
            draw.ui_sprite_rot(self.assets.bumper, c.x, c.y, sz, sz, 0.0, Color::WHITE);
        }

        // --- 10. ANIMATED RIPPLES & SCENERY (Trees, Wildflowers, Lilypads) ---
        for r in &self.ripples {
            let t = 1.0 - clamp(r.life / r.max_life, 0.0, 1.0);
            let rad = lerp(r.r_start, r.r_end, t) * z;
            let c = w2s(r.pos);
            draw.ui_circle_outline(c.x, c.y, rad, 2.5 * z, 24, Color::rgba(r.color.r, r.color.g, r.color.b, 1.0 - t));
        }

        for sc in &hole.scenery {
            let c = w2s(sc.pos);
            let sway = (elapsed * 1.8 + sc.phase).sin() * 0.05;
            let sz = sc.size * z;
            let tex = match sc.kind {
                SceneryKind::Tree => self.assets.tree,
                SceneryKind::Flower => self.assets.flower,
                SceneryKind::Lilypad => self.assets.lilypad,
            };
            draw.ui_sprite_rot(tex, c.x, c.y, sz, sz, sway, Color::WHITE);
        }

        // --- 11. AIMING SLINGSHOT GUIDE & PREDICTED BOUNCE DOTS ---
        let drag_scale = data.config("cozy.cfg").map(|c| c.f32("ball.drag_scale", 4.4)).unwrap_or(4.4);
        let max_power = data.config("cozy.cfg").map(|c| c.f32("ball.max_power", 840.0)).unwrap_or(840.0);
        let preview_vel = if let Some(fa) = self.forced_aim {
            Some(fa)
        } else if self.dragging {
            let v = (self.drag_start - self.drag_cur) * drag_scale;
            if v.len() >= 32.0 {
                let s = v.len().min(max_power);
                Some(v.norm() * s)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(pvel) = preview_vel
            && self.phase == Phase::Playing
        {
            let frac = clamp(pvel.len() / max_power, 0.0, 1.0);
            let aim_col = if frac < 0.5 {
                Color::rgb(lerp(0.98, 0.98, frac * 2.0), lerp(0.96, 0.80, frac * 2.0), lerp(0.76, 0.28, frac * 2.0))
            } else {
                let u = (frac - 0.5) * 2.0;
                Color::rgb(lerp(0.98, 0.92, u), lerp(0.80, 0.36, u), lerp(0.28, 0.26, u))
            };
            let bs = w2s(self.ball_pos);
            // Pull-back indicator ring around ball
            draw.ui_circle_outline(bs.x, bs.y, (22.0 + frac * 14.0) * z, 2.5 * z, 24, Color::rgba(aim_col.r, aim_col.g, aim_col.b, 0.82));

            if self.show_aim_guide {
                let pts = self.predict_trajectory(pvel, data);
                let count = pts.len().max(1);
                for (i, pt) in pts.iter().enumerate() {
                    let spt = w2s(*pt);
                    let fade = 1.0 - (i as f32 / count as f32) * 0.72;
                    let dsz = lerp(10.0, 5.0, i as f32 / count as f32) * z;
                    draw.ui_sprite_rot(
                        self.assets.disc,
                        spt.x,
                        spt.y,
                        dsz,
                        dsz,
                        0.0,
                        Color::rgba(aim_col.r, aim_col.g, aim_col.b, fade),
                    );
                }
            }
            // Short directional arrow behind ball showing pull-back tension
            let dir = pvel.norm();
            let tail = w2s(self.ball_pos - dir * (22.0 + frac * 68.0));
            draw.ui_line(bs, tail, 4.0 * z, Color::rgba(aim_col.r, aim_col.g, aim_col.b, 0.75));
        }

        // --- 12. GOLF BALL (with soft shadow & squash-and-stretch!) ---
        let ball_r = data.config("cozy.cfg").map(|c| c.f32("ball.radius", 13.0)).unwrap_or(13.0);
        let sink_sc = (1.0 - self.ball_sink * 0.78).max(0.0);
        if sink_sc > 0.05 {
            let bs = w2s(self.ball_pos);
            let sq = self.ball_squash;
            let bw = ball_r * 2.15 * z * sink_sc * (1.0 + sq);
            let bh = ball_r * 2.15 * z * sink_sc * (1.0 - sq * 0.65);
            draw.ui_sprite_rot(
                self.assets.disc,
                bs.x + 3.5 * z,
                bs.y + 5.0 * z,
                bw * 0.88,
                bh * 0.88,
                self.ball_squash_angle,
                Color::rgba(0.14, 0.24, 0.13, 0.42 * sink_sc),
            );
            draw.ui_sprite_rot(self.assets.ball, bs.x, bs.y, bw, bh, self.ball_squash_angle, Color::WHITE);
        }

        // --- 13. PENNANT FLAG (smoothly lifts as ball approaches the cup!) ---
        let flag_y_off = self.flag_lift * 28.0 * z;
        let flag_alpha = 1.0 - self.flag_lift * 0.42;
        let flag_sz = 58.0 * z;
        draw.ui_sprite(
            self.assets.flag,
            cup_s.x - 14.0 * z,
            cup_s.y - flag_sz + 8.0 * z - flag_y_off,
            flag_sz,
            flag_sz,
            Color::rgba(1.0, 1.0, 1.0, flag_alpha),
        );

        // --- 14. PARTICLES & DRIFTING MEADOW POLLEN MOTES ---
        for p in &self.particles {
            let t = 1.0 - clamp(p.life / p.max_life, 0.0, 1.0);
            let sz = lerp(p.size_start, p.size_end, t) * z;
            let sp = w2s(p.pos);
            let tex = if p.is_star { self.assets.star } else { self.assets.disc };
            draw.ui_sprite_rot(
                tex,
                sp.x,
                sp.y,
                sz,
                sz,
                p.rot,
                Color::rgba(p.color.r, p.color.g, p.color.b, 1.0 - t * t),
            );
        }
        for (mp, ph) in &self.motes {
            let sp = w2s(*mp);
            let a = 0.22 + (ph.sin() * 0.5 + 0.5) * 0.25;
            draw.ui_sprite_rot(self.assets.disc, sp.x, sp.y, 4.5 * z, 4.5 * z, 0.0, Color::rgba(1.0, 0.97, 0.78, a));
        }

        // --- 15. F1 2D COLLIDER DEBUG OVERLAY ---
        if draw.overlay == DebugOverlay::Colliders {
            for w in &hole.walls {
                draw.ui_line(w2s(w.a), w2s(w.b), w.half_thick * 2.0 * z, Color::rgba(1.0, 0.92, 0.2, 0.45));
            }
            for b in &hole.bumpers {
                let c = w2s(b.pos);
                draw.ui_circle_outline(c.x, c.y, b.radius * z, 2.0, 20, Color::from_hex(0x48FF84));
            }
            draw.ui_circle_outline(cup_s.x, cup_s.y, hole_r * z, 2.0, 20, Color::from_hex(0x44FAFF));
        }

        // --- 16. WARM STORYBOOK PARCHMENT HUD & MENUS ---
        self.render_hud(input, draw, wf, hf);
    }

    fn render_hud(&mut self, input: &InputState, draw: &mut DrawList, w: f32, h: f32) {
        let cream = Color::rgba(0.98, 0.95, 0.87, 0.94);
        let dark_wood = Color::from_hex(0x5E3E29);
        let ink = Color::from_hex(0x3B281B);
        let sage = Color::from_hex(0x3C7A47);
        let honey = Color::from_hex(0xD69122);
        let terra = Color::from_hex(0xC94E3B);
        let muted_ink = Color::from_hex(0x6E5643);

        // Top-Right Controls
        let mlbl = if self.muted { "AUDIO OFF" } else { "AUDIO ON" };
        if draw.ui_button(input, w - 150.0, 16.0, 134.0, 36.0, mlbl) {
            self.muted = !self.muted;
        }
        if !self.paused && draw.ui_button(input, w - 294.0, 16.0, 134.0, 36.0, "MENU [ESC]") {
            self.paused = true;
        }
        if self.phase == Phase::Playing && !self.paused {
            if draw.ui_button(input, w - 428.0, 16.0, 124.0, 36.0, "RESET [R]") {
                self.load_hole(self.hole_idx);
            }
        }

        // Top-Left Course Plaque (visible during play & hole clear)
        if matches!(self.phase, Phase::Playing | Phase::HoleClear) {
            let hole = &self.holes[self.hole_idx];
            let title_line = format!("HOLE {}/{}  {}", self.hole_idx + 1, self.holes.len(), hole.name);
            let stat_line = format!("PAR {}    STROKES {}", hole.par, self.strokes);
            let (tw1, _) = draw.ui_text_measure(2.0, &title_line);
            let (tw2, _) = draw.ui_text_measure(2.0, &stat_line);
            let pw = tw1.max(tw2) + 32.0;
            draw.ui_panel(16.0, 16.0, pw, 64.0, cream, dark_wood, 3.0);
            draw.ui_text(30.0, 26.0, 2.0, sage, &title_line);
            draw.ui_text(30.0, 50.0, 2.0, ink, &stat_line);

            // Subtle bottom-left drag hint before first putt on Hole 1
            if self.phase == Phase::Playing && !self.paused && self.strokes == 0 && !self.dragging {
                let hint = "DRAG MOUSE TO AIM & PUTT   RMB CANCEL   R RESET";
                let (hw, _) = draw.ui_text_measure(2.0, hint);
                let px = (w - hw) * 0.5;
                draw.ui_panel(px - 16.0, h - 48.0, hw + 32.0, 34.0, cream, dark_wood, 2.0);
                draw.ui_text(px, h - 38.0, 2.0, muted_ink, hint);
            }
        }

        // Toast Banner (centered in the open top band during play, or hero-centered on Hole Clear)
        if self.toast_timer > 0.0 && !self.toast_text.is_empty() && !self.paused && self.phase != Phase::Title {
            let scale = if self.phase == Phase::HoleClear { 3.0 } else { 2.0 };
            let (tw, _) = draw.ui_text_measure(scale, &self.toast_text);
            let pw = tw + 26.0;
            let ph = if self.phase == Phase::HoleClear { 48.0 } else { 36.0 };
            let px = if self.phase == Phase::HoleClear {
                (w - pw) * 0.5
            } else {
                let left_edge = 368.0;
                let right_edge = w - 442.0;
                (left_edge + ((right_edge - left_edge) - pw) * 0.5).max(left_edge)
            };
            let py = if self.phase == Phase::HoleClear { h * 0.24 } else { 16.0 };
            draw.ui_panel(px, py, pw, ph, Color::rgba(0.24, 0.16, 0.11, 0.92), honey, 2.5);
            draw.ui_text(px + 13.0, py + if self.phase == Phase::HoleClear { 13.0 } else { 11.0 }, scale, self.toast_color, &self.toast_text);
        }

        // Hole Clear Card
        if self.phase == Phase::HoleClear && !self.paused {
            let bw = 420.0;
            let bh = 176.0;
            let bx = (w - bw) * 0.5;
            let by = h * 0.38;
            draw.ui_panel(bx, by, bw, bh, cream, dark_wood, 3.0);
            let par = self.holes[self.hole_idx].par;
            let diff = self.strokes as i32 - par as i32;
            let diff_str = if diff == 0 {
                "EVEN PAR".to_string()
            } else if diff < 0 {
                format!("{diff} UNDER PAR")
            } else {
                format!("+{diff} OVER PAR")
            };
            let s1 = format!("STROKES: {}   PAR: {}", self.strokes, par);
            let (w1, _) = draw.ui_text_measure(2.0, &s1);
            draw.ui_text((w - w1) * 0.5, by + 26.0, 2.0, ink, &s1);

            let (w2, _) = draw.ui_text_measure(3.0, &diff_str);
            let dcol = if diff <= 0 { sage } else { terra };
            draw.ui_text((w - w2) * 0.5, by + 58.0, 3.0, dcol, &diff_str);

            let btn_lbl = if self.hole_idx + 1 < self.holes.len() {
                "NEXT HOLE [SPACE]"
            } else {
                "VIEW SCORECARD [SPACE]"
            };
            if draw.ui_button(input, (w - 270.0) * 0.5, by + 114.0, 270.0, 42.0, btn_lbl) {
                self.next_hole();
            }
        }

        // Warm Cozy Title Card
        if self.phase == Phase::Title && !self.paused {
            let bw = 648.0;
            let bh = 250.0;
            let bx = (w - bw) * 0.5;
            let by = (h - bh) * 0.44;
            draw.ui_panel(bx, by, bw, bh, cream, dark_wood, 4.0);

            let title = "COZY GOLF";
            let (tw, _) = draw.ui_text_measure(5.0, title);
            draw.ui_text((w - tw) * 0.5 + 3.0, by + 25.0, 5.0, honey, title);
            draw.ui_text((w - tw) * 0.5, by + 22.0, 5.0, sage, title);

            let sub = "A SUNLIT 6-HOLE GARDEN MINI GOLF STROLL";
            let (sw, _) = draw.ui_text_measure(2.0, sub);
            draw.ui_text((w - sw) * 0.5, by + 76.0, 2.0, terra, sub);

            let c1 = "CLICK & DRAG TO AIM AND PUTT   RMB TO CANCEL";
            let (c1w, _) = draw.ui_text_measure(2.0, c1);
            draw.ui_text((w - c1w) * 0.5, by + 114.0, 2.0, ink, c1);

            let c2 = "BANK OFF TERRACOTTA WALLS, TOADSTOOLS & WINDMILLS";
            let (c2w, _) = draw.ui_text_measure(2.0, c2);
            draw.ui_text((w - c2w) * 0.5, by + 142.0, 2.0, muted_ink, c2);

            if draw.ui_button(input, (w - 240.0) * 0.5, by + 184.0, 240.0, 44.0, "TEE OFF [SPACE]") {
                self.start_course();
            }
        }

        // Full 6-Hole Warm Parchment Scorecard
        if self.phase == Phase::Scorecard && !self.paused {
            draw.ui_rect(0.0, 0.0, w, h, Color::rgba(0.14, 0.19, 0.12, 0.55));
            let bw = 620.0;
            let bh = 370.0;
            let bx = (w - bw) * 0.5;
            let by = (h - bh) * 0.5;
            draw.ui_panel(bx, by, bw, bh, cream, dark_wood, 4.0);

            let head = "GARDEN SCORECARD";
            let (hw, _) = draw.ui_text_measure(4.0, head);
            draw.ui_text((w - hw) * 0.5, by + 22.0, 4.0, sage, head);

            let mut total_par = 0u32;
            let mut total_str = 0u32;
            for (i, hole) in self.holes.iter().enumerate() {
                let y = by + 84.0 + i as f32 * 30.0;
                let st = self.hole_scores.get(i).copied().unwrap_or(hole.par);
                total_par += hole.par;
                total_str += st;
                draw.ui_text(bx + 36.0, y, 2.0, ink, &format!("{}  {}", i + 1, hole.name));
                draw.ui_text(bx + 370.0, y, 2.0, muted_ink, &format!("PAR {}", hole.par));
                let scol = if st <= hole.par { sage } else { terra };
                draw.ui_text(bx + 476.0, y, 2.0, scol, &format!("SCORE {}", st));
            }
            let diff = total_str as i32 - total_par as i32;
            let diff_lbl = if diff == 0 {
                "EVEN PAR!".into()
            } else if diff < 0 {
                format!("{diff} UNDER PAR!")
            } else {
                format!("+{diff} OVER PAR")
            };
            let tot = format!("TOTAL {} (PAR {})  {}", total_str, total_par, diff_lbl);
            let (tw, _) = draw.ui_text_measure(2.0, &tot);
            draw.ui_text((w - tw) * 0.5, by + 276.0, 2.0, honey, &tot);

            if draw.ui_button(input, (w - 250.0) * 0.5, by + 310.0, 250.0, 42.0, "PLAY AGAIN [SPACE]") {
                self.start_course();
            }
        }

        // Cozy Pause & Settings Menu
        if self.paused {
            draw.ui_rect(0.0, 0.0, w, h, Color::rgba(0.10, 0.14, 0.09, 0.58));
            let bw = 500.0;
            let bh = 338.0;
            let bx = (w - bw) * 0.5;
            let by = (h - bh) * 0.5;
            draw.ui_panel(bx, by, bw, bh, cream, dark_wood, 4.0);

            let head = "COZY GOLF // PAUSED";
            let (hw, _) = draw.ui_text_measure(3.0, head);
            draw.ui_text((w - hw) * 0.5, by + 22.0, 3.0, sage, head);

            let lx = bx + 28.0;
            let sx = bx + 236.0;
            let sw = 172.0;

            // Master Volume Slider
            let y1 = by + 84.0;
            draw.ui_text(lx, y1 + 4.0, 2.0, ink, "MASTER VOLUME");
            draw.ui_slider(input, sx, y1, sw, 22.0, 0.0, 1.0, &mut self.master_volume);
            draw.ui_text(bx + 420.0, y1 + 4.0, 2.0, sage, &format!("{:>3.0}%", self.master_volume * 100.0));

            // Toggles Row
            let y2 = by + 132.0;
            let aim_lbl = if self.show_aim_guide { "AIM GUIDE: ON" } else { "AIM GUIDE: OFF" };
            if draw.ui_button(input, lx, y2, 214.0, 38.0, aim_lbl) {
                self.show_aim_guide = !self.show_aim_guide;
            }
            let aud_lbl = if self.muted { "AUDIO: MUTED" } else { "AUDIO: ON" };
            if draw.ui_button(input, bx + bw - 242.0, y2, 214.0, 38.0, aud_lbl) {
                self.muted = !self.muted;
            }

            if draw.ui_button(input, lx, by + 190.0, bw - 56.0, 42.0, "RESUME ROUND [ESC]") {
                self.paused = false;
            }
            if draw.ui_button(input, lx, by + 244.0, 214.0, 38.0, "RESTART HOLE") {
                self.load_hole(self.hole_idx);
            }
            if draw.ui_button(input, bx + bw - 242.0, by + 244.0, 214.0, 38.0, "RESTART COURSE") {
                self.start_course();
            }
            if draw.ui_button(input, (w - 220.0) * 0.5, by + 290.0, 220.0, 34.0, "QUIT TO DESKTOP") {
                self.quit_requested = true;
            }
        }
    }

    pub fn save_snapshot(&self, snap: &mut Snapshot) {
        snap.set_str("game", "phase", self.phase.as_str());
        snap.set_i32("game", "hole", self.hole_idx as i32);
        snap.set_i32("game", "strokes", self.strokes as i32);
        snap.set_f32("ball", "x", self.ball_pos.x);
        snap.set_f32("ball", "y", self.ball_pos.y);
    }

    pub fn load_snapshot(&mut self, snap: &Snapshot) {
        let h = snap.get_i32("game", "hole", 0).max(0) as usize;
        if h < self.holes.len() {
            self.load_hole(h);
        }
        self.phase = match snap.get_str("game", "phase", "title") {
            "playing" => Phase::Playing,
            "hole_clear" => Phase::HoleClear,
            "scorecard" => Phase::Scorecard,
            _ => Phase::Title,
        };
        self.strokes = snap.get_i32("game", "strokes", 0).max(0) as u32;
        self.ball_pos = Vec2::new(
            snap.get_f32("ball", "x", self.ball_pos.x),
            snap.get_f32("ball", "y", self.ball_pos.y),
        );
    }
}

/// Exact 2D circle-vs-capsule segment collision returning `(push_normal, penetration_depth)`.
fn circle_vs_segment(center: Vec2, radius: f32, a: Vec2, b: Vec2, seg_half_thick: f32) -> Option<(Vec2, f32)> {
    let ab = b - a;
    let len_sq = ab.len_sq();
    let t = if len_sq > 1e-6 { clamp((center - a).dot(ab) / len_sq, 0.0, 1.0) } else { 0.0 };
    let closest = a + ab * t;
    let delta = center - closest;
    let dist_sq = delta.len_sq();
    let min_d = radius + seg_half_thick;
    if dist_sq < min_d * min_d {
        let dist = dist_sq.sqrt();
        let n = if dist > 1e-4 { delta / dist } else { Vec2::new(0.0, -1.0) };
        Some((n, min_d - dist))
    } else {
        None
    }
}

/// 6 Hand-Crafted Warm Garden Mini Golf Holes!
fn build_course() -> Vec<HoleDef> {
    vec![
        // HOLE 1: SUNNY MEADOW — Gentle angled bank fairway with wildflowers
        HoleDef {
            name: "SUNNY MEADOW",
            par: 2,
            tee: Vec2::new(-330.0, 75.0),
            cup: Vec2::new(320.0, -85.0),
            fairways: vec![
                RectZone::new(-420.0, -10.0, 560.0, 170.0),
                RectZone::new(140.0, -170.0, 280.0, 330.0),
            ],
            bridges: vec![],
            waters: vec![],
            walls: vec![
                WallSeg::new(-420.0, 160.0, 420.0, 160.0),
                WallSeg::new(-420.0, -10.0, -420.0, 160.0),
                WallSeg::new(-420.0, -10.0, 140.0, -10.0),
                WallSeg::new(140.0, -10.0, 140.0, -170.0),
                WallSeg::new(140.0, -170.0, 420.0, -170.0),
                WallSeg::new(420.0, -170.0, 420.0, 160.0),
                // Friendly 45-degree bank bumper at the outer corner!
                WallSeg::new(330.0, 160.0, 420.0, 70.0),
            ],
            bumpers: vec![Bumper { pos: Vec2::new(195.0, 75.0), radius: 22.0, pulse: 0.0 }],
            windmills: vec![],
            sands: vec![SandTrap { pos: Vec2::new(-80.0, 120.0), rx: 68.0, ry: 30.0 }],
            boosts: vec![],
            scenery: vec![
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-470.0, -140.0), size: 120.0, phase: 0.2 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-320.0, -180.0), size: 105.0, phase: 1.1 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(490.0, -140.0), size: 125.0, phase: 2.3 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(495.0, 180.0), size: 110.0, phase: 3.1 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-480.0, 210.0), size: 115.0, phase: 0.8 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-180.0, -90.0), size: 64.0, phase: 0.5 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-40.0, -135.0), size: 58.0, phase: 1.7 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(20.0, 225.0), size: 62.0, phase: 2.6 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-260.0, 220.0), size: 56.0, phase: 3.4 },
            ],
        },
        // HOLE 2: TOADSTOOL GARDEN — Bouncy red-capped mushrooms around a lotus pond
        HoleDef {
            name: "TOADSTOOL GARDEN",
            par: 3,
            tee: Vec2::new(-360.0, 110.0),
            cup: Vec2::new(350.0, -110.0),
            fairways: vec![
                RectZone::new(-440.0, -190.0, 880.0, 380.0),
            ],
            bridges: vec![],
            waters: vec![
                RectZone::new(-120.0, -60.0, 240.0, 140.0),
            ],
            walls: vec![
                WallSeg::new(-440.0, -190.0, 440.0, -190.0),
                WallSeg::new(-440.0, 190.0, 440.0, 190.0),
                WallSeg::new(-440.0, -190.0, -440.0, 190.0),
                WallSeg::new(440.0, -190.0, 440.0, 190.0),
                WallSeg::new(-440.0, -110.0, -360.0, -190.0),
                WallSeg::new(360.0, 190.0, 440.0, 110.0),
            ],
            bumpers: vec![
                Bumper { pos: Vec2::new(-215.0, -95.0), radius: 28.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(0.0, -130.0), radius: 26.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(210.0, 95.0), radius: 28.0, pulse: 0.0 },
            ],
            windmills: vec![],
            sands: vec![
                SandTrap { pos: Vec2::new(190.0, -135.0), rx: 56.0, ry: 34.0 },
            ],
            boosts: vec![],
            scenery: vec![
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(-45.0, -5.0), size: 54.0, phase: 0.4 },
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(55.0, 35.0), size: 48.0, phase: 2.1 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-510.0, -190.0), size: 118.0, phase: 0.3 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(515.0, 170.0), size: 122.0, phase: 1.9 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-510.0, 140.0), size: 64.0, phase: 1.2 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(510.0, -170.0), size: 64.0, phase: 2.8 },
            ],
        },
        // HOLE 3: WINDMILL COURTYARD — Rotating wooden paddle guarding the green!
        HoleDef {
            name: "WINDMILL COURTYARD",
            par: 3,
            tee: Vec2::new(-370.0, 0.0),
            cup: Vec2::new(350.0, 0.0),
            fairways: vec![
                RectZone::new(-440.0, -150.0, 880.0, 300.0),
            ],
            bridges: vec![],
            waters: vec![],
            walls: vec![
                WallSeg::new(-440.0, -150.0, 440.0, -150.0),
                WallSeg::new(-440.0, 150.0, 440.0, 150.0),
                WallSeg::new(-440.0, -150.0, -440.0, 150.0),
                WallSeg::new(440.0, -150.0, 440.0, 150.0),
                // Narrowing wooden gate pillars around the windmill
                WallSeg::new(0.0, -150.0, 0.0, -96.0),
                WallSeg::new(0.0, 96.0, 0.0, 150.0),
            ],
            bumpers: vec![
                Bumper { pos: Vec2::new(-185.0, -78.0), radius: 22.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(-185.0, 78.0), radius: 22.0, pulse: 0.0 },
            ],
            windmills: vec![
                Windmill { center: Vec2::new(0.0, 0.0), half_len: 82.0, half_thick: 10.0, angle: 0.4, speed: 1.45 },
            ],
            sands: vec![
                SandTrap { pos: Vec2::new(200.0, -96.0), rx: 66.0, ry: 34.0 },
                SandTrap { pos: Vec2::new(200.0, 96.0), rx: 66.0, ry: 34.0 },
            ],
            boosts: vec![],
            scenery: vec![
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-495.0, -195.0), size: 118.0, phase: 0.2 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(495.0, -195.0), size: 118.0, phase: 1.6 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-495.0, 195.0), size: 114.0, phase: 2.5 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(495.0, 195.0), size: 114.0, phase: 3.7 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(0.0, -210.0), size: 68.0, phase: 0.9 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(0.0, 210.0), size: 68.0, phase: 2.2 },
            ],
        },
        // HOLE 4: CLOVER EXPRESS — Twin pond islands connected by a wooden bridge & boost pads!
        HoleDef {
            name: "CLOVER EXPRESS",
            par: 3,
            tee: Vec2::new(-370.0, 80.0),
            cup: Vec2::new(360.0, -80.0),
            fairways: vec![
                RectZone::new(-450.0, -160.0, 310.0, 320.0),
                RectZone::new(140.0, -160.0, 310.0, 320.0),
            ],
            bridges: vec![
                RectZone::new(-145.0, -48.0, 290.0, 96.0),
            ],
            waters: vec![
                RectZone::new(-140.0, -210.0, 280.0, 420.0),
            ],
            walls: vec![
                // Left island perimeter (gap for bridge)
                WallSeg::new(-450.0, -160.0, -140.0, -160.0),
                WallSeg::new(-450.0, 160.0, -140.0, 160.0),
                WallSeg::new(-450.0, -160.0, -450.0, 160.0),
                WallSeg::new(-140.0, -160.0, -140.0, -48.0),
                WallSeg::new(-140.0, 48.0, -140.0, 160.0),
                // Right island perimeter
                WallSeg::new(140.0, -160.0, 450.0, -160.0),
                WallSeg::new(140.0, 160.0, 450.0, 160.0),
                WallSeg::new(450.0, -160.0, 450.0, 160.0),
                WallSeg::new(140.0, -160.0, 140.0, -48.0),
                WallSeg::new(140.0, 48.0, 140.0, 160.0),
                // Angled bank wall on right island
                WallSeg::new(365.0, 160.0, 450.0, 75.0),
            ],
            bumpers: vec![
                Bumper { pos: Vec2::new(265.0, 50.0), radius: 24.0, pulse: 0.0 },
            ],
            windmills: vec![],
            sands: vec![
                SandTrap { pos: Vec2::new(-285.0, -90.0), rx: 64.0, ry: 34.0 },
            ],
            boosts: vec![
                BoostPad { pos: Vec2::new(-45.0, 0.0), size: Vec2::new(72.0, 62.0), dir: Vec2::new(1.0, 0.0) },
                BoostPad { pos: Vec2::new(45.0, 0.0), size: Vec2::new(72.0, 62.0), dir: Vec2::new(1.0, 0.0) },
            ],
            scenery: vec![
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(-45.0, -130.0), size: 56.0, phase: 0.5 },
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(48.0, 128.0), size: 52.0, phase: 1.9 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-515.0, -180.0), size: 120.0, phase: 0.2 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(515.0, -180.0), size: 120.0, phase: 2.0 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-510.0, 175.0), size: 64.0, phase: 1.1 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(510.0, 175.0), size: 64.0, phase: 2.7 },
            ],
        },
        // HOLE 5: SUNLIT LABYRINTH — Cozy S-curve garden with baffles & toadstools
        HoleDef {
            name: "SUNLIT LABYRINTH",
            par: 4,
            tee: Vec2::new(-375.0, 130.0),
            cup: Vec2::new(375.0, -130.0),
            fairways: vec![
                RectZone::new(-450.0, -200.0, 900.0, 400.0),
            ],
            bridges: vec![],
            waters: vec![],
            walls: vec![
                WallSeg::new(-450.0, -200.0, 450.0, -200.0),
                WallSeg::new(-450.0, 200.0, 450.0, 200.0),
                WallSeg::new(-450.0, -200.0, -450.0, 200.0),
                WallSeg::new(450.0, -200.0, 450.0, 200.0),
                // S-curve internal garden baffles
                WallSeg::new(-165.0, 200.0, -165.0, -65.0),
                WallSeg::new(165.0, -200.0, 165.0, 65.0),
                // Corner bank chamfers
                WallSeg::new(-450.0, -120.0, -370.0, -200.0),
                WallSeg::new(370.0, 200.0, 450.0, 120.0),
            ],
            bumpers: vec![
                Bumper { pos: Vec2::new(-305.0, -105.0), radius: 22.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(0.0, 0.0), radius: 30.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(305.0, 105.0), radius: 22.0, pulse: 0.0 },
            ],
            windmills: vec![],
            sands: vec![
                SandTrap { pos: Vec2::new(0.0, -135.0), rx: 72.0, ry: 34.0 },
                SandTrap { pos: Vec2::new(0.0, 135.0), rx: 72.0, ry: 34.0 },
            ],
            boosts: vec![
                BoostPad { pos: Vec2::new(0.0, 68.0), size: Vec2::new(66.0, 56.0), dir: Vec2::new(0.0, 1.0) },
            ],
            scenery: vec![
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-515.0, -205.0), size: 118.0, phase: 0.4 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(515.0, -205.0), size: 118.0, phase: 1.5 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-515.0, 205.0), size: 112.0, phase: 2.3 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(515.0, 205.0), size: 112.0, phase: 3.1 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-512.0, 0.0), size: 64.0, phase: 0.8 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(512.0, 0.0), size: 64.0, phase: 2.4 },
            ],
        },
        // HOLE 6: GOLDEN PAVILION — Grand finale with twin windmills & trick boost lane!
        HoleDef {
            name: "GOLDEN PAVILION",
            par: 3,
            tee: Vec2::new(-385.0, 0.0),
            cup: Vec2::new(375.0, 0.0),
            fairways: vec![
                RectZone::new(-455.0, -185.0, 910.0, 370.0),
            ],
            bridges: vec![],
            waters: vec![
                RectZone::new(-65.0, -185.0, 130.0, 90.0),
                RectZone::new(-65.0, 95.0, 130.0, 90.0),
            ],
            walls: vec![
                WallSeg::new(-455.0, -185.0, 455.0, -185.0),
                WallSeg::new(-455.0, 185.0, 455.0, 185.0),
                WallSeg::new(-455.0, -185.0, -455.0, 185.0),
                WallSeg::new(455.0, -185.0, 455.0, 185.0),
                WallSeg::new(-455.0, -105.0, -375.0, -185.0),
                WallSeg::new(-455.0, 105.0, -375.0, 185.0),
                WallSeg::new(375.0, -185.0, 455.0, -105.0),
                WallSeg::new(375.0, 185.0, 455.0, 105.0),
            ],
            bumpers: vec![
                Bumper { pos: Vec2::new(225.0, -90.0), radius: 24.0, pulse: 0.0 },
                Bumper { pos: Vec2::new(225.0, 90.0), radius: 24.0, pulse: 0.0 },
            ],
            windmills: vec![
                Windmill { center: Vec2::new(-165.0, 0.0), half_len: 76.0, half_thick: 9.5, angle: 0.2, speed: 1.55 },
                Windmill { center: Vec2::new(155.0, 0.0), half_len: 76.0, half_thick: 9.5, angle: 1.8, speed: -1.55 },
            ],
            sands: vec![
                SandTrap { pos: Vec2::new(-275.0, -118.0), rx: 56.0, ry: 30.0 },
                SandTrap { pos: Vec2::new(-275.0, 118.0), rx: 56.0, ry: 30.0 },
            ],
            boosts: vec![
                BoostPad { pos: Vec2::new(0.0, 0.0), size: Vec2::new(82.0, 64.0), dir: Vec2::new(1.0, 0.0) },
            ],
            scenery: vec![
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(0.0, -142.0), size: 50.0, phase: 0.6 },
                Scenery { kind: SceneryKind::Lilypad, pos: Vec2::new(0.0, 142.0), size: 50.0, phase: 2.2 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-520.0, -195.0), size: 122.0, phase: 0.1 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(520.0, -195.0), size: 122.0, phase: 1.4 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(-520.0, 195.0), size: 118.0, phase: 2.7 },
                Scenery { kind: SceneryKind::Tree, pos: Vec2::new(520.0, 195.0), size: 118.0, phase: 3.5 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(-250.0, -225.0), size: 62.0, phase: 1.0 },
                Scenery { kind: SceneryKind::Flower, pos: Vec2::new(250.0, 225.0), size: 62.0, phase: 2.9 },
            ],
        },
    ]
}
