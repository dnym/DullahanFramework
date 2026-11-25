mod game;

use dullahan::App;
use dullahan::dev::AppConfig;
use dullahan::gfx::DebugOverlay;
use dullahan::platform::Key;
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
        // Esc toggles the Pause & Settings menu during gameplay, closes Settings on Title, or quits on Title.
        if app.input.key_pressed(Key::Escape) {
            if state.phase == Phase::Playing {
                state.paused = !state.paused;
            } else if state.paused {
                state.paused = false;
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

        // Keep FPS mouse lock synced with active unpaused gameplay
        let want_lock = state.phase == Phase::Playing && !state.paused && app.input.focused;
        if want_lock && !app.is_cursor_locked() {
            app.set_cursor_locked(true);
        } else if !want_lock && app.is_cursor_locked() {
            app.set_cursor_locked(false);
        }

        app.handle_snapshots(
            &mut state,
            GameState::save_snapshot,
            GameState::load_snapshot,
        );

        let mut pending: Vec<(String, f32, f32, f32)> = Vec::new();
        app.handle_game_commands(|cmd| {
            let mut parts = cmd.split_whitespace();
            match parts.next() {
                Some("start") => {
                    pending.push(("start".into(), 0.0, 0.0, 0.0));
                    Ok("operation started".into())
                }
                Some("pause") => {
                    pending.push(("pause".into(), 0.0, 0.0, 0.0));
                    Ok("pause toggled".into())
                }
                Some("fire") => {
                    pending.push(("fire".into(), 0.0, 0.0, 0.0));
                    Ok("laser fired".into())
                }
                Some("slide") => {
                    pending.push(("slide".into(), 0.0, 0.0, 0.0));
                    Ok("parkour slide triggered".into())
                }
                Some("touch") => {
                    pending.push(("touch".into(), 0.0, 0.0, 0.0));
                    Ok("touch triggered".into())
                }
                Some("match_demo") => {
                    pending.push(("match_demo".into(), 0.0, 0.0, 0.0));
                    Ok("demo match queued".into())
                }
                Some("look") => {
                    let yaw: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let pitch: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(-0.12);
                    pending.push(("look".into(), yaw, pitch, 0.0));
                    Ok(format!("look yaw={yaw:.2} pitch={pitch:.2}"))
                }
                Some("tp") => {
                    let nums: Vec<f32> = parts.filter_map(|s| s.parse().ok()).collect();
                    let (x, y, z) = match nums.as_slice() {
                        [x, y, z, ..] => (*x, *y, *z),
                        [x, z] => (*x, 0.0, *z),
                        _ => (0.0, 0.0, 10.0),
                    };
                    pending.push(("tp".into(), x, y, z));
                    Ok(format!("tp x={x:.2} y={y:.2} z={z:.2}"))
                }
                _ => Err("usage: cmd <start|pause|fire|slide|touch|match_demo|look yaw pitch|tp x [y] z>".into()),
            }
        });

        for (act, a, b, c) in pending {
            match act.as_str() {
                "start" => state.start_run(),
                "pause" => state.paused = !state.paused,
                "fire" => state.fire_laser(&app.data),
                "slide" => state.trigger_slide(&app.data),
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
                    state.pos.y = b;
                    state.pos.z = c;
                }
                _ => {}
            }
        }

        if app.should_simulate() {
            state.update(&app.input, &app.data, app.dt);
        }
        state.flush_audio(&mut app);

        app.set_state_summary(format!(
            "phase={} paused={} sliding={} pos={:.1},{:.1},{:.1} hp={:.0} pairs={}/{} score={}",
            state.phase.as_str(),
            state.paused as u8,
            state.sliding as u8,
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
        if state.quit_requested {
            break;
        }
        app.end_frame();
    }
}
