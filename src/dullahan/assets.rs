//! Handle-based asset registry with safe file hot-reloading and fallback placeholders.
//!
//! Handles remain stable across reloads. Corrupt or partially-written files never crash
//! the game: diagnostics are logged and the previous valid GPU/audio resource stays live.

use crate::dullahan::formats::gltf::{self, GltfCollider, GltfModel, TextureSource};
use crate::dullahan::formats::png;
use crate::dullahan::formats::svg;
use crate::dullahan::formats::wav::{self, AudioClip};
use crate::dullahan::geom::HeightField;
use crate::dullahan::gfx::{self, GpuInstances, GpuMaterial, GpuMesh, GpuTexture};
use crate::dullahan::math::{Aabb, Mat4};
use crate::dullahan::platform::GlFns;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TexHandle(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MeshHandle(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoundHandle(pub u32);

/// Handle to a static GPU buffer of per-instance transforms (see `Assets::create_instances`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceHandle(pub u32);

#[derive(Copy, Clone, Debug, Default)]
pub struct ReloadReport {
    pub reloaded: u32,
    pub errors: u32,
}

struct TexEntry {
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    linear_filter: bool,
    svg_size: Option<(u32, u32)>,
    gpu: GpuTexture,
    in_use: bool,
}

struct MeshEntry {
    path: PathBuf,
    mtime: Option<SystemTime>,
    dep_mtimes: Vec<(PathBuf, Option<SystemTime>)>,
    embedded_tex_handles: Vec<TexHandle>,
    gpu: GpuMesh,
    is_fallback: bool,
}

struct SoundEntry {
    path: PathBuf,
    mtime: Option<SystemTime>,
    clip: Arc<AudioClip>,
}

struct ShaderWatch {
    vert_path: PathBuf,
    frag_path: PathBuf,
    vert_mtime: Option<SystemTime>,
    frag_mtime: Option<SystemTime>,
}

pub struct Assets {
    root: PathBuf,
    textures: Vec<TexEntry>,
    meshes: Vec<MeshEntry>,
    sounds: Vec<SoundEntry>,
    /// Instance buffers; `None` marks a freed slot that `create_instances` reuses.
    instance_sets: Vec<Option<GpuInstances>>,
    pbr_shader_watch: ShaderWatch,
    post_frag_path: PathBuf,
    post_frag_mtime: Option<SystemTime>,
    pub white_tex: TexHandle,
    pub black_tex: TexHandle,
    pub checker_tex: TexHandle,
    last_poll: Instant,
    poll_interval: Duration,
    pub total_reloads: u32,
    pub last_error: String,
}

impl Assets {
    pub fn new<P: AsRef<Path>>(gl: &GlFns, asset_root: P) -> Self {
        let root = asset_root.as_ref().to_path_buf();
        let white_gpu = gfx::upload_texture_rgba(gl, 1, 1, &[255, 255, 255, 255], false);
        let black_gpu = gfx::upload_texture_rgba(gl, 1, 1, &[0, 0, 0, 255], false);
        let checker_pixels = make_checkerboard_8x8();
        let checker_gpu = gfx::upload_texture_rgba(gl, 8, 8, &checker_pixels, false);

        let textures = vec![
            TexEntry { path: None, mtime: None, linear_filter: false, svg_size: None, gpu: white_gpu, in_use: true },
            TexEntry { path: None, mtime: None, linear_filter: false, svg_size: None, gpu: black_gpu, in_use: true },
            TexEntry { path: None, mtime: None, linear_filter: false, svg_size: None, gpu: checker_gpu, in_use: true },
        ];

        let vert_path = root.join("shaders/pbr.vert");
        let frag_path = root.join("shaders/pbr.frag");
        let post_frag_path = root.join("shaders/post.frag");
        let pbr_shader_watch = ShaderWatch {
            vert_mtime: file_mtime(&vert_path),
            frag_mtime: file_mtime(&frag_path),
            vert_path,
            frag_path,
        };
        let post_frag_mtime = file_mtime(&post_frag_path);

        Self {
            root,
            textures,
            meshes: Vec::new(),
            sounds: Vec::new(),
            instance_sets: Vec::new(),
            pbr_shader_watch,
            post_frag_path,
            post_frag_mtime,
            white_tex: TexHandle(0),
            black_tex: TexHandle(1),
            checker_tex: TexHandle(2),
            last_poll: Instant::now(),
            poll_interval: Duration::from_millis(120),
            total_reloads: 0,
            last_error: String::new(),
        }
    }

    pub fn pbr_shader_sources(&self) -> (Option<String>, Option<String>) {
        let vs = fs::read_to_string(&self.pbr_shader_watch.vert_path).ok();
        let fs = fs::read_to_string(&self.pbr_shader_watch.frag_path).ok();
        (vs, fs)
    }

    pub fn post_shader_source(&self) -> Option<String> {
        fs::read_to_string(&self.post_frag_path).ok()
    }

    fn resolve_path(&self, rel_or_abs: &Path) -> PathBuf {
        if rel_or_abs.is_absolute() || rel_or_abs.exists() {
            rel_or_abs.to_path_buf()
        } else {
            self.root.join(rel_or_abs)
        }
    }

    fn alloc_tex_slot(&mut self, entry: TexEntry) -> TexHandle {
        if let Some((idx, slot)) = self.textures.iter_mut().enumerate().skip(3).find(|(_, t)| !t.in_use) {
            *slot = entry;
            return TexHandle(idx as u32);
        }
        let idx = self.textures.len() as u32;
        self.textures.push(entry);
        TexHandle(idx)
    }

    fn release_embedded_tex_slots(&mut self, handles: &[TexHandle]) {
        for &h in handles {
            if let Some(entry) = self.textures.get_mut(h.0 as usize) {
                entry.in_use = false;
                entry.gpu = GpuTexture { id: 0, width: 0, height: 0 };
            }
        }
    }

    pub fn load_texture<P: AsRef<Path>>(&mut self, gl: &GlFns, path: P, linear_filter: bool) -> TexHandle {
        let is_svg = path
            .as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        if is_svg {
            return self.load_svg(gl, path, 0, 0);
        }
        self.load_texture_inner(gl, path.as_ref(), linear_filter, None)
    }

    pub fn load_svg<P: AsRef<Path>>(&mut self, gl: &GlFns, path: P, width: u32, height: u32) -> TexHandle {
        self.load_texture_inner(gl, path.as_ref(), true, Some((width, height)))
    }

    fn load_texture_inner(
        &mut self,
        gl: &GlFns,
        path: &Path,
        linear_filter: bool,
        svg_size: Option<(u32, u32)>,
    ) -> TexHandle {
        let full = self.resolve_path(path);
        if let Some(idx) = self.textures.iter().position(|t| {
            t.in_use && t.path.as_deref() == Some(full.as_path()) && t.svg_size == svg_size
        }) {
            return TexHandle(idx as u32);
        }

        let mtime = file_mtime(&full);
        let loaded = if let Some((w, h)) = svg_size {
            svg::load_svg_file(&full, w, h)
        } else {
            png::load_png_rgba(&full)
        };
        let gpu = match loaded {
            Ok(img) => gfx::upload_texture_rgba(gl, img.width, img.height, &img.rgba, linear_filter),
            Err(e) => {
                self.record_error(format!("texture {}: {e}", full.display()));
                let px = make_checkerboard_8x8();
                gfx::upload_texture_rgba(gl, 8, 8, &px, false)
            }
        };

        self.alloc_tex_slot(TexEntry {
            path: Some(full),
            mtime,
            linear_filter,
            svg_size,
            gpu,
            in_use: true,
        })
    }

    pub fn load_mesh<P: AsRef<Path>>(&mut self, gl: &GlFns, path: P) -> MeshHandle {
        let full = self.resolve_path(path.as_ref());
        if let Some(idx) = self.meshes.iter().position(|m| m.path == full) {
            return MeshHandle(idx as u32);
        }

        let mtime = file_mtime(&full);
        let (gpu, dep_mtimes, embedded_tex_handles, is_fallback) = match self.build_gpu_mesh_from_file(gl, &full) {
            Ok((gpu, deps, emb)) => (gpu, deps, emb, false),
            Err(e) => {
                self.record_error(format!("mesh {}: {e}", full.display()));
                (
                    gfx::create_fallback_mesh(gl, self.checker_tex, self.white_tex, self.black_tex),
                    Vec::new(),
                    Vec::new(),
                    true,
                )
            }
        };

        let idx = self.meshes.len() as u32;
        self.meshes.push(MeshEntry {
            path: full,
            mtime,
            dep_mtimes,
            embedded_tex_handles,
            gpu,
            is_fallback,
        });
        MeshHandle(idx)
    }

    /// Creates a lit PBR terrain `MeshHandle` directly from a `HeightField` + optional color/splat texture.
    pub fn create_terrain_mesh(
        &mut self,
        gl: &GlFns,
        hf: &HeightField,
        albedo_tex: Option<TexHandle>,
        grid_x: u32,
        grid_z: u32,
    ) -> MeshHandle {
        let tex = albedo_tex.unwrap_or(self.white_tex);
        let gpu = gfx::create_heightfield_mesh(gl, hf, tex, self.white_tex, self.black_tex, grid_x, grid_z);
        let idx = self.meshes.len() as u32;
        self.meshes.push(MeshEntry {
            path: PathBuf::new(),
            mtime: None,
            dep_mtimes: Vec::new(),
            embedded_tex_handles: Vec::new(),
            gpu,
            is_fallback: false,
        });
        MeshHandle(idx)
    }

    pub fn load_sound<P: AsRef<Path>>(&mut self, path: P) -> SoundHandle {
        let full = self.resolve_path(path.as_ref());
        if let Some(idx) = self.sounds.iter().position(|s| s.path == full) {
            return SoundHandle(idx as u32);
        }

        let mtime = file_mtime(&full);
        let clip = match wav::load_wav_file(&full) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                self.record_error(format!("sound {}: {e}", full.display()));
                Arc::new(AudioClip { sample_rate: wav::TARGET_SAMPLE_RATE, frames: Vec::new(), duration: 0.0 })
            }
        };

        let idx = self.sounds.len() as u32;
        self.sounds.push(SoundEntry { path: full, mtime, clip });
        SoundHandle(idx)
    }

    /// Checks file mtimes (throttled to ~120ms unless `force == true`) and swaps modified
    /// assets & optional shader overrides in-place.
    pub fn hot_reload(&mut self, gl: &GlFns, gfx: &mut gfx::Gfx, force: bool) -> ReloadReport {
        if !force && self.last_poll.elapsed() < self.poll_interval {
            return ReloadReport::default();
        }
        self.last_poll = Instant::now();
        let mut report = ReloadReport::default();

        // 0. Optional GLSL PBR shader overrides (`assets/shaders/pbr.vert|.frag`)
        let cur_vm = file_mtime(&self.pbr_shader_watch.vert_path);
        let cur_fm = file_mtime(&self.pbr_shader_watch.frag_path);
        if cur_vm != self.pbr_shader_watch.vert_mtime || cur_fm != self.pbr_shader_watch.frag_mtime {
            self.pbr_shader_watch.vert_mtime = cur_vm;
            self.pbr_shader_watch.frag_mtime = cur_fm;
            let (vs, fs) = self.pbr_shader_sources();
            match gfx.reload_pbr_shader(gl, vs.as_deref(), fs.as_deref()) {
                Ok(()) => {
                    report.reloaded += 1;
                    self.total_reloads += 1;
                    eprintln!("[assets] reloaded PBR shader override");
                }
                Err(e) => {
                    report.errors += 1;
                    self.record_error(format!("reload shader: {e}"));
                }
            }
        }

        let cur_pm = file_mtime(&self.post_frag_path);
        if cur_pm != self.post_frag_mtime {
            self.post_frag_mtime = cur_pm;
            let ps = self.post_shader_source();
            match gfx.reload_post_shader(gl, ps.as_deref()) {
                Ok(()) => {
                    report.reloaded += 1;
                    self.total_reloads += 1;
                    eprintln!("[assets] reloaded post-process shader");
                }
                Err(e) => {
                    report.errors += 1;
                    self.record_error(format!("reload post shader: {e}"));
                }
            }
        }

        // 1. Standalone / external textures (PNG & SVG)
        for idx in 3..self.textures.len() {
            if !self.textures[idx].in_use {
                continue;
            }
            let Some(path) = self.textures[idx].path.clone() else { continue };
            let cur_mtime = file_mtime(&path);
            if cur_mtime.is_some() && cur_mtime != self.textures[idx].mtime {
                self.textures[idx].mtime = cur_mtime;
                let linear = self.textures[idx].linear_filter;
                let svg_size = self.textures[idx].svg_size;
                let loaded = if let Some((w, h)) = svg_size {
                    svg::load_svg_file(&path, w, h)
                } else {
                    png::load_png_rgba(&path)
                };
                match loaded {
                    Ok(img) => {
                        let new_gpu = gfx::upload_texture_rgba(gl, img.width, img.height, &img.rgba, linear);
                        let old = std::mem::replace(&mut self.textures[idx].gpu, new_gpu);
                        gfx::destroy_texture(gl, old);
                        report.reloaded += 1;
                        self.total_reloads += 1;
                        eprintln!("[assets] reloaded texture {}", path.display());
                    }
                    Err(e) => {
                        report.errors += 1;
                        self.record_error(format!("reload texture {}: {e}", path.display()));
                    }
                }
            }
        }

        // 2. Meshes (.glb / .gltf + external deps)
        for idx in 0..self.meshes.len() {
            if self.meshes[idx].path.as_os_str().is_empty() {
                continue;
            }
            let path = self.meshes[idx].path.clone();
            let cur_mtime = file_mtime(&path);
            let main_changed = cur_mtime.is_some() && cur_mtime != self.meshes[idx].mtime;
            let dep_changed = self.meshes[idx].dep_mtimes.iter().any(|(dp, prev)| {
                let cm = file_mtime(dp);
                cm.is_some() && cm != *prev
            });

            if main_changed || dep_changed {
                self.meshes[idx].mtime = cur_mtime;
                match self.build_gpu_mesh_from_file(gl, &path) {
                    Ok((new_gpu, new_deps, new_emb)) => {
                        let old = std::mem::replace(&mut self.meshes[idx].gpu, new_gpu);
                        let old_emb = std::mem::replace(&mut self.meshes[idx].embedded_tex_handles, new_emb);
                        gfx::destroy_mesh(gl, &old);
                        self.release_embedded_tex_slots(&old_emb);
                        self.meshes[idx].dep_mtimes = new_deps;
                        self.meshes[idx].is_fallback = false;
                        report.reloaded += 1;
                        self.total_reloads += 1;
                        eprintln!("[assets] reloaded mesh {}", path.display());
                    }
                    Err(e) => {
                        report.errors += 1;
                        self.record_error(format!("reload mesh {}: {e}", path.display()));
                    }
                }
            }
        }

        // 3. Sounds (.wav)
        for idx in 0..self.sounds.len() {
            let path = self.sounds[idx].path.clone();
            let cur_mtime = file_mtime(&path);
            if cur_mtime.is_some() && cur_mtime != self.sounds[idx].mtime {
                self.sounds[idx].mtime = cur_mtime;
                match wav::load_wav_file(&path) {
                    Ok(clip) => {
                        self.sounds[idx].clip = Arc::new(clip);
                        report.reloaded += 1;
                        self.total_reloads += 1;
                        eprintln!("[assets] reloaded sound {}", path.display());
                    }
                    Err(e) => {
                        report.errors += 1;
                        self.record_error(format!("reload sound {}: {e}", path.display()));
                    }
                }
            }
        }

        report
    }

    fn build_gpu_mesh_from_file(
        &mut self,
        gl: &GlFns,
        path: &Path,
    ) -> Result<(GpuMesh, Vec<(PathBuf, Option<SystemTime>)>, Vec<TexHandle>), String> {
        let model: GltfModel = gltf::load_gltf_file(path)?;
        let dep_mtimes: Vec<(PathBuf, Option<SystemTime>)> = model
            .dependencies
            .iter()
            .map(|p| (p.clone(), file_mtime(p)))
            .collect();

        let mut owned_gl_textures = Vec::new();
        let mut embedded_handles = Vec::new();
        let mut tex_map: Vec<TexHandle> = Vec::with_capacity(model.images.len());

        for img in &model.images {
            match &img.source {
                TextureSource::External(ext_path) => {
                    let h = self.load_texture(gl, ext_path, img.linear_filter);
                    tex_map.push(h);
                }
                TextureSource::Embedded(decoded) => {
                    let gpu_tex = gfx::upload_texture_rgba(
                        gl,
                        decoded.width,
                        decoded.height,
                        &decoded.rgba,
                        img.linear_filter,
                    );
                    owned_gl_textures.push(gpu_tex.id);
                    let h = self.alloc_tex_slot(TexEntry {
                        path: None,
                        mtime: None,
                        linear_filter: img.linear_filter,
                        svg_size: None,
                        gpu: gpu_tex,
                        in_use: true,
                    });
                    embedded_handles.push(h);
                    tex_map.push(h);
                }
            }
        }

        let mut gpu_materials = Vec::with_capacity(model.materials.len());
        for m in &model.materials {
            let resolve = |opt: Option<usize>, default_h: TexHandle| -> TexHandle {
                opt.and_then(|i| tex_map.get(i).copied()).unwrap_or(default_h)
            };
            gpu_materials.push(GpuMaterial {
                base_color_factor: m.base_color_factor,
                base_color_tex: resolve(m.base_color_tex, self.white_tex),
                metallic_factor: m.metallic_factor,
                roughness_factor: m.roughness_factor,
                metallic_roughness_tex: resolve(m.metallic_roughness_tex, self.white_tex),
                occlusion_tex: resolve(m.occlusion_tex, self.white_tex),
                emissive_factor: m.emissive_factor,
                emissive_tex: resolve(
                    m.emissive_tex,
                    if m.emissive_tex.is_some() { self.white_tex } else { self.black_tex },
                ),
                has_emissive_tex: m.emissive_tex.is_some(),
                alpha_mode: m.alpha_mode,
                alpha_cutoff: m.alpha_cutoff,
                double_sided: m.double_sided,
            });
        }
        if gpu_materials.is_empty() {
            gpu_materials.push(GpuMaterial::default_with(self.white_tex, self.black_tex));
        }

        let gpu_mesh = gfx::upload_gltf_mesh(gl, &model, gpu_materials, owned_gl_textures);
        Ok((gpu_mesh, dep_mtimes, embedded_handles))
    }

    pub fn texture(&self, handle: TexHandle) -> &GpuTexture {
        self.textures
            .get(handle.0 as usize)
            .filter(|e| e.in_use)
            .map(|e| &e.gpu)
            .unwrap_or(&self.textures[self.checker_tex.0 as usize].gpu)
    }

    /// Uploads per-instance transforms once; draw them with `DrawList::mesh_instances`.
    pub fn create_instances(&mut self, gl: &GlFns, transforms: &[Mat4]) -> InstanceHandle {
        let gpu = gfx::upload_instances(gl, transforms);
        if let Some(i) = self.instance_sets.iter().position(Option::is_none) {
            self.instance_sets[i] = Some(gpu);
            return InstanceHandle(i as u32);
        }
        self.instance_sets.push(Some(gpu));
        InstanceHandle(self.instance_sets.len() as u32 - 1)
    }

    /// Replaces the transforms of an existing instance buffer.
    pub fn update_instances(&mut self, gl: &GlFns, handle: InstanceHandle, transforms: &[Mat4]) {
        if let Some(Some(inst)) = self.instance_sets.get_mut(handle.0 as usize) {
            gfx::write_instances(gl, inst, transforms);
        }
    }

    /// Frees an instance buffer. Drawing a destroyed handle is a no-op.
    pub fn destroy_instances(&mut self, gl: &GlFns, handle: InstanceHandle) {
        if let Some(slot) = self.instance_sets.get_mut(handle.0 as usize) {
            if let Some(inst) = slot.take() {
                gfx::destroy_instances(gl, &inst);
            }
        }
    }

    pub fn instances(&self, handle: InstanceHandle) -> Option<&GpuInstances> {
        self.instance_sets.get(handle.0 as usize).and_then(Option::as_ref)
    }

    pub fn mesh(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(handle.0 as usize).map(|e| &e.gpu)
    }

    pub fn sound(&self, handle: SoundHandle) -> Option<&Arc<AudioClip>> {
        self.sounds.get(handle.0 as usize).map(|e| &e.clip)
    }

    pub fn mesh_node_index(&self, handle: MeshHandle, name: &str) -> Option<usize> {
        self.mesh(handle)?.node_index(name)
    }

    pub fn mesh_colliders(&self, handle: MeshHandle) -> &[GltfCollider] {
        self.mesh(handle).map(|m| m.colliders.as_slice()).unwrap_or(&[])
    }

    pub fn mesh_bounds(&self, handle: MeshHandle) -> Aabb {
        self.mesh(handle).map(|m| m.bounds).unwrap_or(Aabb::new(
            crate::dullahan::math::Vec3::ZERO,
            crate::dullahan::math::Vec3::ZERO,
        ))
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let active_tex = self.textures.iter().filter(|t| t.in_use).count();
        (self.meshes.len(), active_tex, self.sounds.len())
    }

    pub fn destroy(&self, gl: &GlFns) {
        for inst in self.instance_sets.iter().flatten() {
            gfx::destroy_instances(gl, inst);
        }
        for m in &self.meshes {
            gfx::destroy_mesh(gl, &m.gpu);
        }
        for t in &self.textures {
            // Skip embedded mesh textures (already freed by destroy_mesh)
            if t.in_use && (t.path.is_some() || t.gpu.id == self.textures[0].gpu.id || t.gpu.id == self.textures[1].gpu.id || t.gpu.id == self.textures[2].gpu.id) {
                gfx::destroy_texture(gl, t.gpu);
            }
        }
    }

    fn record_error(&mut self, msg: String) {
        eprintln!("[assets] ERROR: {msg}");
        self.last_error = msg;
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn make_checkerboard_8x8() -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8 {
        for x in 0..8 {
            let mag = ((x + y) & 1) == 0;
            if mag {
                out.extend_from_slice(&[235, 35, 220, 255]);
            } else {
                out.extend_from_slice(&[24, 24, 28, 255]);
            }
        }
    }
    out
}
