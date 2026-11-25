//! Dullahan Framework — zero-dependency, flat, LLM-first Rust game framework.
#![allow(dead_code)]

pub mod assets;
pub mod audio;
pub mod data;
pub mod dev;
pub mod font;
pub mod formats;
pub mod geom;
pub mod gfx;
pub mod math;
pub mod platform;

use assets::{Assets, MeshHandle, SoundHandle, TexHandle};
use audio::{AudioBus, AudioEngine, PlayParams, VoiceId};
use data::{DataStore, Snapshot};
use dev::{AppConfig, DevAction, DevBridge, DevStream, PendingScreenshot};
use gfx::{DrawList, Gfx};
use platform::{GL_RENDERER, GL_VERSION, InputState, Platform};
use std::path::{Path, PathBuf};
use std::time::Instant;

enum SnapshotReq {
    Save(PathBuf, Option<DevStream>),
    Load(PathBuf, Option<DevStream>),
}

pub struct App {
    pub config: AppConfig,
    pub platform: Platform,
    pub gfx: Gfx,
    pub assets: Assets,
    pub data: DataStore,
    pub audio: AudioEngine,
    pub draw: DrawList,
    pub input: InputState,
    pub dev: DevBridge,

    pub frame: u64,
    /// Max milliseconds per frame spent uploading background-loaded meshes (at least one
    /// mesh is always uploaded per frame while any are waiting).
    pub upload_budget_ms: f32,
    /// Background loading summary for the current frame.
    pub load_report: crate::dullahan::assets::LoadReport,
    pub dt: f32,
    pub elapsed: f32,
    pub fps: f32,
    last_time: Instant,
    running: bool,
    sim_this_frame: bool,
    custom_state_text: String,
    pending_game_cmds: Vec<(String, Option<DevStream>)>,
    pending_snapshots: Vec<SnapshotReq>,
}

impl App {
    pub fn from_args(title: &str, default_width: u32, default_height: u32) -> Result<Self, String> {
        let config = AppConfig::from_args(title, default_width, default_height);
        Self::new(config)
    }

    pub fn new(config: AppConfig) -> Result<Self, String> {
        let platform = Platform::new(&config.title, config.width, config.height, config.headless)?;
        let mut gfx = Gfx::new(&platform.gl)?;
        let assets = Assets::new(&platform.gl, &config.asset_root);
        let data = DataStore::new(&config.data_root);
        let audio = AudioEngine::new(config.headless);
        let dev = DevBridge::new(config.sock_path.clone(), config.script_path.as_deref());

        let (vs_ov, fs_ov) = assets.pbr_shader_sources();
        if vs_ov.is_some() || fs_ov.is_some() {
            if let Err(e) = gfx.reload_pbr_shader(&platform.gl, vs_ov.as_deref(), fs_ov.as_deref()) {
                eprintln!("[dullahan] shader override failed at startup (using default): {e}");
            }
        }
        if let Some(post_fs) = assets.post_shader_source() {
            if let Err(e) = gfx.reload_post_shader(&platform.gl, Some(&post_fs)) {
                eprintln!("[dullahan] post shader override failed at startup (using default): {e}");
            }
        }

        let gl_ver = platform.gl.gl_string(GL_VERSION);
        let gl_ren = platform.gl.gl_string(GL_RENDERER);
        let win_mode = if cfg!(target_os = "windows") { "win32" } else { "x11" };
        eprintln!(
            "[dullahan] ready mode={} size={}x{} audio={} gl=\"{}\" renderer=\"{}\"",
            if config.headless { "headless" } else { win_mode },
            platform.width,
            platform.height,
            audio.backend_name,
            gl_ver,
            gl_ren,
        );

        Ok(Self {
            config,
            platform,
            gfx,
            assets,
            data,
            audio,
            draw: DrawList::default(),
            input: InputState::default(),
            dev,
            frame: 0,
            upload_budget_ms: 2.0,
            load_report: Default::default(),
            dt: 1.0 / 60.0,
            elapsed: 0.0,
            fps: 60.0,
            last_time: Instant::now(),
            running: true,
            sim_this_frame: true,
            custom_state_text: String::new(),
            pending_game_cmds: Vec::new(),
            pending_snapshots: Vec::new(),
        })
    }

    /// Loads the startup `--restore <path>` snapshot if passed on the command line.
    pub fn startup_snapshot(&self) -> Option<Snapshot> {
        let path = self.config.restore_path.as_ref()?;
        match Snapshot::load_file(path) {
            Ok(s) => {
                eprintln!("[dullahan] restored snapshot {}", path.display());
                Some(s)
            }
            Err(e) => {
                eprintln!("[dullahan] warning: failed to restore {}: {e}", path.display());
                None
            }
        }
    }

