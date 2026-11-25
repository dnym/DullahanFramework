mod game;

use dullahan::App;
use dullahan::dev::AppConfig;
use dullahan::gfx::DebugOverlay;
use dullahan::platform::{Key, MouseButton};
use game::{GameState, Phase};
use std::path::PathBuf;

fn main() {
    let mut cfg = AppConfig::from_args("MAHJONG WARS — Dullahan Framework", 1280, 720);
    if !cfg.asset_root.exists() {
        cfg.asset_root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
    }
    if !cfg.data_root.exists() {
        cfg.data_root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/data"));
    }

    let mut app = match App::new(cfg) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[dullahan] init failed: {e}");
            std::process::exit(1);
        }
    };
    let mut state = GameState::new(&mut app);

    while app.begin_frame() {
        // Esc unlocks cursor when playing; press Esc again while unlocked (or on Title) to quit.
        if app.input.key_pressed(Key::Escape) {
            if app.is_cursor_locked() {
                app.set_cursor_locked(false);
            } else {
                break;
            }
        }
        if app.input.key_pressed(Key::F1) {
            app.draw.overlay = match app.draw.overlay {
                DebugOverlay::None => DebugOverlay::Colliders,
                DebugOverlay::Colliders => DebugOverlay::Wireframe,
                DebugOverlay::Wireframe => DebugOverlay::Grid,
                DebugOverlay::Grid => DebugOverlay::None,
            };
        }

        // Engage smooth FPS cursor lock automatically when clicking into gameplay
        if state.phase == Phase::Playing && app.input.mouse_pressed(MouseButton::Left) && !app.is_cursor_locked() {
            app.set_cursor_locked(true);
        }
        if state.phase != Phase::Playing && app.is_cursor_locked() {
            app.set_cursor_locked(false);
        }

        app.handle_snapshots(
            &mut state,
            GameState::save_snapshot,
            GameState::load_snapshot,
        );

        let mut pending: Vec<(String, f32, f32)> = Vec::new();
        app.handle_game_commands(|cmd| {
            let mut parts = cmd.split_whitespace();
            match parts.next() {
                Some("start") => {
                    pending.push(("start".into(), 0.0, 0.0));
                    Ok("operation started".into())
                }
                Some("fire") => {
                    pending.push(("fire".into(), 0.0, 0.0));
                    Ok("laser fired".into())
                }
                Some("touch") => {
                    pending.push(("touch".into(), 0.0, 0.0));
                    Ok("touch triggered".into())
                }
                Some("match_demo") => {
                    pending.push(("match_demo".into(), 0.0, 0.0));
                    Ok("demo match queued".into())
                }
                Some("look") => {
                    let yaw: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let pitch: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(-0.12);
                    pending.push(("look".into(), yaw, pitch));
                    Ok(format!("look yaw={yaw:.2} pitch={pitch:.2}"))
                }
                Some("tp") => {
                    let x: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let z: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(10.0);
                    pending.push(("tp".into(), x, z));
                    Ok(format!("tp x={x:.2} z={z:.2}"))
                }
                _ => Err("usage: cmd <start|fire|touch|match_demo|look yaw pitch|tp x z>".into()),
            }
        });

        for (act, a, b) in pending {
            match act.as_str() {
                "start" => state.start_run(),
                "fire" => state.fire_laser(&app.data),
                "touch" => state.touch_aimed_tile(&app.data),
                "match_demo" => {
                    let _ = state.trigger_demo_match();
                }
                "look" => {
                    state.yaw = a;
                    state.pitch = b;
                }
                "tp" => {
                    state.pos.x = a;
                    state.pos.z = b;
                }
                _ => {}
            }
        }

        if app.should_simulate() {
            state.update(&app.input, &app.data, app.dt);
        }
        state.flush_audio(&mut app);

        app.set_state_summary(format!(
            "phase={} pos={:.1},{:.1},{:.1} hp={:.0} pairs={}/{} score={}",
            state.phase.as_str(),
            state.pos.x,
            state.pos.y,
            state.pos.z,
            state.hp,
            state.pairs_cleared,
            state.total_pairs,
            state.score,
        ));

        let (w, h, elapsed) = (app.width(), app.height(), app.elapsed);
        state.render(&app.input, &app.data, &mut app.draw, w, h, elapsed);
        app.end_frame();
    }
}
