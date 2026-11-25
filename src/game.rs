//! NEON RUSH — Synthwave Endless Cyber-Runner showcase built on Dullahan Framework.
//!
//! Flat, data-oriented architecture: one `GameState`, plain `Vec`s / `Pool<T>`,
//! linear `update()` -> `render()`, 100% live-tunable via `data/runner.cfg`.

use crate::dullahan::App;
use crate::dullahan::assets::{Assets, MeshHandle, SoundHandle, TexHandle};
use crate::dullahan::audio::{AudioBus, PlayParams, VoiceId};
use crate::dullahan::data::{DataStore, Snapshot};
use crate::dullahan::gfx::{DrawList, PostSettings};
use crate::dullahan::math::{Aabb, Color, Quat, Rng, Transform, Vec2, Vec3, clamp, lerp};
use crate::dullahan::platform::{InputState, Key};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Title,
    Playing,
    GameOver,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Playing => "playing",
            Self::GameOver => "gameover",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ObstacleKind {
    Train,
    MovingTrain,
    Ramp,
    LowBarrier,
    HighGate,
    BoostPad,
}

struct Obstacle {
    kind: ObstacleKind,
    lane: i32,
    z: f32,
    extra_speed: f32,
    near_miss_awarded: bool,
    triggered: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PickupKind {
    Coin,
    Shield,
}

struct Pickup {
    kind: PickupKind,
    lane: i32,
    y: f32,
    z: f32,
    collected: bool,
}

struct Particle {
    pos: Vec3,
    vel: Vec3,
    size: f32,
    life: f32,
    max_life: f32,
    color: Color,
}

struct BannerMsg {
    text: String,
    color: Color,
    timer: f32,
}

struct SoundEvent {
    sound: SoundHandle,
    params: PlayParams,
}

pub struct GameAssets {
    runner: MeshHandle,
    train: MeshHandle,
    ramp: MeshHandle,
    barrier_low: MeshHandle,
    gate_high: MeshHandle,
    arch: MeshHandle,
    coin: MeshHandle,
    shield: MeshHandle,
    shield_aura: MeshHandle,
    boost_pad: MeshHandle,
    mountain: MeshHandle,
    sun_tex: TexHandle,
    coin_ui_tex: TexHandle,
    shield_ui_tex: TexHandle,
    spark_tex: TexHandle,
    music_loop: SoundHandle,
    sfx_coin: SoundHandle,
    sfx_jump: SoundHandle,
    sfx_slide: SoundHandle,
    sfx_lane: SoundHandle,
    sfx_boost: SoundHandle,
    sfx_nearmiss: SoundHandle,
    sfx_crash: SoundHandle,
}

impl GameAssets {
    pub fn load(app: &mut App) -> Self {
        app.data.load_config("runner.cfg");
        Self {
            runner: app.load_mesh("models/runner.glb"),
            train: app.load_mesh("models/train.glb"),
            ramp: app.load_mesh("models/ramp.glb"),
            barrier_low: app.load_mesh("models/barrier_low.glb"),
            gate_high: app.load_mesh("models/gate_high.glb"),
            arch: app.load_mesh("models/arch.glb"),
            coin: app.load_mesh("models/coin.glb"),
            shield: app.load_mesh("models/shield.glb"),
            shield_aura: app.load_mesh("models/shield_aura.glb"),
            boost_pad: app.load_mesh("models/boost_pad.glb"),
            mountain: app.load_mesh("models/mountain.glb"),
            sun_tex: app.load_svg("ui/sun.svg", 256, 256),
            coin_ui_tex: app.load_svg("ui/coin.svg", 64, 64),
            shield_ui_tex: app.load_svg("ui/shield.svg", 64, 64),
            spark_tex: app.load_svg("ui/spark.svg", 64, 64),
            music_loop: app.load_sound("sfx/music_loop.wav"),
            sfx_coin: app.load_sound("sfx/coin.wav"),
            sfx_jump: app.load_sound("sfx/jump.wav"),
            sfx_slide: app.load_sound("sfx/slide.wav"),
            sfx_lane: app.load_sound("sfx/lane.wav"),
            sfx_boost: app.load_sound("sfx/boost.wav"),
            sfx_nearmiss: app.load_sound("sfx/nearmiss.wav"),
            sfx_crash: app.load_sound("sfx/crash.wav"),
        }
    }
}

pub struct GameState {
    pub phase: Phase,
    pub assets: GameAssets,
    music_voice: Option<VoiceId>,
    muted: bool,

    // Player movement & feel
    pub lane: i32,
    pub player_x: f32,
    pub player_y: f32,
    pub vel_y: f32,
    pub grounded: bool,
    coyote_timer: f32,
    jump_buf_timer: f32,
    slide_timer: f32,
    bank_angle: f32,
    cam_roll: f32,
    cam_x: f32,
    cam_y: f32,
    cam_fov: f32,

    // Progression & juice
    pub speed: f32,
    pub distance: f32,
    pub score: f32,
    pub best_score: f32,
    pub coins: u32,
    pub combo: u32,
    combo_timer: f32,
    pub has_shield: bool,
    boost_timer: f32,
    hitstop_timer: f32,
    shake_trauma: f32,
    flash_timer: f32,
    flash_color: Color,
    banner: Option<BannerMsg>,

    // Scrolling world entities
    world_scroll: f32,
    next_spawn_z: f32,
    pattern_count: u32,
    obstacles: Vec<Obstacle>,
    pickups: Vec<Pickup>,
    particles: Vec<Particle>,
    trail_accum: f32,
    rng: Rng,
    queued_sounds: Vec<SoundEvent>,
}

impl GameState {
    pub fn new(app: &mut App) -> Self {
        let assets = GameAssets::load(app);
        let music_voice = app.play_loop(assets.music_loop, 0.42, AudioBus::Music);

        let mut s = Self {
            phase: Phase::Title,
            assets,
            music_voice,
            muted: false,
            lane: 0,
            player_x: 0.0,
            player_y: 0.0,
            vel_y: 0.0,
            grounded: true,
            coyote_timer: 0.0,
            jump_buf_timer: 0.0,
            slide_timer: 0.0,
            bank_angle: 0.0,
            cam_roll: 0.0,
            cam_x: 0.0,
            cam_y: 4.5,
            cam_fov: 60.0,
            speed: 26.0,
            distance: 0.0,
            score: 0.0,
            best_score: 0.0,
            coins: 0,
            combo: 0,
            combo_timer: 0.0,
            has_shield: false,
            boost_timer: 0.0,
            hitstop_timer: 0.0,
            shake_trauma: 0.0,
            flash_timer: 0.0,
            flash_color: Color::WHITE,
            banner: None,
            world_scroll: 0.0,
            next_spawn_z: -55.0,
            pattern_count: 0,
            obstacles: Vec::new(),
            pickups: Vec::new(),
            particles: Vec::new(),
            trail_accum: 0.0,
            rng: Rng::new(0x51A7_E808_2026),
            queued_sounds: Vec::new(),
        };
        // Seed initial eye-catching showcase wave down the highway!
        s.seed_opening_scene(&app.data);
        if let Some(snap) = app.startup_snapshot() {
            s.load_snapshot(&snap);
        }
        s
    }

