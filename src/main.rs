mod dullahan;

use dullahan::App;
use dullahan::assets::{MeshHandle, SoundHandle, TexHandle};
use dullahan::audio::PlayParams;
use dullahan::data::{Pool, Snapshot};
use dullahan::geom::{HeightField, RigidBody};
use dullahan::gfx::DebugOverlay;
use dullahan::math::{Aabb, Color, Quat, Transform, Vec2, Vec3, clamp, deg_to_rad};
use dullahan::platform::{InputState, Key, MouseButton};

struct Prop {
    mesh: MeshHandle,
    pos: Vec3,
    yaw_deg: f32,
    scale: f32,
    spin_rotor: bool,
}

struct SandboxState {
    beacon_mesh: MeshHandle,
    terrain_mesh: MeshHandle,
    heightfield: HeightField,
    star_svg_tex: TexHandle,
    ping_sfx: SoundHandle,
    props: Pool<Prop>,
    ball: RigidBody,
    last_data_gen: u64,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_dist: f32,
    cam_target: Vec3,
    rotor_angle: f32,
    energy: f32,
    auto_orbit: bool,
    play_ping: bool,
}

impl SandboxState {
    fn new(app: &mut App) -> Self {
        let beacon_mesh = app.load_mesh("beacon.glb");
        let star_svg_tex = app.load_svg("star_icon.svg", 64, 64);
        let ping_sfx = app.load_sound("ping.wav");
        app.data.load_config("tuning.cfg");
        app.data.load_level("sandbox.lvl");

        // Procedural gentle rolling terrain heightfield (proves HeightField + terrain mesh builder)
        let (tw, td) = (32u32, 32u32);
        let mut samples = vec![0u16; (tw * td) as usize];
        for z in 0..td {
            for x in 0..tw {
                let fx = (x as f32 / (tw - 1) as f32 - 0.5) * 2.0;
                let fz = (z as f32 / (td - 1) as f32 - 0.5) * 2.0;
                let r2 = fx * fx + fz * fz;
                // Flat center pad around origin, gentle hills around the perimeter
                let rim = ((r2 - 0.14).max(0.0) * 1.4).min(1.0);
                let wave = 0.5 + 0.5 * (fx * 4.5).sin() * (fz * 4.5).cos();
                let h = (rim * (0.35 + 0.65 * wave)).clamp(0.0, 1.0);
                samples[(z * tw + x) as usize] = (h * 65535.0) as u16;
            }
        }
        let h16 = dullahan::formats::png::DecodedHeight16 { width: tw, height: td, samples };
        let heightfield = HeightField::from_height16(&h16, Vec3::new(40.0, 2.8, 40.0), Vec3::ZERO);
        let terrain_mesh = app.create_terrain_mesh(&heightfield, None, 40, 40);

        let mut s = Self {
            beacon_mesh,
            terrain_mesh,
            heightfield,
            star_svg_tex,
            ping_sfx,
            props: Pool::new(),
            ball: RigidBody {
                pos: Vec3::new(1.8, 3.2, 2.2),
                vel: Vec3::new(-0.8, 0.0, -0.5),
                radius: 0.42,
                restitution: 0.65,
                ..Default::default()
            },
            last_data_gen: 0,
            cam_yaw: 0.65,
            cam_pitch: 0.36,
            cam_dist: 8.8,
            cam_target: Vec3::new(0.0, 1.3, 0.0),
            rotor_angle: 0.0,
            energy: 0.72,
            auto_orbit: false,
            play_ping: false,
        };
        s.sync_level(&app.data);
        if let Some(snap) = app.startup_snapshot() {
            s.load_snapshot(&snap);
        }
        s
    }

