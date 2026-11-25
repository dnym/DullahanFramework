mod game;

use dullahan::App;
use dullahan::dev::AppConfig;
use dullahan::gfx::DebugOverlay;
use dullahan::math::Vec2;
use dullahan::platform::Key;
use game::{GameState, Phase};
use std::path::PathBuf;

fn main() {
    let mut cfg = AppConfig::from_args("COZY GOLF — Dullahan Framework", 1280, 720);
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
        if app.input.key_pressed(Key::Escape) {
            if state.phase == Phase::Title && !state.paused {
                break;
            } else {
                state.paused = !state.paused;
            }
        }
        if app.input.key_pressed(Key::F1) {
            app.draw.overlay = match app.draw.overlay {
                DebugOverlay::None => DebugOverlay::Colliders,
                DebugOverlay::Colliders => DebugOverlay::None,
                _ => DebugOverlay::None,
            };
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
                    Ok("course started".into())
                }
                Some("pause") => {
                    pending.push(("pause".into(), 0.0, 0.0));
                    Ok("pause toggled".into())
                }
                Some("hole") => {
                    let n: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                    pending.push(("hole".into(), n, 0.0));
                    Ok(format!("loading hole {n}"))
                }
                Some("aim") => {
                    let vx: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(450.0);
                    let vy: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(-120.0);
                    pending.push(("aim".into(), vx, vy));
                    Ok(format!("aim {vx:.0},{vy:.0}"))
                }
                Some("putt") => {
                    let vx: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(450.0);
                    let vy: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(-120.0);
                    pending.push(("putt".into(), vx, vy));
                    Ok(format!("putt {vx:.0},{vy:.0}"))
                }
                Some("next") => {
                    pending.push(("next".into(), 0.0, 0.0));
                    Ok("next hole".into())
                }
                _ => Err("usage: cmd <start|pause|hole 1..6|aim vx vy|putt vx vy|next>".into()),
            }
        });

        for (act, a, b) in pending {
            match act.as_str() {
                "start" => state.start_course(),
                "pause" => state.paused = !state.paused,
                "hole" => state.load_hole((a as usize).saturating_sub(1)),
                "aim" => state.set_demo_aim(Vec2::new(a, b)),
                "putt" => state.execute_putt(Vec2::new(a, b), &app.data),
                "next" => state.next_hole(),
                _ => {}
            }
        }

        let (w, h, elapsed) = (app.width(), app.height(), app.elapsed);
        if app.should_simulate() {
            state.update(&app.input, &app.data, app.dt, w, h);
        }
        state.flush_audio(&mut app);

        app.set_state_summary(format!(
            "phase={} hole={}/{} par={} strokes={} ball={:.0},{:.0}",
            state.phase.as_str(),
            state.hole_idx + 1,
            state.hole_count(),
            state.current_par(),
            state.strokes,
            state.ball_pos.x,
            state.ball_pos.y,
        ));

        state.render(&app.input, &app.data, &mut app.draw, w, h, elapsed);
        if state.quit_requested {
            break;
        }
        app.end_frame();
    }
}
