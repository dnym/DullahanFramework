mod game;

use dullahan::App;
use dullahan::dev::AppConfig;
use dullahan::gfx::DebugOverlay;
use dullahan::platform::Key;
use game::GameState;
use std::path::PathBuf;

fn main() {
    let mut cfg = AppConfig::from_args("NEON RUSH — Dullahan Framework", 1280, 720);
    // Resolve assets/ & data/ relative to CWD first, falling back to demos/neon_rush/ so
    // `cargo run -p neon_rush` works seamlessly from the workspace root as well.
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

        app.handle_snapshots(
            &mut state,
            GameState::save_snapshot,
            GameState::load_snapshot,
        );

        let mut pending_cmd_actions = Vec::new();
        app.handle_game_commands(|cmd| {
            let mut parts = cmd.split_whitespace();
            match parts.next() {
                Some("start") => {
                    pending_cmd_actions.push(("start".to_string(), 0));
                    Ok("starting run".into())
                }
                Some("jump") => {
                    pending_cmd_actions.push(("jump".to_string(), 0));
                    Ok("jump queued".into())
                }
                Some("slide") => {
                    pending_cmd_actions.push(("slide".to_string(), 0));
                    Ok("slide triggered".into())
                }
                Some("lane") => {
                    let target: i32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    pending_cmd_actions.push(("lane".to_string(), target));
                    Ok(format!("lane={target}"))
                }
                Some("boost") => {
                    pending_cmd_actions.push(("boost".to_string(), 0));
                    Ok("boost triggered".into())
                }
                Some("shield") => {
                    pending_cmd_actions.push(("shield".to_string(), 0));
                    Ok("shield enabled".into())
                }
                Some("crash") => {
                    pending_cmd_actions.push(("crash".to_string(), 0));
                    Ok("crash triggered".into())
                }
                _ => Err("usage: cmd <start|jump|slide|lane -1..1|boost|shield|crash>".into()),
            }
        });

        for (act, arg) in pending_cmd_actions {
            match act.as_str() {
                "start" => state.start_run(&app.data),
                "jump" => state.trigger_jump(&app.data),
                "slide" => state.trigger_slide(&app.data),
                "lane" => state.trigger_lane(arg - state.lane),
                "boost" => state.trigger_boost(&app.data),
                "shield" => state.has_shield = true,
                "crash" => state.trigger_crash(),
                _ => {}
            }
        }

        if app.should_simulate() {
            state.update(&app.input, &app.data, app.dt);
        }
        state.flush_audio(&mut app);

        app.set_state_summary(format!(
            "phase={} lane={} x={:.2} y={:.2} spd={:.1} score={:.0} coins={} shield={}",
            state.phase.as_str(),
            state.lane,
            state.player_x,
            state.player_y,
            state.speed,
            state.score,
            state.coins,
            if state.has_shield { 1 } else { 0 }
        ));

        let (w, h, elapsed) = (app.width(), app.height(), app.elapsed);
        state.render(&app.input, &app.assets, &app.data, &mut app.draw, w, h, elapsed);
        app.end_frame();
    }
}