    fn sync_level(&mut self, data: &dullahan::data::DataStore) {
        if self.last_data_gen == data.generation {
            return;
        }
        self.last_data_gen = data.generation;
        self.props.clear();
        if let Some(lvl) = data.level("sandbox.lvl") {
            for e in &lvl.entries {
                if e.kind == "prop" && e.name == "beacon" {
                    let y = self.heightfield.sample_height(e.pos.x, e.pos.z) + e.pos.y;
                    self.props.insert(Prop {
                        mesh: self.beacon_mesh,
                        pos: Vec3::new(e.pos.x, y, e.pos.z),
                        yaw_deg: e.attr_f32("yaw", 0.0),
                        scale: e.attr_f32("scale", 1.0),
                        spin_rotor: e.attr_f32("spin", 0.0) > 0.5,
                    });
                }
            }
        }
    }

    fn save_snapshot(&self, snap: &mut Snapshot) {
        snap.set_f32("camera", "yaw", self.cam_yaw);
        snap.set_f32("camera", "pitch", self.cam_pitch);
        snap.set_f32("camera", "dist", self.cam_dist);
        snap.set_vec3("camera", "target", self.cam_target);
        snap.set_f32("sim", "rotor_angle", self.rotor_angle);
        snap.set_f32("sim", "energy", self.energy);
        snap.set_vec3("ball", "pos", self.ball.pos);
        snap.set_vec3("ball", "vel", self.ball.vel);
    }

    fn load_snapshot(&mut self, snap: &Snapshot) {
        self.cam_yaw = snap.get_f32("camera", "yaw", self.cam_yaw);
        self.cam_pitch = snap.get_f32("camera", "pitch", self.cam_pitch);
        self.cam_dist = snap.get_f32("camera", "dist", self.cam_dist);
        self.cam_target = snap.get_vec3("camera", "target", self.cam_target);
        self.rotor_angle = snap.get_f32("sim", "rotor_angle", self.rotor_angle);
        self.energy = snap.get_f32("sim", "energy", self.energy);
        self.ball.pos = snap.get_vec3("ball", "pos", self.ball.pos);
        self.ball.vel = snap.get_vec3("ball", "vel", self.ball.vel);
    }
}

fn update(s: &mut SandboxState, input: &InputState, data: &dullahan::data::DataStore, dt: f32) {
    let cfg = data.config("tuning.cfg");
    let orbit_speed = cfg.map(|c| c.f32("camera.orbit_speed", 1.6)).unwrap_or(1.6);
    let rotor_speed = cfg.map(|c| c.f32("rotor.speed", 2.4)).unwrap_or(2.4);

    if input.key_pressed(Key::Space) {
        s.play_ping = true;
        s.ball.apply_impulse(Vec3::new(0.0, 9.5, 0.0));
    }
    if input.key_pressed(Key::O) {
        s.auto_orbit = !s.auto_orbit;
    }

    if input.key_down(Key::A) || input.key_down(Key::Left) {
        s.cam_yaw -= orbit_speed * dt;
        s.auto_orbit = false;
    }
    if input.key_down(Key::D) || input.key_down(Key::Right) {
        s.cam_yaw += orbit_speed * dt;
        s.auto_orbit = false;
    }
    if input.key_down(Key::W) || input.key_down(Key::Up) {
        s.cam_pitch = clamp(s.cam_pitch + 1.1 * dt, 0.08, 1.4);
    }
    if input.key_down(Key::S) || input.key_down(Key::Down) {
        s.cam_pitch = clamp(s.cam_pitch - 1.1 * dt, 0.08, 1.4);
    }
    if input.mouse_down(MouseButton::Left) {
        s.cam_yaw -= input.mouse_delta.x * 0.008;
        s.cam_pitch = clamp(s.cam_pitch + input.mouse_delta.y * 0.008, 0.08, 1.45);
        s.auto_orbit = false;
    }
    if input.scroll_delta.abs() > 0.0 {
        s.cam_dist = clamp(s.cam_dist - input.scroll_delta * 1.0, 2.5, 35.0);
    }

    if s.auto_orbit {
        s.cam_yaw += 0.35 * dt;
    }
    s.rotor_angle += rotor_speed * dt;
    s.energy = 0.55 + 0.35 * (s.rotor_angle * 0.4).sin();

    // Step flat RigidBody against HeightField + static beacon collider AABBs
    let static_boxes: Vec<Aabb> = s
        .props
        .iter()
        .map(|(_, p)| {
            let h = Vec3::new(0.9 * p.scale, 1.55 * p.scale, 0.9 * p.scale);
            Aabb::from_center_half(p.pos + Vec3::new(0.0, h.y, 0.0), h)
        })
        .collect();
    s.ball.step(Vec3::new(0.0, -22.0, 0.0), &static_boxes, Some(&s.heightfield), None, dt);
}