    pub fn load_mesh<P: AsRef<Path>>(&mut self, path: P) -> MeshHandle {
        self.assets.load_mesh(&self.platform.gl, path)
    }

    /// Loads a mesh on a background thread; see `Assets::load_mesh_async`.
    pub fn load_mesh_async<P: AsRef<Path>>(&mut self, path: P) -> MeshHandle {
        self.assets.load_mesh_async(path)
    }

    pub fn unload_mesh(&mut self, handle: MeshHandle) {
        self.assets.unload_mesh(&self.platform.gl, handle)
    }

    pub fn load_texture<P: AsRef<Path>>(&mut self, path: P, linear: bool) -> TexHandle {
        self.assets.load_texture(&self.platform.gl, path, linear)
    }

    pub fn load_svg<P: AsRef<Path>>(&mut self, path: P, width: u32, height: u32) -> TexHandle {
        self.assets.load_svg(&self.platform.gl, path, width, height)
    }

    pub fn create_terrain_mesh(
        &mut self,
        hf: &geom::HeightField,
        albedo_tex: Option<TexHandle>,
        grid_x: u32,
        grid_z: u32,
    ) -> MeshHandle {
        self.assets.create_terrain_mesh(&self.platform.gl, hf, albedo_tex, grid_x, grid_z)
    }

    pub fn load_sound<P: AsRef<Path>>(&mut self, path: P) -> SoundHandle {
        self.assets.load_sound(path)
    }

    pub fn play_sound(&mut self, handle: SoundHandle, params: PlayParams) -> Option<VoiceId> {
        let clip = self.assets.sound(handle)?.clone();
        Some(self.audio.play(clip, params))
    }

    pub fn play_loop(&mut self, handle: SoundHandle, gain: f32, bus: AudioBus) -> Option<VoiceId> {
        let clip = self.assets.sound(handle)?.clone();
        Some(self.audio.play_loop(clip, gain, bus))
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.platform.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.platform.height
    }

    pub fn set_cursor_locked(&mut self, locked: bool) {
        self.platform.set_cursor_locked(locked);
    }

    pub fn is_cursor_locked(&self) -> bool {
        self.platform.is_cursor_locked()
    }

