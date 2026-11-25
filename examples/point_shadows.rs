//! Point-light shadows: a night courtyard with pillars and an arch (models from the
//! mahjong_wars demo) lit by an orbiting lamp that casts shadows (`point_light_shadowed`)
//! and a fixed one that doesn't (`point_light`), for comparison.
//!
//!     cargo run --release --example point_shadows
//!     cargo run --release --example point_shadows -- --headless --frames 90 --snap shadows.png
//!
//! Keys: Space pauses the lamp, S toggles its shadows (`--plain` starts without), Esc quits.

use dullahan::App;
use dullahan::dev::AppConfig;
use dullahan::geom::HeightField;
use dullahan::math::{Color, Quat, Transform, Vec2, Vec3};
use dullahan::platform::Key;

fn main() {
    let mut cfg = AppConfig::from_args("Dullahan - point-light shadows", 1280, 720);
    cfg.asset_root = concat!(env!("CARGO_MANIFEST_DIR"), "/demos/mahjong_wars/assets").into();
    cfg.sock_path = None;
    let mut app = App::new(cfg).expect("init");

    let floor = HeightField {
        width: 2,
        depth: 2,
        world_size: Vec3::new(40.0, 1.0, 40.0),
        origin: Vec3::ZERO,
        heights: vec![0.0; 4],
    };
    let floor = app.create_terrain_mesh(&floor, None, 16, 16);
    let pillar = app.load_mesh("models/pillar.glb");
    let arch = app.load_mesh("models/gate_arch.glb");

    {
        let d = &mut app.draw;
        d.camera.pos = Vec3::new(0.0, 9.0, 15.0);
        d.camera.target = Vec3::new(0.0, 0.8, 0.0);
        d.sun_color = Color::rgb(0.05, 0.06, 0.09);
        d.sky_color = Color::rgb(0.05, 0.06, 0.10);
        d.ground_color = Color::rgb(0.02, 0.02, 0.03);
        d.clear_color = Color::rgb(0.02, 0.025, 0.04);
        d.fog_color = d.clear_color;
        d.fog_start = 60.0;
        d.fog_end = 120.0;
        d.shadows_enabled = false; // the (moon-dim) sun casts none; only the lamp does
        d.post.enabled = false;
    }

    let plain = std::env::args().any(|a| a == "--plain"); // start without shadows
    let (mut t, mut paused, mut shadows) = (0.0f32, false, !plain);
    while app.begin_frame() {
        if app.input.key_pressed(Key::Escape) {
            break;
        }
        if app.input.key_pressed(Key::Space) {
            paused = !paused;
        }
        if app.input.key_pressed(Key::S) {
            shadows = !shadows;
        }
        if !paused {
            t += app.dt;
        }
        let d = &mut app.draw;
        d.mesh(floor, Transform::from_pos(Vec3::ZERO));
        for i in 0..6 {
            let a = i as f32 / 6.0 * std::f32::consts::TAU;
            d.mesh(pillar, Transform { pos: Vec3::new(a.cos() * 5.0, 0.0, a.sin() * 5.0), rot: Quat::IDENTITY, scale: Vec3::new(1.0, 1.0, 1.0) });
        }
        d.mesh(arch, Transform::from_pos(Vec3::new(0.0, 0.0, -1.5)));

        // The lamp circles inside the ring of pillars, so their shadows sweep outwards.
        let lamp = Vec3::new((t * 0.6).cos() * 2.2, 2.4, (t * 0.6).sin() * 2.2);
        let warm = Color::rgb(1.0, 0.78, 0.5);
        if shadows {
            d.point_light_shadowed(lamp, warm, 16.0, 3.0);
        } else {
            d.point_light(lamp, warm, 16.0, 3.0);
        }
        d.point_light(Vec3::new(9.0, 1.5, 6.0), Color::rgb(0.5, 0.7, 1.0), 7.0, 1.5);
        // Lamp marker: an unlit billboard. (A `solid_box` casts shadows, and a caster around the
        // light would shadow everything.)
        d.billboard(None, lamp, Vec2::new(0.3, 0.3), warm);
        app.end_frame();
    }
}