    pub fn start_run(&mut self, data: &DataStore) {
        let cfg = data.config("runner.cfg");
        self.phase = Phase::Playing;
        self.lane = 0;
        self.player_x = 0.0;
        self.player_y = 0.0;
        self.vel_y = 0.0;
        self.grounded = true;
        self.coyote_timer = 0.0;
        self.jump_buf_timer = 0.0;
        self.slide_timer = 0.0;
        self.bank_angle = 0.0;
        self.speed = cfg.map(|c| c.f32("speed.start", 26.0)).unwrap_or(26.0);
        self.distance = 0.0;
        self.score = 0.0;
        self.coins = 0;
        self.combo = 0;
        self.combo_timer = 0.0;
        self.has_shield = false;
        self.boost_timer = 0.0;
        self.hitstop_timer = 0.0;
        self.shake_trauma = 0.0;
        self.flash_timer = 0.0;
        self.banner = Some(BannerMsg {
            text: "GO! NEON RUSH!".into(),
            color: Color::from_hex(0x30F8FF),
            timer: 1.4,
        });
        self.obstacles.clear();
        self.pickups.clear();
        self.particles.clear();
        self.pattern_count = 0;
        self.rng = Rng::new(0x51A7_E808_2026 ^ ((self.world_scroll * 100.0) as u64));
        self.seed_opening_scene(data);
        self.queue_sfx(self.assets.sfx_boost, 0.55, 1.0, 0.0);
    }