    pub fn begin_frame(&mut self) -> bool {
        if !self.running || self.input.quit_requested {
            return false;
        }
        if let Some(max_f) = self.config.max_frames
            && self.frame >= max_f
        {
            return false;
        }

        let now = Instant::now();
        let raw_dt = (now - self.last_time).as_secs_f32().clamp(1e-5, 0.1);
        self.last_time = now;
        self.dt = self.config.fixed_dt.unwrap_or(raw_dt);
        if raw_dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / raw_dt) * 0.1;
        }

        // Tier 1 (assets & shaders) + Tier 2 (data configs/tables/levels) hot-reload
        self.assets.hot_reload(&self.platform.gl, &mut self.gfx, false);
        self.data.hot_reload(false);
        // Upload meshes parsed by background loader threads, within the frame budget.
        let budget = std::time::Duration::from_secs_f32(self.upload_budget_ms.max(0.0) / 1000.0);
        self.load_report = self.assets.poll_loads(&self.platform.gl, budget);

        self.input.begin_frame(self.dt);
        self.platform.poll_events(&mut self.input);
        if self.input.quit_requested {
            return false;
        }

        let summary = self.format_state_summary();
        let actions = self.dev.poll(self.frame, &summary, &mut self.input);
        for action in actions {
            match action {
                DevAction::None => {}
                DevAction::Quit => {
                    self.running = false;
                    return false;
                }
                DevAction::Reload { mut stream } => {
                    let a_rep = self.assets.hot_reload(&self.platform.gl, &mut self.gfx, true);
                    let (d_rel, d_err) = self.data.hot_reload(true);
                    dev::reply(
                        &mut stream,
                        &format!(
                            "OK reloaded={} errors={} total={} data_gen={}\n",
                            a_rep.reloaded + d_rel,
                            a_rep.errors + d_err,
                            self.assets.total_reloads + self.data.total_reloads,
                            self.data.generation
                        ),
                    );
                }
                DevAction::SetConfig { key, val, mut stream } => {
                    self.data.set_override(&key, &val);
                    dev::reply(&mut stream, &format!("OK set {key}={val} data_gen={}\n", self.data.generation));
                }
                DevAction::SnapshotSave { path, stream } => {
                    self.pending_snapshots.push(SnapshotReq::Save(path, stream));
                }
                DevAction::SnapshotLoad { path, stream } => {
                    self.pending_snapshots.push(SnapshotReq::Load(path, stream));
                }
                DevAction::SetOverlay(ov) => {
                    self.draw.overlay = ov;
                }
                DevAction::GameCmd { args, stream } => {
                    self.pending_game_cmds.push((args, stream));
                }
            }
        }

        self.sim_this_frame = self.dev.should_simulate();
        if self.sim_this_frame {
            self.elapsed += self.dt;
        }

        self.draw.clear();
        true
    }

    #[inline]
    pub fn should_simulate(&self) -> bool {
        self.sim_this_frame
    }

    pub fn set_state_summary(&mut self, text: String) {
        self.custom_state_text = text;
    }

    pub fn handle_game_commands<F>(&mut self, mut f: F)
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        for (args, mut stream) in self.pending_game_cmds.drain(..) {
            match f(&args) {
                Ok(msg) => dev::reply(&mut stream, &format!("OK {msg}\n")),
                Err(err) => dev::reply(&mut stream, &format!("ERR {err}\n")),
            }
        }
    }

    /// Services any `dctl snapshot save|load` commands without closure borrow conflicts on `state`.
    pub fn handle_snapshots<T, FSave, FLoad>(&mut self, state: &mut T, mut save_fn: FSave, mut load_fn: FLoad)
    where
        FSave: FnMut(&T, &mut Snapshot),
        FLoad: FnMut(&mut T, &Snapshot),
    {
        for req in self.pending_snapshots.drain(..) {
            match req {
                SnapshotReq::Save(path, mut stream) => {
                    let mut snap = Snapshot::new();
                    snap.set_i32("meta", "frame", self.frame as i32);
                    snap.set_f32("meta", "elapsed", self.elapsed);
                    save_fn(state, &mut snap);
                    match snap.save_file(&path) {
                        Ok(()) => dev::reply(&mut stream, &format!("OK snapshot saved {}\n", path.display())),
                        Err(e) => dev::reply(&mut stream, &format!("ERR {e}\n")),
                    }
                }
                SnapshotReq::Load(path, mut stream) => match Snapshot::load_file(&path) {
                    Ok(snap) => {
                        load_fn(state, &snap);
                        dev::reply(&mut stream, &format!("OK snapshot loaded {}\n", path.display()));
                    }
                    Err(e) => dev::reply(&mut stream, &format!("ERR {e}\n")),
                },
            }
        }
    }

    pub fn end_frame(&mut self) {
        let fbo = self.platform.default_framebuffer();
        let w = self.platform.width;
        let h = self.platform.height;

        self.gfx.render_frame(&self.platform.gl, fbo, w, h, &mut self.draw, &self.assets);

        if let Some(PendingScreenshot { path, mut reply_stream }) = self.dev.pending_screenshot.take() {
            match self.gfx.save_screenshot(&self.platform.gl, fbo, w, h, &path) {
                Ok(()) => dev::reply(
                    &mut reply_stream,
                    &format!("OK screenshot {} {}x{}\n", path.display(), w, h),
                ),
                Err(e) => dev::reply(&mut reply_stream, &format!("ERR {e}\n")),
            }
        }

        let is_final = self.config.max_frames.is_some_and(|m| self.frame + 1 >= m);
        if is_final && let Some(snap_path) = self.config.snap_path.clone() {
            match self.gfx.save_screenshot(&self.platform.gl, fbo, w, h, &snap_path) {
                Ok(()) => {
                    println!(
                        "SUMMARY {} snap={}",
                        self.format_state_summary(),
                        snap_path.display()
                    );
                }
                Err(e) => eprintln!("[dullahan] final snap failed: {e}"),
            }
        }

        self.platform.swap_buffers();
        self.frame += 1;
    }

    pub fn format_state_summary(&self) -> String {
        let s = self.gfx.last_stats;
        let (nm, nt, ns) = self.assets.counts();
        let base = format!(
            "frame={} fps={:.1} dt={:.4} size={}x{} draws={} tris={} lines={} ui={} assets={}/{}/{} reloads={} data_gen={} paused={} overlay={}",
            self.frame,
            self.fps,
            self.dt,
            self.platform.width,
            self.platform.height,
            s.draw_calls,
            s.triangles,
            s.lines,
            s.ui_quads,
            nm,
            nt,
            ns,
            self.assets.total_reloads + self.data.total_reloads,
            self.data.generation,
            if self.dev.paused { 1 } else { 0 },
            self.draw.overlay.as_str(),
        );
        if self.custom_state_text.is_empty() {
            base
        } else {
            format!("{base} {}", self.custom_state_text)
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.assets.destroy(&self.platform.gl);
        self.gfx.destroy(&self.platform.gl);
    }
}