fn render(
    s: &mut SandboxState,
    input: &InputState,
    assets: &dullahan::assets::Assets,
    data: &dullahan::data::DataStore,
    draw: &mut dullahan::gfx::DrawList,
    width: u32,
    height: u32,
    fps: f32,
) {
    let cfg = data.config("tuning.cfg");
    let fov = cfg.map(|c| c.f32("camera.fov", 55.0)).unwrap_or(55.0);
    let sun_dir = cfg.map(|c| c.vec3("sun.dir", Vec3::new(0.62, 0.72, 0.32))).unwrap_or(Vec3::new(0.62, 0.72, 0.32));
    let ground_col = cfg.map(|c| c.color("ground.color", Color::from_hex(0x4F7D3E))).unwrap_or(Color::from_hex(0x4F7D3E));
    let shadows = cfg.map(|c| c.bool("shadows.enabled", true)).unwrap_or(true);
    let lantern_col = cfg.map(|c| c.color("lantern.color", Color::from_hex(0xFFB84D))).unwrap_or(Color::from_hex(0xFFB84D));
    let lantern_rad = cfg.map(|c| c.f32("lantern.radius", 9.0)).unwrap_or(9.0);
    let lantern_int = cfg.map(|c| c.f32("lantern.intensity", 2.2)).unwrap_or(2.2);

    let cp = s.cam_pitch.cos();
    let cam_offset = Vec3::new(
        s.cam_yaw.sin() * cp * s.cam_dist,
        s.cam_pitch.sin() * s.cam_dist,
        s.cam_yaw.cos() * cp * s.cam_dist,
    );
    draw.camera.pos = s.cam_target + cam_offset;
    draw.camera.target = s.cam_target;
    draw.camera.fov_deg = fov;
    draw.sun_dir = sun_dir.norm();
    draw.shadows_enabled = shadows;

    draw.point_light(Vec3::new(0.0, 2.55, 0.0), lantern_col, lantern_rad, lantern_int);

    // Heightfield terrain mesh + subtle reference axes
    draw.mesh_tinted(s.terrain_mesh, Transform::IDENTITY, ground_col);
    draw.debug_axes(Vec3::new(0.0, 0.04, 0.0), 1.4);

    let rotor_idx = assets.mesh_node_index(s.beacon_mesh, "rotor");
    for (_, prop) in s.props.iter() {
        let tr = Transform {
            pos: prop.pos,
            rot: Quat::from_axis_angle(Vec3::Y, deg_to_rad(prop.yaw_deg)),
            scale: Vec3::splat(prop.scale),
        };
        if prop.spin_rotor
            && let Some(ri) = rotor_idx
            && let Some(gm) = assets.mesh(prop.mesh)
        {
            let mut ntr = gm.nodes[ri].transform;
            ntr.rot = Quat::from_axis_angle(Vec3::Y, s.rotor_angle);
            draw.mesh_nodes(prop.mesh, tr, &[(ri, ntr)]);
        } else {
            draw.mesh(prop.mesh, tr);
        }
    }

    // Dynamic RigidBody cube & floating SVG star billboard above center beacon
    draw.solid_box(
        Transform { pos: s.ball.pos, rot: s.ball.rot, scale: Vec3::splat(s.ball.radius * 2.0) },
        Color::from_hex(0x3DA5E8),
    );
    draw.collider_box(s.ball.pos, Vec3::splat(s.ball.radius), Color::GREEN);
    draw.billboard(
        Some(s.star_svg_tex),
        Vec3::new(0.0, 3.65 + (s.rotor_angle * 0.8).sin() * 0.12, 0.0),
        Vec2::new(0.75, 0.75),
        Color::WHITE,
    );

    // --- 2D HUD Overlay (with live SVG sprite + interactive button + rotated icon) ---
    let panel_bg = Color::rgba(0.09, 0.11, 0.15, 0.84);
    let panel_border = Color::rgba(0.32, 0.40, 0.50, 0.90);
    let text_hi = Color::rgb(0.95, 0.97, 1.0);
    let text_dim = Color::rgb(0.68, 0.75, 0.84);

    draw.ui_panel(16.0, 16.0, 290.0, 68.0, panel_bg, panel_border, 2.0);
    draw.ui_sprite(s.star_svg_tex, 26.0, 26.0, 48.0, 48.0, Color::WHITE);
    draw.ui_text(84.0, 28.0, 2.0, text_hi, "DULLAHAN V1");
    draw.ui_text(
        84.0,
        52.0,
        2.0,
        text_dim,
        &format!("PROPS {}  GEN {}", s.props.len(), data.generation),
    );

    let w = width as f32;
    let h = height as f32;
    if draw.ui_button(input, w - 180.0, 16.0, 164.0, 36.0, "BOUNCE BALL") {
        s.ball.apply_impulse(Vec3::new(0.0, 9.5, 0.0));
        s.play_ping = true;
    }

    draw.ui_panel(w - 266.0, h - 86.0, 250.0, 70.0, panel_bg, panel_border, 2.0);
    draw.ui_text(
        w - 252.0,
        h - 74.0,
        2.0,
        text_hi,
        &format!("POWER {:>3.0}%  {:>3.0}F", s.energy * 100.0, fps.min(999.0)),
    );
    draw.ui_bar(
        w - 252.0,
        h - 44.0,
        222.0,
        14.0,
        s.energy,
        Color::rgba(0.18, 0.22, 0.28, 1.0),
        Color::from_hex(0x3CD070),
    );
}