    fn seed_opening_scene(&mut self, _data: &DataStore) {
        self.obstacles.clear();
        self.pickups.clear();

        // Iconic opening vista: glowing coin line ahead, trains + ramp + barrier + laser gate!
        for i in 0..5 {
            self.pickups.push(Pickup {
                kind: PickupKind::Coin,
                lane: 0,
                y: 0.9,
                z: -14.0 - i as f32 * 3.0,
                collected: false,
            });
        }
        // Left lane cyber-train + ramp leading to its roof with gold coin arc!
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::Ramp,
            lane: -1,
            z: -32.0,
            extra_speed: 0.0,
            near_miss_awarded: false,
            triggered: false,
        });
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::Train,
            lane: -1,
            z: -41.0,
            extra_speed: 0.0,
            near_miss_awarded: false,
            triggered: false,
        });
        for i in 0..4 {
            self.pickups.push(Pickup {
                kind: PickupKind::Coin,
                lane: -1,
                y: 3.6,
                z: -36.5 - i as f32 * 2.8,
                collected: false,
            });
        }
        // Center lane jump-over barrier + boost pad ahead
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::LowBarrier,
            lane: 0,
            z: -34.0,
            extra_speed: 0.0,
            near_miss_awarded: false,
            triggered: false,
        });
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::BoostPad,
            lane: 0,
            z: -46.0,
            extra_speed: 0.0,
            near_miss_awarded: false,
            triggered: false,
        });
        // Right lane high slide-under laser gate + Holo-Shield behind it!
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::HighGate,
            lane: 1,
            z: -28.0,
            extra_speed: 0.0,
            near_miss_awarded: false,
            triggered: false,
        });
        self.pickups.push(Pickup {
            kind: PickupKind::Shield,
            lane: 1,
            y: 1.0,
            z: -38.0,
            collected: false,
        });
        // Distant oncoming express train in right lane
        self.obstacles.push(Obstacle {
            kind: ObstacleKind::MovingTrain,
            lane: 1,
            z: -74.0,
            extra_speed: 14.0,
            near_miss_awarded: false,
            triggered: false,
        });

        self.next_spawn_z = -68.0;
        while self.next_spawn_z > -165.0 {
            self.spawn_next_pattern();
        }
    }

    fn spawn_next_pattern(&mut self) {
        let z0 = self.next_spawn_z;
        let kind = (self.rng.usize(5) + (self.pattern_count as usize % 2)) % 5;
        self.pattern_count += 1;

        match kind {
            // Pattern 0: Ramp -> Train Roof Coin Run + side laser gate
            0 => {
                let ramp_lane = (self.rng.usize(3) as i32) - 1;
                let other_lane = if ramp_lane == 0 { 1 } else { 0 };
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::Ramp,
                    lane: ramp_lane,
                    z: z0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::Train,
                    lane: ramp_lane,
                    z: z0 - 9.0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::Train,
                    lane: ramp_lane,
                    z: z0 - 21.0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                for i in 0..6 {
                    self.pickups.push(Pickup {
                        kind: PickupKind::Coin,
                        lane: ramp_lane,
                        y: 3.65,
                        z: z0 - 5.0 - i as f32 * 3.0,
                        collected: false,
                    });
                }
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::HighGate,
                    lane: other_lane,
                    z: z0 - 11.0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                self.next_spawn_z = z0 - 34.0;
            }
            // Pattern 1: Twin Oncoming Cyber-Trains (leaves 1 guaranteed open escape lane + coins!)
            1 => {
                let safe_lane = (self.rng.usize(3) as i32) - 1;
                for l in -1..=1 {
                    if l == safe_lane {
                        for i in 0..5 {
                            self.pickups.push(Pickup {
                                kind: PickupKind::Coin,
                                lane: l,
                                y: 0.9,
                                z: z0 - i as f32 * 3.2,
                                collected: false,
                            });
                        }
                        if self.rng.f32() < 0.35 {
                            self.obstacles.push(Obstacle {
                                kind: ObstacleKind::BoostPad,
                                lane: l,
                                z: z0 - 18.0,
                                extra_speed: 0.0,
                                near_miss_awarded: false,
                                triggered: false,
                            });
                        }
                    } else {
                        self.obstacles.push(Obstacle {
                            kind: ObstacleKind::MovingTrain,
                            lane: l,
                            z: z0 - self.rng.range(4.0, 14.0),
                            extra_speed: self.rng.range(12.0, 20.0),
                            near_miss_awarded: false,
                            triggered: false,
                        });
                    }
                }
                self.next_spawn_z = z0 - 30.0;
            }
            // Pattern 2: Jump-Over Barrier + High Slide-Under Laser Slalom
            2 => {
                let first_l = (self.rng.usize(3) as i32) - 1;
                for l in -1..=1 {
                    let k = if l == first_l { ObstacleKind::HighGate } else { ObstacleKind::LowBarrier };
                    self.obstacles.push(Obstacle {
                        kind: k,
                        lane: l,
                        z: z0,
                        extra_speed: 0.0,
                        near_miss_awarded: false,
                        triggered: false,
                    });
                }
                // Jumping coin arc over center barrier
                let arc_lane = if first_l == 0 { 1 } else { 0 };
                for i in 0..5 {
                    let t = (i as f32 - 2.0) / 2.0;
                    let arc_y = 0.9 + (1.0 - t * t) * 1.65;
                    self.pickups.push(Pickup {
                        kind: PickupKind::Coin,
                        lane: arc_lane,
                        y: arc_y,
                        z: z0 + 4.0 - i as f32 * 2.0,
                        collected: false,
                    });
                }
                self.next_spawn_z = z0 - 22.0;
            }
            // Pattern 3: Diagonal Weave with Boost Pad & Shield
            3 => {
                let dir = if self.rng.f32() < 0.5 { 1 } else { -1 };
                for step in 0..3 {
                    let l = (-1 + step) * dir;
                    self.obstacles.push(Obstacle {
                        kind: if step == 1 { ObstacleKind::HighGate } else { ObstacleKind::LowBarrier },
                        lane: -l,
                        z: z0 - step as f32 * 9.0,
                        extra_speed: 0.0,
                        near_miss_awarded: false,
                        triggered: false,
                    });
                    self.pickups.push(Pickup {
                        kind: if step == 2 && !self.has_shield { PickupKind::Shield } else { PickupKind::Coin },
                        lane: l,
                        y: 0.95,
                        z: z0 - step as f32 * 9.0,
                        collected: false,
                    });
                }
                self.next_spawn_z = z0 - 30.0;
            }
            // Pattern 4: Triple-Threat Train + Low Barrier + Boost Strip
            _ => {
                let train_l = (self.rng.usize(3) as i32) - 1;
                let boost_l = if train_l == 1 { -1 } else { train_l + 1 };
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::Train,
                    lane: train_l,
                    z: z0 - 6.0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                self.obstacles.push(Obstacle {
                    kind: ObstacleKind::BoostPad,
                    lane: boost_l,
                    z: z0 - 4.0,
                    extra_speed: 0.0,
                    near_miss_awarded: false,
                    triggered: false,
                });
                for i in 0..4 {
                    self.pickups.push(Pickup {
                        kind: PickupKind::Coin,
                        lane: boost_l,
                        y: 0.9,
                        z: z0 - 8.0 - i as f32 * 2.8,
                        collected: false,
                    });
                }
                self.next_spawn_z = z0 - 26.0;
            }
        }
    }

    pub fn trigger_jump(&mut self, data: &DataStore) {
        let buf = data.config("runner.cfg").map(|c| c.f32("player.jump_buf", 0.13)).unwrap_or(0.13);
        self.jump_buf_timer = buf;
    }

    pub fn trigger_slide(&mut self, data: &DataStore) {
        let cfg = data.config("runner.cfg");
        let dur = cfg.map(|c| c.f32("player.slide_dur", 0.62)).unwrap_or(0.62);
        let fast_fall = cfg.map(|c| c.f32("player.fast_fall", 58.0)).unwrap_or(58.0);
        if !self.grounded && self.player_y > 0.25 {
            self.vel_y = -fast_fall * 0.45;
        }
        self.slide_timer = dur;
        self.queue_sfx(self.assets.sfx_slide, 0.65, 1.0, 0.0);
        self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.25, 0.3), 8, Color::from_hex(0xFF38A8), 4.5);
    }

    pub fn trigger_lane(&mut self, delta: i32) {
        let next = (self.lane + delta).clamp(-1, 1);
        if next != self.lane {
            self.lane = next;
            self.bank_angle = -(delta as f32) * 0.42;
            let pitch = 0.95 + self.rng.f32() * 0.15;
            self.queue_sfx(self.assets.sfx_lane, 0.55, pitch, (next as f32) * 0.35);
            self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.3, 0.4), 6, Color::from_hex(0x30F8FF), 3.5);
        }
    }

    pub fn trigger_boost(&mut self, data: &DataStore) {
        let dur = data.config("runner.cfg").map(|c| c.f32("speed.boost_dur", 2.2)).unwrap_or(2.2);
        self.boost_timer = dur;
        self.shake_trauma = (self.shake_trauma + 0.38).min(1.0);
        self.flash(Color::from_hex(0x88FFFF), 0.16);
        self.banner = Some(BannerMsg {
            text: "OVERDRIVE BOOST!".into(),
            color: Color::from_hex(0x48FF92),
            timer: 1.2,
        });
        self.queue_sfx(self.assets.sfx_boost, 0.80, 1.0, 0.0);
        self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.5, 0.4), 18, Color::from_hex(0x48FF92), 8.0);
    }

    pub fn trigger_crash(&mut self) {
        if self.has_shield {
            self.has_shield = false;
            self.hitstop_timer = 0.09;
            self.shake_trauma = 0.85;
            self.flash(Color::from_hex(0x44FFFF), 0.25);
            self.banner = Some(BannerMsg {
                text: "SHIELD SAVED YOU!".into(),
                color: Color::from_hex(0x30F8FF),
                timer: 1.4,
            });
            self.queue_sfx(self.assets.sfx_crash, 0.65, 1.45, 0.0);
            self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.9, -0.4), 26, Color::from_hex(0x30F8FF), 11.0);
            // Clear the immediate colliding obstacle so the player rides cleanly through!
            self.obstacles.retain(|o| !(o.lane == self.lane && o.z.abs() < 7.5));
            return;
        }
        self.phase = Phase::GameOver;
        self.hitstop_timer = 0.12;
        self.shake_trauma = 1.0;
        self.flash(Color::from_hex(0xFF3870), 0.35);
        if self.score > self.best_score {
            self.best_score = self.score;
        }
        self.queue_sfx(self.assets.sfx_crash, 0.90, 1.0, 0.0);
        self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.9, 0.0), 34, Color::from_hex(0xFF38A8), 12.0);
        self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.9, 0.0), 22, Color::from_hex(0xFFD84A), 9.0);
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

    fn spawn_burst(&mut self, origin: Vec3, count: usize, color: Color, speed: f32) {
        for _ in 0..count {
            let vx = self.rng.range(-speed, speed);
            let vy = self.rng.range(0.5, speed * 0.9);
            let vz = self.rng.range(-speed * 0.4, speed * 1.1);
            let life = self.rng.range(0.22, 0.55);
            self.particles.push(Particle {
                pos: origin,
                vel: Vec3::new(vx, vy, vz),
                size: self.rng.range(0.18, 0.48),
                life,
                max_life: life,
                color,
            });
        }
    }

    pub fn flush_audio(&mut self, app: &mut App) {
        for ev in self.queued_sounds.drain(..) {
            app.play_sound(ev.sound, ev.params);
        }
        if let Some(vid) = self.music_voice {
            let music_gain = match self.phase {
                Phase::Title => 0.30,
                Phase::Playing => if self.boost_timer > 0.0 { 0.50 } else { 0.42 },
                Phase::GameOver => 0.16,
            };
            let music_pitch = if self.boost_timer > 0.0 { 1.04 } else { 1.0 };
            app.audio.set_voice(vid, music_gain, music_pitch, 0.0);
        }
        app.audio.set_muted(self.muted);
    }

    pub fn update(&mut self, input: &InputState, data: &DataStore, dt: f32) {
        let cfg = data.config("runner.cfg");
        let lane_w = cfg.map(|c| c.f32("lane.width", 3.0)).unwrap_or(3.0);
        let switch_spd = cfg.map(|c| c.f32("lane.switch_speed", 15.5)).unwrap_or(15.5);
        let max_spd = cfg.map(|c| c.f32("speed.max", 62.0)).unwrap_or(62.0);
        let accel = cfg.map(|c| c.f32("speed.accel", 0.55)).unwrap_or(0.55);
        let boost_bonus = cfg.map(|c| c.f32("speed.boost_bonus", 18.0)).unwrap_or(18.0);
        let jump_vel = cfg.map(|c| c.f32("player.jump_vel", 14.2)).unwrap_or(14.2);
        let gravity = cfg.map(|c| c.f32("player.gravity", 39.0)).unwrap_or(39.0);
        let coyote_max = cfg.map(|c| c.f32("player.coyote", 0.11)).unwrap_or(0.11);
        let cam_fov_base = cfg.map(|c| c.f32("camera.fov_base", 60.0)).unwrap_or(60.0);
        let cam_fov_max = cfg.map(|c| c.f32("camera.fov_max", 78.0)).unwrap_or(78.0);
        let cam_roll_max = cfg.map(|c| c.f32("camera.roll", 0.11)).unwrap_or(0.11);

        if input.key_pressed(Key::M) {
            self.muted = !self.muted;
        }

        // Decay juices even during hitstop/gameover
        self.shake_trauma = (self.shake_trauma - dt * 1.8).max(0.0);
        self.flash_timer = (self.flash_timer - dt).max(0.0);
        if let Some(b) = &mut self.banner {
            b.timer -= dt;
            if b.timer <= 0.0 {
                self.banner = None;
            }
        }
        self.update_particles(dt);

        if self.hitstop_timer > 0.0 {
            self.hitstop_timer = (self.hitstop_timer - dt).max(0.0);
            return;
        }

        match self.phase {
            Phase::Title => {
                self.world_scroll += 18.0 * dt;
                self.player_x = (self.world_scroll * 0.04).sin() * 0.35;
                self.cam_x = lerp(self.cam_x, self.player_x * 0.5, (dt * 6.0).min(1.0));
                self.cam_fov = lerp(self.cam_fov, cam_fov_base, (dt * 6.0).min(1.0));
                self.emit_hover_trail(dt);
                if input.key_pressed(Key::Space) || input.key_pressed(Key::Enter) {
                    self.start_run(data);
                }
                return;
            }
            Phase::GameOver => {
                self.cam_fov = lerp(self.cam_fov, cam_fov_base - 4.0, (dt * 4.0).min(1.0));
                if input.key_pressed(Key::Space) || input.key_pressed(Key::R) || input.key_pressed(Key::Enter) {
                    self.start_run(data);
                }
                return;
            }
            Phase::Playing => {}
        }

        // --- Playing Input ---
        if input.key_pressed(Key::A) || input.key_pressed(Key::Left) {
            self.trigger_lane(-1);
        }
        if input.key_pressed(Key::D) || input.key_pressed(Key::Right) {
            self.trigger_lane(1);
        }
        if input.key_pressed(Key::W) || input.key_pressed(Key::Up) || input.key_pressed(Key::Space) {
            self.trigger_jump(data);
        }
        if input.key_pressed(Key::S) || input.key_pressed(Key::Down) {
            self.trigger_slide(data);
        }

        // Speed & Boost curve
        self.speed = (self.speed + accel * dt).min(max_spd);
        if self.boost_timer > 0.0 {
            self.boost_timer = (self.boost_timer - dt).max(0.0);
        }
        let eff_speed = self.speed + if self.boost_timer > 0.0 { boost_bonus } else { 0.0 };
        self.world_scroll += eff_speed * dt;
        self.distance += eff_speed * dt;
        let mult = 1 + (self.combo / 5).min(4);
        self.score += eff_speed * dt * mult as f32;

        if self.combo_timer > 0.0 {
            self.combo_timer -= dt;
            if self.combo_timer <= 0.0 {
                self.combo = 0;
            }
        }

        // Smooth spring-like lane movement & hoverboard banking
        let target_x = self.lane as f32 * lane_w;
        let dx = target_x - self.player_x;
        self.player_x += dx * (switch_spd * dt).min(1.0);
        let target_bank = clamp(-dx * 0.16, -0.45, 0.45);
        self.bank_angle = lerp(self.bank_angle, target_bank, (14.0 * dt).min(1.0));

        // Vertical physics + coyote + jump buffer + train roof & ramp support
        if self.slide_timer > 0.0 {
            self.slide_timer = (self.slide_timer - dt).max(0.0);
        }
        if self.grounded {
            self.coyote_timer = coyote_max;
        } else {
            self.coyote_timer = (self.coyote_timer - dt).max(0.0);
        }
        if self.jump_buf_timer > 0.0 {
            self.jump_buf_timer = (self.jump_buf_timer - dt).max(0.0);
        }

        if self.jump_buf_timer > 0.0 && self.coyote_timer > 0.0 {
            self.vel_y = jump_vel;
            self.grounded = false;
            self.coyote_timer = 0.0;
            self.jump_buf_timer = 0.0;
            self.slide_timer = 0.0;
            self.queue_sfx(self.assets.sfx_jump, 0.68, 1.0, 0.0);
            self.spawn_burst(Vec3::new(self.player_x, self.player_y + 0.1, 0.2), 10, Color::from_hex(0x30F8FF), 5.0);
        }

        self.vel_y -= gravity * dt;
        let prev_y = self.player_y;
        self.player_y += self.vel_y * dt;

        // Scroll obstacles & pickups toward camera (+Z)
        for o in &mut self.obstacles {
            o.z += (eff_speed + o.extra_speed) * dt;
        }
        for p in &mut self.pickups {
            p.z += eff_speed * dt;
        }
        self.next_spawn_z += eff_speed * dt;
        while self.next_spawn_z > -165.0 {
            self.spawn_next_pattern();
        }

        // Compute dynamic floor height under player (ground = 0.0, ramp slope, or train roof = 2.88)
        let mut floor_y = 0.0f32;
        for o in &self.obstacles {
            let ox = o.lane as f32 * lane_w;
            if (self.player_x - ox).abs() > 1.32 {
                continue;
            }
            match o.kind {
                ObstacleKind::Ramp => {
                    // Ramp spans z in [o.z - 3.2, o.z + 3.2], rising from +Z (0.0) to -Z (2.88)
                    if o.z > -3.5 && o.z < 3.5 {
                        let t = clamp((3.2 - (-o.z)) / 6.4, 0.0, 1.0);
                        // When player (at z=0) moves from front (+3.2 -> o.z=-3.2) to back (-3.2 -> o.z=+3.2):
                        let progress = clamp((o.z + 3.2) / 6.4, 0.0, 1.0);
                        let _ = t;
                        let ramp_h = progress * 2.90;
                        if prev_y + 0.75 >= ramp_h {
                            floor_y = floor_y.max(ramp_h);
                        }
                    }
                }
                ObstacleKind::Train | ObstacleKind::MovingTrain => {
                    if o.z.abs() < 6.0 && prev_y >= 2.25 {
                        floor_y = floor_y.max(2.88);
                    }
                }
                _ => {}
            }
        }

        if self.player_y <= floor_y {
            let was_falling_fast = !self.grounded && self.vel_y < -18.0;
            self.player_y = floor_y;
            self.vel_y = 0.0;
            self.grounded = true;
            if was_falling_fast {
                self.shake_trauma = (self.shake_trauma + 0.28).min(1.0);
                self.spawn_burst(Vec3::new(self.player_x, floor_y + 0.15, 0.0), 12, Color::from_hex(0xFF38A8), 6.0);
            }
        } else {
            self.grounded = false;
        }

        // Player AABB (shorter when sliding!)
        let player_h = if self.slide_timer > 0.0 { 0.85 } else { 1.65 };
        let player_box = Aabb::new(
            Vec3::new(self.player_x - 0.42, self.player_y + 0.12, -0.55),
            Vec3::new(self.player_x + 0.42, self.player_y + player_h, 0.55),
        );

        // Obstacle collisions, Boost Pads & Near-Misses
        let mut crashed = false;
        let mut hit_boost = false;
        let mut near_miss_pos: Option<Vec3> = None;

        for o in &mut self.obstacles {
            let ox = o.lane as f32 * lane_w;
            match o.kind {
                ObstacleKind::Ramp => continue,
                ObstacleKind::BoostPad => {
                    if !o.triggered && (self.player_x - ox).abs() < 1.15 && o.z.abs() < 2.2 && self.player_y < 0.8 {
                        o.triggered = true;
                        hit_boost = true;
                    }
                    continue;
                }
                ObstacleKind::Train | ObstacleKind::MovingTrain => {
                    let ob = Aabb::new(
                        Vec3::new(ox - 1.05, 0.0, o.z - 5.6),
                        Vec3::new(ox + 1.05, 2.72, o.z + 5.6),
                    );
                    if player_box.intersects(ob) {
                        crashed = true;
                    } else if !o.near_miss_awarded
                        && o.z > 0.5
                        && o.z < 5.5
                        && (o.lane - self.lane).abs() == 1
                        && self.player_y < 2.5
                    {
                        o.near_miss_awarded = true;
                        near_miss_pos = Some(Vec3::new(lerp(self.player_x, ox, 0.5), self.player_y + 1.0, 0.0));
                    }
                }
                ObstacleKind::LowBarrier => {
                    let ob = Aabb::new(
                        Vec3::new(ox - 1.08, 0.0, o.z - 0.30),
                        Vec3::new(ox + 1.08, 0.96, o.z + 0.30),
                    );
                    if player_box.intersects(ob) {
                        crashed = true;
                    } else if !o.near_miss_awarded
                        && o.z > 0.3
                        && o.z < 2.5
                        && o.lane == self.lane
                        && self.player_y >= 0.96
                    {
                        o.near_miss_awarded = true;
                        near_miss_pos = Some(Vec3::new(self.player_x, 1.0, 0.0));
                    }
                }
                ObstacleKind::HighGate => {
                    let ob = Aabb::new(
                        Vec3::new(ox - 1.10, 1.16, o.z - 0.32),
                        Vec3::new(ox + 1.10, 2.95, o.z + 0.32),
                    );
                    if player_box.intersects(ob) {
                        crashed = true;
                    } else if !o.near_miss_awarded
                        && o.z > 0.3
                        && o.z < 2.5
                        && o.lane == self.lane
                        && self.slide_timer > 0.0
                    {
                        o.near_miss_awarded = true;
                        near_miss_pos = Some(Vec3::new(self.player_x, 1.1, 0.0));
                    }
                }
            }
        }

        if hit_boost {
            self.trigger_boost(data);
        }
        if let Some(pos) = near_miss_pos {
            self.score += 50.0;
            self.combo += 1;
            self.combo_timer = 3.0;
            self.shake_trauma = (self.shake_trauma + 0.16).min(1.0);
            self.banner = Some(BannerMsg {
                text: "CLOSE CALL! +50".into(),
                color: Color::from_hex(0xFFD84A),
                timer: 0.95,
            });
            self.queue_sfx(self.assets.sfx_nearmiss, 0.65, 1.05, 0.0);
            self.spawn_burst(pos, 10, Color::from_hex(0xFFD84A), 5.5);
        }
        if crashed {
            self.trigger_crash();
        }

        // Pickup collection (generous magnetic feel!)
        let mut collected_events: Vec<(PickupKind, Vec3)> = Vec::new();
        for p in &mut self.pickups {
            if p.collected {
                continue;
            }
            let px = p.lane as f32 * lane_w;
            let ppos = Vec3::new(px, p.y, p.z);
            let player_c = Vec3::new(self.player_x, self.player_y + 0.95, 0.0);
            if (ppos - player_c).len_sq() < 1.55 * 1.55 {
                p.collected = true;
                collected_events.push((p.kind, ppos));
            }
        }
        for (kind, pos) in collected_events {
            match kind {
                PickupKind::Coin => {
                    self.coins += 1;
                    self.combo += 1;
                    self.combo_timer = 2.8;
                    let mult = 1 + (self.combo / 5).min(4);
                    self.score += 25.0 * mult as f32;
                    let pitch = (1.0 + (self.combo.min(14) as f32) * 0.038).min(1.65);
                    self.queue_sfx(self.assets.sfx_coin, 0.58, pitch, clamp(pos.x * 0.1, -0.5, 0.5));
                    self.spawn_burst(pos, 8, Color::from_hex(0xFFD84A), 5.0);
                }
                PickupKind::Shield => {
                    self.has_shield = true;
                    self.score += 100.0;
                    self.flash(Color::from_hex(0x66FFFF), 0.16);
                    self.banner = Some(BannerMsg {
                        text: "HOLO-SHIELD ACTIVE!".into(),
                        color: Color::from_hex(0x30F8FF),
                        timer: 1.3,
                    });
                    self.queue_sfx(self.assets.sfx_boost, 0.65, 1.25, 0.0);
                    self.spawn_burst(pos, 18, Color::from_hex(0x30F8FF), 7.5);
                }
            }
        }

        // Cull items behind camera
        self.obstacles.retain(|o| o.z < 18.0);
        self.pickups.retain(|p| !p.collected && p.z < 14.0);

        // Smooth juice camera tracking (position, banking roll, speed FOV kick)
        let speed_t = clamp((eff_speed - 24.0) / 48.0, 0.0, 1.0);
        let target_fov = lerp(cam_fov_base, cam_fov_max, speed_t) + if self.boost_timer > 0.0 { 6.0 } else { 0.0 };
        self.cam_fov = lerp(self.cam_fov, target_fov, (7.0 * dt).min(1.0));
        self.cam_x = lerp(self.cam_x, self.player_x * 0.68, (9.0 * dt).min(1.0));
        let cam_h = cfg.map(|c| c.f32("camera.height", 4.5)).unwrap_or(4.5);
        self.cam_y = lerp(self.cam_y, cam_h + self.player_y * 0.72, (9.0 * dt).min(1.0));
        self.cam_roll = lerp(self.cam_roll, -self.bank_angle * (cam_roll_max / 0.42), (10.0 * dt).min(1.0));

        self.emit_hover_trail(dt);
    }

    fn emit_hover_trail(&mut self, dt: f32) {
        self.trail_accum += dt;
        let interval = if self.boost_timer > 0.0 { 0.014 } else { 0.028 };
        while self.trail_accum >= interval {
            self.trail_accum -= interval;
            let col = if self.boost_timer > 0.0 {
                Color::from_hex(0x48FF92)
            } else if self.slide_timer > 0.0 {
                Color::from_hex(0xFF38A8)
            } else {
                Color::from_hex(0x30F8FF)
            };
            for side in [-0.15f32, 0.15] {
                self.particles.push(Particle {
                    pos: Vec3::new(self.player_x + side, self.player_y + 0.18, 0.85),
                    vel: Vec3::new(self.rng.range(-0.6, 0.6), self.rng.range(0.1, 0.8), 14.0),
                    size: if self.boost_timer > 0.0 { 0.34 } else { 0.22 },
                    life: 0.24,
                    max_life: 0.24,
                    color: col,
                });
            }
        }
    }

    fn update_particles(&mut self, dt: f32) {
        for p in &mut self.particles {
            p.life -= dt;
            p.pos += p.vel * dt;
            p.vel.y -= 9.5 * dt;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    pub fn render(
        &mut self,
        input: &InputState,
        assets: &Assets,
        data: &DataStore,
        draw: &mut DrawList,
        width: u32,
        height: u32,
        elapsed: f32,
    ) {
        let cfg = data.config("runner.cfg");
        let lane_w = cfg.map(|c| c.f32("lane.width", 3.0)).unwrap_or(3.0);
        let cam_dist = cfg.map(|c| c.f32("camera.dist", 8.2)).unwrap_or(8.2);

        // Twilight Synthwave atmosphere (deep indigo zenith -> vibrant violet-magenta horizon!)
        draw.clear_color = Color::from_hex(0x16092B);
        draw.sky_gradient = Some((Color::from_hex(0x09041E), Color::from_hex(0x6B1868)));
        draw.fog_color = Color::from_hex(0x461258);
        draw.fog_start = 42.0;
        draw.fog_end = 150.0;
        draw.sun_dir = Vec3::new(-0.22, 0.58, -0.78).norm();
        draw.sun_color = Color::from_hex(0xFF9ED8);
        draw.sky_color = Color::from_hex(0x5A2E88);
        draw.ground_color = Color::from_hex(0x182848);
        draw.shadows_enabled = true;
        draw.shadow_extent = 24.0;

        // Trauma camera shake
        let shake_amt = self.shake_trauma * self.shake_trauma;
        let sx = (elapsed * 47.0).sin() * shake_amt * 0.42;
        let sy = (elapsed * 61.0).cos() * shake_amt * 0.35;

        draw.camera.pos = Vec3::new(self.cam_x + sx, self.cam_y + sy, cam_dist);
        draw.camera.target = Vec3::new(self.cam_x * 0.45, self.player_y * 0.55 + 1.55, -24.0);
        draw.camera.up = Vec3::new(self.cam_roll.sin(), self.cam_roll.cos(), 0.0);
        draw.camera.fov_deg = self.cam_fov;
        draw.camera.far = 360.0;

        // Post-processing juice
        let speed_chroma = clamp((self.speed - 34.0) / 40.0, 0.0, 0.35);
        let boost_chroma = if self.boost_timer > 0.0 { 0.55 } else { 0.0 };
        let hit_chroma = self.shake_trauma * 0.9;
        let tint = if self.flash_timer > 0.0 {
            let a = (self.flash_timer * 3.0).min(0.45);
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
            exposure: cfg.map(|c| c.f32("post.exposure", 1.04)).unwrap_or(1.04),
            contrast: cfg.map(|c| c.f32("post.contrast", 1.06)).unwrap_or(1.06),
            saturation: cfg.map(|c| c.f32("post.saturation", 1.18)).unwrap_or(1.18),
            vignette: cfg.map(|c| c.f32("post.vignette", 0.24)).unwrap_or(0.24),
            bloom_strength: cfg.map(|c| c.f32("post.bloom", 0.62)).unwrap_or(0.62)
                + if self.boost_timer > 0.0 { 0.25 } else { 0.0 },
            bloom_threshold: cfg.map(|c| c.f32("post.threshold", 0.58)).unwrap_or(0.58),
            chroma: (speed_chroma + boost_chroma + hit_chroma).min(1.2),
            time: elapsed,
            tint,
        };

        // Dynamic neon point lights (hoverboard underglow + closest train headlights)
        let board_light_col = if self.boost_timer > 0.0 {
            Color::from_hex(0x48FF92)
        } else if self.has_shield {
            Color::from_hex(0x30F8FF)
        } else {
            Color::from_hex(0xFF38A8)
        };
        draw.point_light(Vec3::new(self.player_x, self.player_y + 1.0, 0.2), board_light_col, 8.5, 2.4);

        let mut train_lights = 0;
        for o in &self.obstacles {
            if matches!(o.kind, ObstacleKind::Train | ObstacleKind::MovingTrain)
                && o.z < 5.0
                && o.z > -65.0
                && train_lights < 3
            {
                let ox = o.lane as f32 * lane_w;
                draw.point_light(Vec3::new(ox, 1.4, o.z + 6.4), Color::from_hex(0xFFE0FF), 11.0, 2.0);
                train_lights += 1;
            }
        }

        // --- GIANT SYNTHWAVE HORIZON SUNSET DISC ---
        draw.billboard(
            Some(self.assets.sun_tex),
            Vec3::new(self.cam_x * 0.22, 34.0, -210.0),
            Vec2::new(88.0, 88.0),
            Color::WHITE,
        );

        // --- 3-LANE CYBER HIGHWAY & SCROLLING OUTRUN GRID ---
        draw.solid_box(
            Transform::from_pos_scale(Vec3::new(0.0, -0.25, -75.0), Vec3::new(11.4, 0.5, 195.0)),
            Color::from_hex(0x130E29),
        );
        // Wide dark synthwave desert floor outside the highway
        draw.solid_box(
            Transform::from_pos_scale(Vec3::new(0.0, -0.45, -80.0), Vec3::new(160.0, 0.2, 210.0)),
            Color::from_hex(0x0D0720),
        );

        // Glowing neon outer rails & lane divider stripes (drawn as crisp bright 3D lines + glowing bars)
        let rail_y = 0.06;
        draw.solid_box(
            Transform::from_pos_scale(Vec3::new(-5.1, rail_y, -75.0), Vec3::new(0.18, 0.14, 190.0)),
            Color::from_hex(0x30F8FF),
        );
        draw.solid_box(
            Transform::from_pos_scale(Vec3::new(5.1, rail_y, -75.0), Vec3::new(0.18, 0.14, 190.0)),
            Color::from_hex(0x30F8FF),
        );
        for div_x in [-1.5f32, 1.5] {
            draw.debug_line(Vec3::new(div_x, 0.03, 12.0), Vec3::new(div_x, 0.03, -165.0), Color::from_hex(0xFF2E9C));
        }

        // Scrolling Outrun grid lines across both highway & outer desert
        let grid_step = 4.0f32;
        let z_off = self.world_scroll % grid_step;
        for i in 0..42 {
            let z = 12.0 - (i as f32) * grid_step + z_off;
            let fade = clamp(1.0 - (-z) / 145.0, 0.0, 1.0);
            let hc = Color::rgba(0.24, 0.92, 1.0, fade * 0.45);
            let oc = Color::rgba(1.0, 0.18, 0.64, fade * 0.42);
            draw.debug_line(Vec3::new(-5.0, 0.025, z), Vec3::new(5.0, 0.025, z), hc);
            draw.debug_line(Vec3::new(-64.0, -0.32, z), Vec3::new(-5.8, -0.32, z), oc);
            draw.debug_line(Vec3::new(5.8, -0.32, z), Vec3::new(64.0, -0.32, z), oc);
        }
        // Longitudinal outer synthwave desert grid lines
        for gx in [10.0, 18.0, 26.0, 36.0, 48.0] {
            let oc = Color::rgba(0.95, 0.16, 0.62, 0.36);
            draw.debug_line(Vec3::new(-gx, -0.32, 12.0), Vec3::new(-gx, -0.32, -155.0), oc);
            draw.debug_line(Vec3::new(gx, -0.32, 12.0), Vec3::new(gx, -0.32, -155.0), oc);
        }

        // Scrolling Giant Overhead Neon Highway Arches
        let arch_step = 30.0f32;
        let arch_off = self.world_scroll % arch_step;
        for i in 0..6 {
            let z = 10.0 - (i as f32) * arch_step + arch_off;
            if z < 14.0 && z > -165.0 {
                draw.mesh(self.assets.arch, Transform::from_pos(Vec3::new(0.0, 0.0, z)));
            }
        }

        // Scrolling Flanking Synthwave Mountains on both sides
        let mtn_step = 28.0f32;
        let mtn_off = self.world_scroll % mtn_step;
        for i in 0..7 {
            let z = 16.0 - (i as f32) * mtn_step + mtn_off;
            if z < 24.0 && z > -175.0 {
                let s_l = 1.15 + ((i * 7) % 3) as f32 * 0.28;
                let s_r = 1.25 + ((i * 5 + 1) % 3) as f32 * 0.25;
                draw.mesh(
                    self.assets.mountain,
                    Transform {
                        pos: Vec3::new(-24.0 - (i % 2) as f32 * 5.0, -0.35, z),
                        rot: Quat::from_axis_angle(Vec3::Y, (i as f32) * 0.8),
                        scale: Vec3::new(s_l, s_l * 1.1, s_l),
                    },
                );
                draw.mesh(
                    self.assets.mountain,
                    Transform {
                        pos: Vec3::new(24.0 + ((i + 1) % 2) as f32 * 5.0, -0.35, z - 12.0),
                        rot: Quat::from_axis_angle(Vec3::Y, (i as f32) * -0.7 + 1.2),
                        scale: Vec3::new(s_r, s_r * 1.15, s_r),
                    },
                );
            }
        }

        // --- OBSTACLES & PICKUPS ---
        for o in &self.obstacles {
            let ox = o.lane as f32 * lane_w;
            let mesh = match o.kind {
                ObstacleKind::Train | ObstacleKind::MovingTrain => self.assets.train,
                ObstacleKind::Ramp => self.assets.ramp,
                ObstacleKind::LowBarrier => self.assets.barrier_low,
                ObstacleKind::HighGate => self.assets.gate_high,
                ObstacleKind::BoostPad => self.assets.boost_pad,
            };
            draw.mesh(mesh, Transform::from_pos(Vec3::new(ox, 0.0, o.z)));
        }

        for p in &self.pickups {
            if p.collected {
                continue;
            }
            let px = p.lane as f32 * lane_w;
            let bob = (elapsed * 4.5 + p.z * 0.25).sin() * 0.14;
            let spin = Quat::from_axis_angle(Vec3::Y, elapsed * 3.4 + p.z * 0.12);
            let mesh = match p.kind {
                PickupKind::Coin => self.assets.coin,
                PickupKind::Shield => self.assets.shield,
            };
            draw.mesh(mesh, Transform { pos: Vec3::new(px, p.y + bob, p.z), rot: spin, scale: Vec3::ONE });
        }

        // --- ARTICULATED CYBER-RUNNER CHARACTER ---
        let hover_bob = if self.grounded { (elapsed * 9.0).sin() * 0.055 } else { 0.0 };
        let pitch = if !self.grounded {
            clamp(self.vel_y * 0.028, -0.35, 0.35)
        } else if self.slide_timer > 0.0 {
            -0.18
        } else {
            0.0
        };
        let runner_tr = Transform {
            pos: Vec3::new(self.player_x, self.player_y + hover_bob, 0.0),
            rot: Quat::from_euler_yxz(self.bank_angle * 0.32, pitch, self.bank_angle),
            scale: Vec3::ONE,
        };

        let mut overrides = Vec::new();
        if let Some(gm) = assets.mesh(self.assets.runner) {
            if let Some(body_idx) = gm.node_index("body") {
                let mut btr = gm.nodes[body_idx].transform;
                if self.slide_timer > 0.0 {
                    // Low cyber-tuck slide pose!
                    btr.scale = Vec3::new(1.08, 0.52, 1.15);
                    btr.rot = Quat::from_axis_angle(Vec3::X, -0.62);
                    btr.pos.y = 0.14;
                } else if !self.grounded {
                    // Heroic airborne grab crouch!
                    btr.rot = Quat::from_axis_angle(Vec3::X, -0.22);
                    btr.scale = Vec3::new(1.0, 0.92, 1.0);
                } else {
                    // Dynamic speed lean forward
                    btr.rot = Quat::from_axis_angle(Vec3::X, -0.14);
                }
                overrides.push((body_idx, btr));
            }
        }
        draw.mesh_nodes(self.assets.runner, runner_tr, &overrides);

        // Active Holo-Shield translucent orbital bubble around the runner (runner stays fully visible!)
        if self.has_shield {
            draw.mesh(
                self.assets.shield_aura,
                Transform {
                    pos: Vec3::new(self.player_x, self.player_y + 1.0, 0.0),
                    rot: Quat::from_euler_yxz(elapsed * 3.4, elapsed * 1.8, 0.0),
                    scale: Vec3::splat(1.18),
                },
            );
        }

        // Player collider wireframe for `dctl overlay colliders`
        let ph = if self.slide_timer > 0.0 { 0.85 } else { 1.65 };
        draw.collider_box(
            Vec3::new(self.player_x, self.player_y + ph * 0.5 + 0.06, 0.0),
            Vec3::new(0.42, ph * 0.5, 0.55),
            Color::from_hex(0x48FF92),
        );

        // --- 3D BILLBOARD PARTICLES (thrusters, coin sparks, impacts) ---
        for pt in &self.particles {
            let alpha = clamp(pt.life / pt.max_life, 0.0, 1.0);
            let s = pt.size * (0.45 + 0.55 * alpha);
            draw.billboard(
                Some(self.assets.spark_tex),
                pt.pos,
                Vec2::new(s, s),
                Color::rgba(pt.color.r, pt.color.g, pt.color.b, alpha),
            );
        }

        // --- CRISP SYNTHWAVE 2D HUD ---
        self.render_hud(input, data, draw, width as f32, height as f32);
    }

    fn render_hud(&mut self, input: &InputState, data: &DataStore, draw: &mut DrawList, w: f32, h: f32) {
        let panel_bg = Color::rgba(0.06, 0.03, 0.14, 0.84);
        let cyan = Color::from_hex(0x30F8FF);
        let magenta = Color::from_hex(0xFF38A8);
        let gold = Color::from_hex(0xFFD84A);
        let lime = Color::from_hex(0x48FF92);
        let white = Color::rgb(0.96, 0.98, 1.0);
        let dim = Color::rgb(0.72, 0.74, 0.88);

        // Top-Left: Score, Multiplier & Coins with SVG icon
        draw.ui_panel(18.0, 16.0, 274.0, 82.0, panel_bg, cyan, 2.0);
        let mult = 1 + (self.combo / 5).min(4);
        draw.ui_text(30.0, 26.0, 3.0, white, &format!("{:07.0}", self.score));
        let mult_col = if mult > 1 { magenta } else { dim };
        draw.ui_text(172.0, 28.0, 2.0, mult_col, &format!("x{mult} COMBO"));
        draw.ui_sprite(self.assets.coin_ui_tex, 30.0, 58.0, 28.0, 28.0, Color::WHITE);
        draw.ui_text(66.0, 64.0, 2.0, gold, &format!("{:03}", self.coins));
        draw.ui_text(124.0, 64.0, 2.0, dim, &format!("{:>4.0}M", self.distance));

        // Active Shield Badge (top-center)
        if self.has_shield {
            let sw = 186.0;
            draw.ui_panel((w - sw) * 0.5, 16.0, sw, 42.0, panel_bg, cyan, 2.0);
            draw.ui_sprite(self.assets.shield_ui_tex, (w - sw) * 0.5 + 10.0, 21.0, 32.0, 32.0, Color::WHITE);
            draw.ui_text((w - sw) * 0.5 + 50.0, 29.0, 2.0, cyan, "SHIELD ON");
        }

        // Top-Right: Mute toggle button + Speed Readout
        let mute_lbl = if self.muted { "AUDIO OFF" } else { "AUDIO ON" };
        if draw.ui_button(input, w - 156.0, 16.0, 138.0, 36.0, mute_lbl) {
            self.muted = !self.muted;
        }

        // Bottom-Right: Speedometer & Boost Bar
        let eff_kmh = (self.speed + if self.boost_timer > 0.0 { 18.0 } else { 0.0 }) * 3.6;
        let spd_frac = clamp((eff_kmh - 80.0) / 210.0, 0.08, 1.0);
        draw.ui_panel(w - 252.0, h - 84.0, 234.0, 68.0, panel_bg, magenta, 2.0);
        let spd_col = if self.boost_timer > 0.0 { lime } else { white };
        draw.ui_text(w - 238.0, h - 72.0, 3.0, spd_col, &format!("{:>3.0}", eff_kmh));
        draw.ui_text(w - 172.0, h - 66.0, 2.0, dim, "KM/H");
        draw.ui_bar(
            w - 238.0,
            h - 36.0,
            206.0,
            10.0,
            spd_frac,
            Color::rgba(0.18, 0.12, 0.28, 1.0),
            if self.boost_timer > 0.0 { lime } else { magenta },
        );

        // Floating Juice Callout Banner ("CLOSE CALL! +50", "OVERDRIVE BOOST!", etc.)
        if let Some(b) = &self.banner {
            let (bw, _) = draw.ui_text_measure(3.0, &b.text);
            let px = (w - bw) * 0.5;
            draw.ui_panel(px - 18.0, 76.0, bw + 36.0, 40.0, panel_bg, b.color, 2.0);
            draw.ui_text(px, 86.0, 3.0, b.color, &b.text);
        }

        // Title Overlay
        if self.phase == Phase::Title {
            let box_w = 460.0;
            let box_h = 196.0;
            let bx = (w - box_w) * 0.5;
            let by = h * 0.14;
            draw.ui_panel(bx, by, box_w, box_h, Color::rgba(0.05, 0.02, 0.13, 0.88), magenta, 3.0);

            let title = "NEON RUSH";
            let (tw, _) = draw.ui_text_measure(5.0, title);
            draw.ui_text((w - tw) * 0.5 + 3.0, by + 25.0, 5.0, magenta, title);
            draw.ui_text((w - tw) * 0.5, by + 22.0, 5.0, cyan, title);

            let sub = "SYNTHWAVE CYBER-RUNNER";
            let (sw, _) = draw.ui_text_measure(2.0, sub);
            draw.ui_text((w - sw) * 0.5, by + 70.0, 2.0, gold, sub);

            let ctrl = "A/D LANES   W/SPC JUMP   S SLIDE";
            let (cw, _) = draw.ui_text_measure(2.0, ctrl);
            draw.ui_text((w - cw) * 0.5, by + 104.0, 2.0, dim, ctrl);

            if draw.ui_button(input, (w - 200.0) * 0.5, by + 138.0, 200.0, 42.0, "RIDE NOW [SPC]") {
                self.start_run(data);
            }
        }

        // Game Over Overlay
        if self.phase == Phase::GameOver {
            let box_w = 420.0;
            let box_h = 204.0;
            let bx = (w - box_w) * 0.5;
            let by = (h - box_h) * 0.45;
            draw.ui_panel(bx, by, box_w, box_h, Color::rgba(0.08, 0.02, 0.14, 0.92), magenta, 3.0);

            let title = "WIPEOUT!";
            let (tw, _) = draw.ui_text_measure(4.0, title);
            draw.ui_text((w - tw) * 0.5, by + 22.0, 4.0, magenta, title);

            let sc = format!("SCORE {:07.0}   BEST {:07.0}", self.score, self.best_score);
            let (sw, _) = draw.ui_text_measure(2.0, &sc);
            draw.ui_text((w - sw) * 0.5, by + 74.0, 2.0, white, &sc);

            let st = format!("COINS {}   DIST {:.0}M", self.coins, self.distance);
            let (stw, _) = draw.ui_text_measure(2.0, &st);
            draw.ui_text((w - stw) * 0.5, by + 104.0, 2.0, gold, &st);

            if draw.ui_button(input, (w - 220.0) * 0.5, by + 142.0, 220.0, 42.0, "RETRY [SPACE]") {
                self.start_run(data);
            }
        }
    }

    pub fn save_snapshot(&self, snap: &mut Snapshot) {
        snap.set_str("game", "phase", self.phase.as_str());
        snap.set_i32("player", "lane", self.lane);
        snap.set_f32("player", "x", self.player_x);
        snap.set_f32("player", "y", self.player_y);
        snap.set_f32("player", "vel_y", self.vel_y);
        snap.set_f32("game", "speed", self.speed);
        snap.set_f32("game", "distance", self.distance);
        snap.set_f32("game", "score", self.score);
        snap.set_f32("game", "best_score", self.best_score);
        snap.set_i32("game", "coins", self.coins as i32);
        snap.set_bool("player", "shield", self.has_shield);
    }

    pub fn load_snapshot(&mut self, snap: &Snapshot) {
        self.phase = match snap.get_str("game", "phase", "title") {
            "playing" => Phase::Playing,
            "gameover" => Phase::GameOver,
            _ => Phase::Title,
        };
        self.lane = snap.get_i32("player", "lane", self.lane).clamp(-1, 1);
        self.player_x = snap.get_f32("player", "x", self.player_x);
        self.player_y = snap.get_f32("player", "y", self.player_y);
        self.vel_y = snap.get_f32("player", "vel_y", self.vel_y);
        self.speed = snap.get_f32("game", "speed", self.speed);
        self.distance = snap.get_f32("game", "distance", self.distance);
        self.score = snap.get_f32("game", "score", self.score);
        self.best_score = snap.get_f32("game", "best_score", self.best_score);
        self.coins = snap.get_i32("game", "coins", self.coins as i32).max(0) as u32;
        self.has_shield = snap.get_bool("player", "shield", self.has_shield);
    }
}