fn main() {
    let mut app = match App::from_args("Dullahan Framework — Complete Core", 1280, 720) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[dullahan] init failed: {e}");
            std::process::exit(1);
        }
    };
    let mut state = SandboxState::new(&mut app);

    while app.begin_frame() {
        if app.input.key_pressed(Key::Escape) {
            break;
        }
        if app.input.key_pressed(Key::F1) {
            app.draw.overlay = match app.draw.overlay {
                DebugOverlay::None => DebugOverlay::Colliders,
                DebugOverlay::Colliders => DebugOverlay::Wireframe,
                DebugOverlay::Wireframe => DebugOverlay::Grid,
                DebugOverlay::Grid => DebugOverlay::None,
            };
        }

        state.sync_level(&app.data);

        app.handle_snapshots(
            &mut state,
            SandboxState::save_snapshot,
            SandboxState::load_snapshot,
        );

        app.handle_game_commands(|cmd| {
            let mut parts = cmd.split_whitespace();
            match parts.next() {
                Some("yaw") => {
                    let y: f32 = parts.next().ok_or("missing radians")?.parse().map_err(|_| "bad float")?;
                    state.cam_yaw = y;
                    state.auto_orbit = false;
                    Ok(format!("cam_yaw={y:.3}"))
                }
                Some("bounce") => {
                    state.ball.apply_impulse(Vec3::new(0.0, 9.5, 0.0));
                    Ok("bounced".into())
                }
                _ => Err("usage: cmd <yaw rad | bounce>".into()),
            }
        });

        if app.should_simulate() {
            update(&mut state, &app.input, &app.data, app.dt);
        }
        if state.play_ping {
            state.play_ping = false;
            app.play_sound(state.ping_sfx, PlayParams::default());
        }

        app.set_state_summary(format!(
            "props={} ball_y={:.2} grounded={} yaw={:.2}",
            state.props.len(),
            state.ball.pos.y,
            if state.ball.grounded { 1 } else { 0 },
            state.cam_yaw
        ));

        let (w, h, fps) = (app.width(), app.height(), app.fps);
        render(&mut state, &app.input, &app.assets, &app.data, &mut app.draw, w, h, fps);
        app.end_frame();
    }
}
