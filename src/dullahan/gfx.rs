//! Immediate-mode OpenGL 3.3 Core renderer:
//! - Directional 2048x2048 PCF shadow map pass
//! - PBR Metallic-Roughness + ORM + Emissive shader (up to 8 point lights + fog + ACES + hot GLSL)
//! - glTF 2.0 mesh rendering with per-draw named-node TRS overrides & sorted BLEND pass
//! - Lit solid debug boxes + 3D wireframe/grid/collider debug lines
//! - Batched 2D UI overlay (solid/bordered panels, progress bars, sprites, 9-slice, built-in bitmap font)
//! - Framebuffer PNG screenshot capture

use crate::dullahan::assets::{Assets, InstanceHandle, MeshHandle, TexHandle};
use crate::dullahan::font;
use crate::dullahan::formats::gltf::{AlphaMode, GltfCollider, GltfModel, GltfNode};
use crate::dullahan::formats::png;
use crate::dullahan::geom::HeightField;
use crate::dullahan::math::{Aabb, Color, Frustum, Mat4, Quat, Transform, Vec2, Vec3, clamp, deg_to_rad};
use crate::dullahan::platform::{
    GL_ARRAY_BUFFER, GL_BACK, GL_BLEND, GL_CCW, GL_CLAMP_TO_EDGE, GL_COLOR_ATTACHMENT0,
    GL_COLOR_BUFFER_BIT, GL_COMPILE_STATUS, GL_CULL_FACE, GL_DEPTH_ATTACHMENT,
    GL_DEPTH_BUFFER_BIT, GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT24, GL_DEPTH_TEST,
    GL_DYNAMIC_DRAW, GL_ELEMENT_ARRAY_BUFFER, GL_FALSE, GL_FILL, GL_FLOAT, GL_FRAGMENT_SHADER,
    GL_FRAMEBUFFER, GL_FRAMEBUFFER_COMPLETE, GL_FRONT, GL_FRONT_AND_BACK, GL_INFO_LOG_LENGTH,
    GL_LESS, GL_LINE, GL_LINEAR, GL_LINES, GL_LINK_STATUS, GL_NEAREST, GL_NONE,
    GL_ONE_MINUS_SRC_ALPHA, GL_PACK_ALIGNMENT, GL_RENDERBUFFER, GL_REPEAT, GL_RGBA, GL_RGBA8,
    GL_SRC_ALPHA, GL_STATIC_DRAW, GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_TEXTURE_MIN_FILTER,
    GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T, GL_TEXTURE0, GL_TEXTURE1, GL_TEXTURE2, GL_TEXTURE3,
    GL_TEXTURE4, GL_TRIANGLES, GL_TRUE, GL_UNPACK_ALIGNMENT, GL_UNSIGNED_BYTE, GL_UNSIGNED_INT,
    GL_VERTEX_SHADER, GLenum, GLint, GLsizei, GLsizeiptr, GLuint, GlFns, InputState, MouseButton,
};
use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::ptr;

pub const MAX_POINT_LIGHTS: usize = 8;
const SHADOW_MAP_SIZE: i32 = 2048;

// ---------------------------------------------------------------------------
// Camera, Lights, Overlay & Stats
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct Camera3D {
    pub pos: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self {
            pos: Vec3::new(8.0, 6.0, 10.0),
            target: Vec3::new(0.0, 1.0, 0.0),
            up: Vec3::Y,
            fov_deg: 55.0,
            near: 0.1,
            far: 500.0,
        }
    }
}

impl Camera3D {
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj = Mat4::perspective(deg_to_rad(self.fov_deg), aspect.max(0.01), self.near, self.far);
        let view = Mat4::look_at(self.pos, self.target, self.up);
        proj * view
    }
}

/// 2D world camera helper for pure 2D games (converts world units <-> pixel screen coords).
#[derive(Copy, Clone, Debug)]
pub struct Camera2D {
    pub pos: Vec2,
    pub zoom: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self { pos: Vec2::ZERO, zoom: 1.0 }
    }
}

impl Camera2D {
    pub fn world_to_screen(&self, world: Vec2, screen_w: f32, screen_h: f32) -> Vec2 {
        Vec2::new(
            (world.x - self.pos.x) * self.zoom + screen_w * 0.5,
            (world.y - self.pos.y) * self.zoom + screen_h * 0.5,
        )
    }

    pub fn screen_to_world(&self, screen: Vec2, screen_w: f32, screen_h: f32) -> Vec2 {
        let inv = 1.0 / self.zoom.max(1e-4);
        Vec2::new(
            (screen.x - screen_w * 0.5) * inv + self.pos.x,
            (screen.y - screen_h * 0.5) * inv + self.pos.y,
        )
    }
}

#[derive(Copy, Clone, Debug)]
pub struct PostSettings {
    pub enabled: bool,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub bloom_strength: f32,
    pub bloom_threshold: f32,
    pub chroma: f32,
    pub time: f32,
    pub tint: Color,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            exposure: 1.0,
            contrast: 1.02,
            saturation: 1.04,
            vignette: 0.14,
            bloom_strength: 0.22,
            bloom_threshold: 0.85,
            chroma: 0.0,
            time: 0.0,
            tint: Color::WHITE,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct PointLight {
    pub pos: Vec3,
    pub color: Color,
    pub radius: f32,
    pub intensity: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DebugOverlay {
    None,
    Grid,
    Wireframe,
    Colliders,
}

impl DebugOverlay {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Grid => "grid",
            Self::Wireframe => "wireframe",
            Self::Colliders => "colliders",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "grid" => Some(Self::Grid),
            "wire" | "wireframe" => Some(Self::Wireframe),
            "col" | "colliders" => Some(Self::Colliders),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub triangles: u32,
    pub lines: u32,
    pub ui_quads: u32,
    /// Mesh / instance-batch draw commands skipped by frustum culling (main pass).
    pub culled: u32,
    /// Instances drawn through `mesh_instances` (main pass).
    pub instances: u32,
}

// ---------------------------------------------------------------------------
// GPU Texture & PBR Mesh resources
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct GpuTexture {
    pub id: GLuint,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct GpuMaterial {
    pub base_color_factor: [f32; 4],
    pub base_color_tex: TexHandle,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub metallic_roughness_tex: TexHandle,
    pub occlusion_tex: TexHandle,
    pub emissive_factor: [f32; 3],
    pub emissive_tex: TexHandle,
    pub has_emissive_tex: bool,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl GpuMaterial {
    pub fn default_with(white: TexHandle, black: TexHandle) -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            base_color_tex: white,
            metallic_factor: 0.0,
            roughness_factor: 0.85,
            metallic_roughness_tex: white,
            occlusion_tex: white,
            emissive_factor: [0.0, 0.0, 0.0],
            emissive_tex: black,
            has_emissive_tex: false,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
struct PbrVertex {
    pos: [f32; 3],
    norm: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

pub struct GpuPrimitive {
    vao: GLuint,
    vbo: GLuint,
    ebo: GLuint,
    index_count: GLsizei,
    material_idx: usize,
}

pub struct GpuMesh {
    meshes: Vec<Vec<GpuPrimitive>>,
    pub materials: Vec<GpuMaterial>,
    owned_textures: Vec<GLuint>,
    pub nodes: Vec<GltfNode>,
    pub roots: Vec<usize>,
    pub colliders: Vec<GltfCollider>,
    pub bounds: Aabb,
}

impl GpuMesh {
    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    pub fn compute_node_matrices(&self, overrides: &[(usize, Transform)]) -> Vec<Mat4> {
        let mut world = vec![Mat4::IDENTITY; self.nodes.len()];
        fn visit(
            nodes: &[GltfNode],
            idx: usize,
            parent_m: Mat4,
            overrides: &[(usize, Transform)],
            world: &mut [Mat4],
        ) {
            let n = &nodes[idx];
            let local = if let Some((_, tr)) = overrides.iter().find(|(i, _)| *i == idx) {
                tr.to_mat4()
            } else if let Some(m) = n.raw_matrix {
                m
            } else {
                n.transform.to_mat4()
            };
            let w = parent_m * local;
            world[idx] = w;
            for &c in &n.children {
                if c < nodes.len() {
                    visit(nodes, c, w, overrides, world);
                }
            }
        }
        for &r in &self.roots {
            if r < self.nodes.len() {
                visit(&self.nodes, r, Mat4::IDENTITY, overrides, &mut world);
            }
        }
        world
    }
}

pub fn upload_texture_rgba(gl: &GlFns, width: u32, height: u32, rgba: &[u8], linear: bool) -> GpuTexture {
    let mut id = 0;
    let filt = if linear { GL_LINEAR } else { GL_NEAREST };
    unsafe {
        (gl.glGenTextures)(1, &mut id);
        (gl.glBindTexture)(GL_TEXTURE_2D, id);
        (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 1);
        (gl.glTexImage2D)(
            GL_TEXTURE_2D,
            0,
            GL_RGBA8 as GLint,
            width as GLsizei,
            height as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            rgba.as_ptr() as *const c_void,
        );
        (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filt);
        (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filt);
        (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
        (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
        (gl.glBindTexture)(GL_TEXTURE_2D, 0);
    }
    GpuTexture { id, width, height }
}

pub fn destroy_texture(gl: &GlFns, tex: GpuTexture) {
    if tex.id != 0 {
        unsafe { (gl.glDeleteTextures)(1, &tex.id) };
    }
}

fn upload_pbr_primitive(gl: &GlFns, verts: &[PbrVertex], indices: &[u32], material_idx: usize) -> GpuPrimitive {
    let mut vao = 0;
    let mut vbo = 0;
    let mut ebo = 0;
    unsafe {
        (gl.glGenVertexArrays)(1, &mut vao);
        (gl.glBindVertexArray)(vao);

        (gl.glGenBuffers)(1, &mut vbo);
        (gl.glBindBuffer)(GL_ARRAY_BUFFER, vbo);
        (gl.glBufferData)(
            GL_ARRAY_BUFFER,
            (verts.len() * std::mem::size_of::<PbrVertex>()) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );

        (gl.glGenBuffers)(1, &mut ebo);
        (gl.glBindBuffer)(GL_ELEMENT_ARRAY_BUFFER, ebo);
        (gl.glBufferData)(
            GL_ELEMENT_ARRAY_BUFFER,
            (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
            indices.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );

        let stride = std::mem::size_of::<PbrVertex>() as GLsizei;
        (gl.glEnableVertexAttribArray)(0);
        (gl.glVertexAttribPointer)(0, 3, GL_FLOAT, GL_FALSE, stride, ptr::null());
        (gl.glEnableVertexAttribArray)(1);
        (gl.glVertexAttribPointer)(1, 3, GL_FLOAT, GL_FALSE, stride, (3 * 4) as *const c_void);
        (gl.glEnableVertexAttribArray)(2);
        (gl.glVertexAttribPointer)(2, 2, GL_FLOAT, GL_FALSE, stride, (6 * 4) as *const c_void);
        (gl.glEnableVertexAttribArray)(3);
        (gl.glVertexAttribPointer)(3, 4, GL_FLOAT, GL_FALSE, stride, (8 * 4) as *const c_void);

        (gl.glBindVertexArray)(0);
    }
    GpuPrimitive {
        vao,
        vbo,
        ebo,
        index_count: indices.len() as GLsizei,
        material_idx,
    }
}

pub fn upload_gltf_mesh(
    gl: &GlFns,
    model: &GltfModel,
    materials: Vec<GpuMaterial>,
    owned_textures: Vec<GLuint>,
) -> GpuMesh {
    let mut gpu_meshes = Vec::with_capacity(model.meshes.len());
    for m in &model.meshes {
        let mut prims = Vec::with_capacity(m.primitives.len());
        for p in &m.primitives {
            let mut verts = Vec::with_capacity(p.positions.len());
            for i in 0..p.positions.len() {
                verts.push(PbrVertex {
                    pos: p.positions[i].to_array(),
                    norm: p.normals[i].to_array(),
                    uv: [p.uvs[i].x, p.uvs[i].y],
                    color: p.colors[i],
                });
            }
            let mat_idx = p.material.unwrap_or(0).min(materials.len().saturating_sub(1));
            prims.push(upload_pbr_primitive(gl, &verts, &p.indices, mat_idx));
        }
        gpu_meshes.push(prims);
    }

    GpuMesh {
        meshes: gpu_meshes,
        materials,
        owned_textures,
        nodes: model.nodes.clone(),
        roots: model.roots.clone(),
        colliders: model.colliders.clone(),
        bounds: model.bounds,
    }
}

pub fn create_fallback_mesh(gl: &GlFns, checker: TexHandle, white: TexHandle, black: TexHandle) -> GpuMesh {
    let raw = build_unit_cube_vertices();
    let mut verts = Vec::with_capacity(36);
    let mut indices = Vec::with_capacity(36);
    for i in 0..36 {
        let b = i * 6;
        let u = if (i % 6) == 1 || (i % 6) == 2 || (i % 6) == 4 { 1.0 } else { 0.0 };
        let v = if (i % 6) == 2 || (i % 6) == 4 || (i % 6) == 5 { 1.0 } else { 0.0 };
        verts.push(PbrVertex {
            pos: [raw[b], raw[b + 1], raw[b + 2]],
            norm: [raw[b + 3], raw[b + 4], raw[b + 5]],
            uv: [u, v],
            color: [1.0, 1.0, 1.0, 1.0],
        });
        indices.push(i as u32);
    }
    let mut mat = GpuMaterial::default_with(white, black);
    mat.base_color_tex = checker;
    let prim = upload_pbr_primitive(gl, &verts, &indices, 0);
    let node = GltfNode {
        name: "fallback".into(),
        parent: None,
        children: Vec::new(),
        mesh: Some(0),
        transform: Transform::from_pos(Vec3::new(0.0, 0.5, 0.0)),
        raw_matrix: None,
        is_collider: false,
    };
    GpuMesh {
        meshes: vec![vec![prim]],
        materials: vec![mat],
        owned_textures: Vec::new(),
        nodes: vec![node],
        roots: vec![0],
        colliders: Vec::new(),
        bounds: Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5)),
    }
}

/// Builds a lit PBR terrain `GpuMesh` from a `HeightField` with smooth normals and `[0..1]` UVs.
pub fn create_heightfield_mesh(
    gl: &GlFns,
    hf: &HeightField,
    albedo_tex: TexHandle,
    white: TexHandle,
    black: TexHandle,
    grid_x: u32,
    grid_z: u32,
) -> GpuMesh {
    let gx = grid_x.max(2);
    let gz = grid_z.max(2);
    let mut verts = Vec::with_capacity((gx * gz) as usize);
    let mut bmin = Vec3::splat(f32::INFINITY);
    let mut bmax = Vec3::splat(f32::NEG_INFINITY);

    for iz in 0..gz {
        let v = iz as f32 / (gz - 1) as f32;
        let wz = hf.origin.z + (v - 0.5) * hf.world_size.z;
        for ix in 0..gx {
            let u = ix as f32 / (gx - 1) as f32;
            let wx = hf.origin.x + (u - 0.5) * hf.world_size.x;
            let wy = hf.sample_height(wx, wz);
            let n = hf.sample_normal(wx, wz);
            let p = Vec3::new(wx, wy, wz);
            bmin = bmin.min(p);
            bmax = bmax.max(p);
            verts.push(PbrVertex {
                pos: p.to_array(),
                norm: n.to_array(),
                uv: [u, v],
                color: [1.0, 1.0, 1.0, 1.0],
            });
        }
    }

    let mut indices = Vec::with_capacity(((gx - 1) * (gz - 1) * 6) as usize);
    for iz in 0..(gz - 1) {
        for ix in 0..(gx - 1) {
            let i00 = iz * gx + ix;
            let i10 = i00 + 1;
            let i01 = i00 + gx;
            let i11 = i01 + 1;
            indices.extend_from_slice(&[i00, i01, i10, i10, i01, i11]);
        }
    }

    let mut mat = GpuMaterial::default_with(white, black);
    mat.base_color_tex = albedo_tex;
    mat.roughness_factor = 0.92;
    let prim = upload_pbr_primitive(gl, &verts, &indices, 0);
    let node = GltfNode {
        name: "terrain".into(),
        parent: None,
        children: Vec::new(),
        mesh: Some(0),
        transform: Transform::IDENTITY,
        raw_matrix: None,
        is_collider: false,
    };
    GpuMesh {
        meshes: vec![vec![prim]],
        materials: vec![mat],
        owned_textures: Vec::new(),
        nodes: vec![node],
        roots: vec![0],
        colliders: Vec::new(),
        bounds: Aabb::new(bmin, bmax),
    }
}

/// Static per-instance transforms on the GPU (one column-major `Mat4` per instance),
/// drawn with `DrawList::mesh_instances`. Instances should use uniform scale: the
/// instanced shader path transforms normals with the model matrix's upper 3x3.
pub struct GpuInstances {
    vbo: GLuint,
    pub count: u32,
    /// Bounds of the instance origins (translation parts).
    pub origin_bounds: Aabb,
    /// Largest axis scale of any instance, used to grow `origin_bounds` by the mesh size.
    pub max_scale: f32,
}

pub fn upload_instances(gl: &GlFns, transforms: &[Mat4]) -> GpuInstances {
    let mut inst = GpuInstances { vbo: 0, count: 0, origin_bounds: Aabb::new(Vec3::ZERO, Vec3::ZERO), max_scale: 0.0 };
    unsafe { (gl.glGenBuffers)(1, &mut inst.vbo) };
    write_instances(gl, &mut inst, transforms);
    inst
}

pub fn write_instances(gl: &GlFns, inst: &mut GpuInstances, transforms: &[Mat4]) {
    let mut bounds: Option<Aabb> = None;
    let mut max_scale = 0.0f32;
    for t in transforms {
        let o = Vec3::new(t.m[12], t.m[13], t.m[14]);
        match bounds.as_mut() {
            Some(b) => b.include(o),
            None => bounds = Some(Aabb::new(o, o)),
        }
        for c in 0..3 {
            let l = (t.m[c * 4] * t.m[c * 4] + t.m[c * 4 + 1] * t.m[c * 4 + 1] + t.m[c * 4 + 2] * t.m[c * 4 + 2]).sqrt();
            max_scale = max_scale.max(l);
        }
    }
    inst.count = transforms.len() as u32;
    inst.origin_bounds = bounds.unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO));
    inst.max_scale = max_scale;
    unsafe {
        (gl.glBindBuffer)(GL_ARRAY_BUFFER, inst.vbo);
        (gl.glBufferData)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(transforms) as GLsizeiptr,
            transforms.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        (gl.glBindBuffer)(GL_ARRAY_BUFFER, 0);
    }
}

/// Draws `prim` (its VAO must be bound) once per instance, feeding `inst`'s matrices to
/// attribute locations 4-7, then restores the VAO's attribute state.
unsafe fn draw_elements_instanced(gl: &GlFns, prim: &GpuPrimitive, inst: &GpuInstances) {
    unsafe {
        (gl.glBindBuffer)(GL_ARRAY_BUFFER, inst.vbo);
        for col in 0..4u32 {
            let loc = 4 + col;
            (gl.glEnableVertexAttribArray)(loc);
            (gl.glVertexAttribPointer)(loc, 4, GL_FLOAT, GL_FALSE, 64, (col as usize * 16) as *const c_void);
            (gl.glVertexAttribDivisor)(loc, 1);
        }
        (gl.glDrawElementsInstanced)(GL_TRIANGLES, prim.index_count, GL_UNSIGNED_INT, ptr::null(), inst.count as GLsizei);
        for loc in 4..8u32 {
            (gl.glVertexAttribDivisor)(loc, 0);
            (gl.glDisableVertexAttribArray)(loc);
        }
        (gl.glBindBuffer)(GL_ARRAY_BUFFER, 0);
    }
}

/// Inserts `#define <name>` right after the `#version` line of a GLSL source.
fn with_define(src: &str, name: &str) -> String {
    match src.find('\n') {
        Some(i) if src.trim_start().starts_with("#version") => format!("{}\n#define {name}{}", &src[..i], &src[i..]),
        _ => format!("#define {name}\n{src}"),
    }
}

pub fn destroy_instances(gl: &GlFns, inst: &GpuInstances) {
    unsafe { (gl.glDeleteBuffers)(1, &inst.vbo) };
}

impl GpuInstances {
    /// Conservative world bounds of all instances of a mesh with local bounds `mesh_bounds`.
    pub fn world_bounds(&self, mesh_bounds: Aabb, model: Mat4) -> Aabb {
        let h = mesh_bounds.half_extents();
        let c = mesh_bounds.center();
        let r = (h.len() + c.len()) * self.max_scale;
        let grown = Aabb::new(
            self.origin_bounds.min - Vec3::new(r, r, r),
            self.origin_bounds.max + Vec3::new(r, r, r),
        );
        grown.transformed(model)
    }
}

pub fn destroy_mesh(gl: &GlFns, mesh: &GpuMesh) {
    unsafe {
        for prims in &mesh.meshes {
            for p in prims {
                (gl.glDeleteVertexArrays)(1, &p.vao);
                (gl.glDeleteBuffers)(1, &p.vbo);
                (gl.glDeleteBuffers)(1, &p.ebo);
            }
        }
        for &tex_id in &mesh.owned_textures {
            if tex_id != 0 {
                (gl.glDeleteTextures)(1, &tex_id);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Immediate-mode DrawList (3D + 2D UI)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct SolidBoxCmd {
    model: Mat4,
    color: Color,
    cast_shadow: bool,
}

#[derive(Clone)]
struct InstancedDrawCmd {
    handle: MeshHandle,
    instances: InstanceHandle,
    model: Mat4,
    tint: Color,
}

struct MeshDrawCmd {
    handle: MeshHandle,
    model: Mat4,
    tint: Color,
    node_overrides: Vec<(usize, Transform)>,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct LineVertex {
    pos: [f32; 3],
    color: [f32; 4],
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum UiTex {
    White,
    Font,
    Asset(TexHandle),
}

#[derive(Copy, Clone)]
struct UiQuadCmd {
    tex: UiTex,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    angle: f32,
    color: Color,
}

#[derive(Copy, Clone)]
struct BillboardCmd {
    tex: Option<TexHandle>,
    pos: Vec3,
    /// `None` = camera-facing billboard; `Some((right_dir, up_dir))` = fixed world-oriented 3D quad/decal.
    axes: Option<(Vec3, Vec3)>,
    size: Vec2,
    color: Color,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct BillboardVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct UiVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

pub struct DrawList {
    pub camera: Camera3D,
    pub clear_color: Color,
    /// Optional vertical sky backdrop `(zenith_color, horizon_color)` drawn behind the 3D scene.
    pub sky_gradient: Option<(Color, Color)>,
    pub sun_dir: Vec3,
    pub sun_color: Color,
    pub sky_color: Color,
    pub ground_color: Color,
    pub fog_color: Color,
    pub fog_start: f32,
    pub fog_end: f32,
    pub shadows_enabled: bool,
    pub shadow_extent: f32,
    pub post: PostSettings,
    pub overlay: DebugOverlay,
    /// Skip mesh draws whose bounds are outside the camera (main pass) or light (shadow pass)
    /// frustum. Draws with node overrides are never culled, since animated nodes may leave
    /// the rest-pose bounds.
    pub frustum_culling: bool,
    point_lights: Vec<PointLight>,
    meshes: Vec<MeshDrawCmd>,
    instanced: Vec<InstancedDrawCmd>,
    boxes: Vec<SolidBoxCmd>,
    billboards: Vec<BillboardCmd>,
    lines: Vec<LineVertex>,
    collider_lines: Vec<LineVertex>,
    ui_quads: Vec<UiQuadCmd>,
}

impl Default for DrawList {
    fn default() -> Self {
        Self {
            camera: Camera3D::default(),
            clear_color: Color::from_hex(0x95B8DC),
            sky_gradient: None,
            sun_dir: Vec3::new(0.45, 0.82, 0.36).norm(),
            sun_color: Color::rgb(1.0, 0.96, 0.88),
            sky_color: Color::rgb(0.42, 0.56, 0.72),
            ground_color: Color::rgb(0.22, 0.25, 0.18),
            fog_color: Color::from_hex(0x95B8DC),
            fog_start: 45.0,
            fog_end: 180.0,
            shadows_enabled: true,
            shadow_extent: 18.0,
            post: PostSettings::default(),
            overlay: DebugOverlay::None,
            frustum_culling: true,
            point_lights: Vec::new(),
            meshes: Vec::new(),
            instanced: Vec::new(),
            boxes: Vec::new(),
            billboards: Vec::new(),
            lines: Vec::new(),
            collider_lines: Vec::new(),
            ui_quads: Vec::new(),
        }
    }
}

impl DrawList {
    pub fn clear(&mut self) {
        self.point_lights.clear();
        self.meshes.clear();
        self.instanced.clear();
        self.boxes.clear();
        self.billboards.clear();
        self.lines.clear();
        self.collider_lines.clear();
        self.ui_quads.clear();
    }

    /// Camera-facing 3D billboard quad (particles, dust puffs, sparks, floating markers).
    pub fn billboard(&mut self, tex: Option<TexHandle>, pos: Vec3, size: Vec2, color: Color) {
        self.billboards.push(BillboardCmd { tex, pos, axes: None, size, color });
    }

    /// World-oriented 3D textured quad (surface decals, wall posters, painted tile emblems).
    /// `right_dir` points along +U and `up_dir` points along -V (top of the texture).
    pub fn quad_3d(
        &mut self,
        tex: Option<TexHandle>,
        pos: Vec3,
        right_dir: Vec3,
        up_dir: Vec3,
        size: Vec2,
        color: Color,
    ) {
        self.billboards.push(BillboardCmd {
            tex,
            pos,
            axes: Some((right_dir.norm(), up_dir.norm())),
            size,
            color,
        });
    }

    /// Flat horizontal 3D decal painted on the XZ plane facing +Y (great for floor/tile emblems).
    pub fn decal_flat(&mut self, tex: Option<TexHandle>, pos: Vec3, size: Vec2, yaw: f32, color: Color) {
        let (s, c) = yaw.sin_cos();
        let right_dir = Vec3::new(c, 0.0, -s);
        let up_dir = Vec3::new(-s, 0.0, -c);
        self.quad_3d(tex, pos, right_dir, up_dir, size, color);
    }

    pub fn point_light(&mut self, pos: Vec3, color: Color, radius: f32, intensity: f32) {
        if self.point_lights.len() < MAX_POINT_LIGHTS {
            self.point_lights.push(PointLight { pos, color, radius, intensity });
        }
    }

    /// Draws `handle` once per transform in `instances` (created with `Assets::create_instances`),
    /// all placed by `transform`, in a single draw call per primitive. Opaque materials only;
    /// instances should use uniform scale.
    pub fn mesh_instances(&mut self, handle: MeshHandle, instances: InstanceHandle, transform: Transform, tint: Color) {
        self.instanced.push(InstancedDrawCmd { handle, instances, model: transform.to_mat4(), tint });
    }

    pub fn mesh(&mut self, handle: MeshHandle, transform: Transform) {
        self.meshes.push(MeshDrawCmd {
            handle,
            model: transform.to_mat4(),
            tint: Color::WHITE,
            node_overrides: Vec::new(),
        });
    }

    pub fn mesh_tinted(&mut self, handle: MeshHandle, transform: Transform, tint: Color) {
        self.meshes.push(MeshDrawCmd {
            handle,
            model: transform.to_mat4(),
            tint,
            node_overrides: Vec::new(),
        });
    }

    pub fn mesh_nodes(&mut self, handle: MeshHandle, transform: Transform, overrides: &[(usize, Transform)]) {
        self.meshes.push(MeshDrawCmd {
            handle,
            model: transform.to_mat4(),
            tint: Color::WHITE,
            node_overrides: overrides.to_vec(),
        });
    }

    pub fn solid_box(&mut self, transform: Transform, color: Color) {
        self.boxes.push(SolidBoxCmd {
            model: transform.to_mat4(),
            color,
            cast_shadow: true,
        });
    }

    pub fn solid_plane(&mut self, y: f32, half_extent: f32, color: Color) {
        let t = Transform {
            pos: Vec3::new(0.0, y - 0.05, 0.0),
            rot: Quat::IDENTITY,
            scale: Vec3::new(half_extent * 2.0, 0.1, half_extent * 2.0),
        };
        self.boxes.push(SolidBoxCmd {
            model: t.to_mat4(),
            color,
            cast_shadow: false,
        });
    }

    pub fn debug_line(&mut self, a: Vec3, b: Vec3, color: Color) {
        let c = color.to_array();
        self.lines.push(LineVertex { pos: a.to_array(), color: c });
        self.lines.push(LineVertex { pos: b.to_array(), color: c });
    }

    pub fn collider_box(&mut self, center: Vec3, half: Vec3, color: Color) {
        push_wire_box(&mut self.collider_lines, center, half, color);
    }

    pub fn debug_box(&mut self, center: Vec3, half: Vec3, color: Color) {
        push_wire_box(&mut self.lines, center, half, color);
    }

    pub fn debug_axes(&mut self, origin: Vec3, len: f32) {
        self.debug_line(origin, origin + Vec3::X * len, Color::RED);
        self.debug_line(origin, origin + Vec3::Y * len, Color::GREEN);
        self.debug_line(origin, origin + Vec3::Z * len, Color::BLUE);
    }

    pub fn debug_grid(&mut self, half_extent: i32, step: f32, color: Color, axis_color: Color) {
        let max_coord = half_extent as f32 * step;
        let y = 0.01;
        for i in -half_extent..=half_extent {
            let p = i as f32 * step;
            let c = if i == 0 { axis_color } else { color };
            self.debug_line(Vec3::new(p, y, -max_coord), Vec3::new(p, y, max_coord), c);
            self.debug_line(Vec3::new(-max_coord, y, p), Vec3::new(max_coord, y, p), c);
        }
    }

    // --- 2D / UI Immediate-Mode API (pixel coords, top-left origin) ---

    pub fn ui_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.ui_quads.push(UiQuadCmd {
            tex: UiTex::White,
            x, y, w, h,
            u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0,
            angle: 0.0,
            color,
        });
    }

    /// Draws a solid 2D rectangle centered at `(cx, cy)` rotated clockwise by `angle_rad`.
    pub fn ui_rect_rot(&mut self, cx: f32, cy: f32, w: f32, h: f32, angle_rad: f32, color: Color) {
        self.ui_quads.push(UiQuadCmd {
            tex: UiTex::White,
            x: cx - w * 0.5,
            y: cy - h * 0.5,
            w,
            h,
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            angle: angle_rad,
            color,
        });
    }

    /// Draws a crisp thick 2D line segment from `a` to `b` (great for aiming trajectories, walls, and 2D debug draw).
    pub fn ui_line(&mut self, a: Vec2, b: Vec2, thickness: f32, color: Color) {
        let d = b - a;
        let len = d.len();
        if len < 0.25 {
            return;
        }
        let mid = (a + b) * 0.5;
        let angle = d.y.atan2(d.x);
        self.ui_rect_rot(mid.x, mid.y, len + thickness * 0.25, thickness, angle, color);
    }

    /// Draws a 2D circular outline (useful for pond ripples, aiming rings, and 2D collider overlays).
    pub fn ui_circle_outline(&mut self, cx: f32, cy: f32, radius: f32, thickness: f32, segments: u32, color: Color) {
        let segs = segments.max(8);
        let step = std::f32::consts::TAU / segs as f32;
        for i in 0..segs {
            let a0 = i as f32 * step;
            let a1 = (i + 1) as f32 * step;
            let p0 = Vec2::new(cx + a0.cos() * radius, cy + a0.sin() * radius);
            let p1 = Vec2::new(cx + a1.cos() * radius, cy + a1.sin() * radius);
            self.ui_line(p0, p1, thickness, color);
        }
    }

    pub fn ui_panel(&mut self, x: f32, y: f32, w: f32, h: f32, bg: Color, border: Color, border_px: f32) {
        self.ui_rect(x, y, w, h, bg);
        if border_px > 0.0 {
            self.ui_rect(x, y, w, border_px, border);
            self.ui_rect(x, y + h - border_px, w, border_px, border);
            self.ui_rect(x, y + border_px, border_px, (h - border_px * 2.0).max(0.0), border);
            self.ui_rect(x + w - border_px, y + border_px, border_px, (h - border_px * 2.0).max(0.0), border);
        }
    }

    /// Immediate-mode clickable button with hover/press feedback. Returns `true` on left click.
    pub fn ui_button(&mut self, input: &InputState, x: f32, y: f32, w: f32, h: f32, label: &str) -> bool {
        let m = input.mouse_pos;
        let hovered = m.x >= x && m.x <= x + w && m.y >= y && m.y <= y + h;
        let pressed = hovered && input.mouse_down(MouseButton::Left);
        let clicked = hovered && input.mouse_pressed(MouseButton::Left);

        let bg = if pressed {
            Color::rgba(0.24, 0.36, 0.52, 0.94)
        } else if hovered {
            Color::rgba(0.16, 0.22, 0.31, 0.92)
        } else {
            Color::rgba(0.09, 0.11, 0.15, 0.85)
        };
        let border = if hovered {
            Color::rgba(0.58, 0.72, 0.90, 0.95)
        } else {
            Color::rgba(0.32, 0.40, 0.50, 0.90)
        };
        self.ui_panel(x, y, w, h, bg, border, 2.0);
        let (tw, th) = self.ui_text_measure(2.0, label);
        let tx = x + (w - tw) * 0.5;
        let ty = y + (h - th) * 0.5 + 1.0;
        self.ui_text(tx, ty, 2.0, Color::rgb(0.95, 0.97, 1.0), label);
        clicked
    }

    pub fn ui_bar(&mut self, x: f32, y: f32, w: f32, h: f32, fraction: f32, bg: Color, fill: Color) {
        self.ui_rect(x, y, w, h, bg);
        let fw = w * clamp(fraction, 0.0, 1.0);
        if fw > 0.5 {
            self.ui_rect(x, y, fw, h, fill);
        }
    }

    /// Interactive horizontal slider (click or drag). Mutates `value` within `[min, max]` and returns `true` if changed.
    #[allow(clippy::too_many_arguments)]
    pub fn ui_slider(
        &mut self,
        input: &InputState,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        min: f32,
        max: f32,
        value: &mut f32,
    ) -> bool {
        let span = (max - min).max(1e-5);
        let m = input.mouse_pos;
        let pad = 6.0;
        let hovered = m.x >= x - pad && m.x <= x + w + pad && m.y >= y - pad && m.y <= y + h + pad;
        let active = hovered && input.mouse_down(MouseButton::Left);
        let mut changed = false;
        if active {
            let frac = clamp((m.x - x) / w.max(1.0), 0.0, 1.0);
            let next = min + frac * span;
            if (next - *value).abs() > 1e-4 {
                *value = next;
                changed = true;
            }
        }
        let frac = clamp((*value - min) / span, 0.0, 1.0);
        let track_h = (h * 0.42).max(6.0);
        let track_y = y + (h - track_h) * 0.5;
        let border = if hovered || active {
            Color::rgba(0.58, 0.85, 0.96, 0.95)
        } else {
            Color::rgba(0.32, 0.40, 0.50, 0.90)
        };
        let fill = if active {
            Color::from_hex(0x44FAFF)
        } else {
            Color::from_hex(0x38F5A8)
        };
        self.ui_panel(x, track_y, w, track_h, Color::rgba(0.08, 0.10, 0.15, 0.92), border, 1.5);
        let fw = (w - 4.0).max(0.0) * frac;
        if fw > 0.5 {
            self.ui_rect(x + 2.0, track_y + 2.0, fw, (track_h - 4.0).max(2.0), fill);
        }
        let knob_w = 12.0;
        let knob_x = x + frac * w - knob_w * 0.5;
        self.ui_panel(
            knob_x,
            y,
            knob_w,
            h,
            if active { Color::WHITE } else { Color::from_hex(0xF0BE3C) },
            Color::rgba(0.06, 0.07, 0.11, 0.95),
            1.5,
        );
        changed
    }

    pub fn ui_sprite(&mut self, tex: TexHandle, x: f32, y: f32, w: f32, h: f32, tint: Color) {
        self.ui_quads.push(UiQuadCmd {
            tex: UiTex::Asset(tex),
            x, y, w, h,
            u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0,
            angle: 0.0,
            color: tint,
        });
    }

    /// Draws a 2D sprite centered at `(cx, cy)` rotated clockwise by `angle_rad` (great for compasses & 2D games).
    pub fn ui_sprite_rot(&mut self, tex: TexHandle, cx: f32, cy: f32, w: f32, h: f32, angle_rad: f32, tint: Color) {
        self.ui_quads.push(UiQuadCmd {
            tex: UiTex::Asset(tex),
            x: cx - w * 0.5,
            y: cy - h * 0.5,
            w,
            h,
            u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0,
            angle: angle_rad,
            color: tint,
        });
    }

    /// Scalable 9-slice UI panel using `border_px` (pixel border in source texture) rendered at `draw_border`.
    pub fn ui_nine_slice(
        &mut self,
        tex: TexHandle,
        tex_w: u32,
        tex_h: u32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border_px: f32,
        tint: Color,
    ) {
        let tw = (tex_w.max(1)) as f32;
        let th = (tex_h.max(1)) as f32;
        let b = border_px.min(w * 0.45).min(h * 0.45);
        let bu = (border_px / tw).min(0.45);
        let bv = (border_px / th).min(0.45);

        let xs = [x, x + b, x + w - b, x + w];
        let ys = [y, y + b, y + h - b, y + h];
        let us = [0.0, bu, 1.0 - bu, 1.0];
        let vs = [0.0, bv, 1.0 - bv, 1.0];

        for row in 0..3 {
            for col in 0..3 {
                let qw = xs[col + 1] - xs[col];
                let qh = ys[row + 1] - ys[row];
                if qw <= 0.0 || qh <= 0.0 {
                    continue;
                }
                self.ui_quads.push(UiQuadCmd {
                    tex: UiTex::Asset(tex),
                    x: xs[col],
                    y: ys[row],
                    w: qw,
                    h: qh,
                    u0: us[col],
                    v0: vs[row],
                    u1: us[col + 1],
                    v1: vs[row + 1],
                    angle: 0.0,
                    color: tint,
                });
            }
        }
    }

    pub fn ui_text_measure(&self, scale: f32, text: &str) -> (f32, f32) {
        let adv_x = (font::GLYPH_W as f32 + 1.0) * scale;
        let line_h = (font::GLYPH_H as f32 + 2.0) * scale;
        let mut max_w = 0.0f32;
        let mut lines = 1usize;
        let mut cur = 0usize;
        for ch in text.chars() {
            if ch == '\n' {
                max_w = max_w.max(cur as f32 * adv_x);
                cur = 0;
                lines += 1;
            } else {
                cur += 1;
            }
        }
        max_w = max_w.max(cur as f32 * adv_x);
        (max_w, lines as f32 * line_h)
    }

    pub fn ui_text(&mut self, x: f32, y: f32, scale: f32, color: Color, text: &str) {
        let gw = font::GLYPH_W as f32 * scale;
        let gh = font::GLYPH_H as f32 * scale;
        let adv_x = (font::GLYPH_W as f32 + 1.0) * scale;
        let adv_y = (font::GLYPH_H as f32 + 2.0) * scale;
        let mut cx = x;
        let mut cy = y;
        for ch in text.chars() {
            if ch == '\n' {
                cx = x;
                cy += adv_y;
                continue;
            }
            if ch != ' ' {
                let (u0, v0, u1, v1) = font::glyph_uv(ch);
                self.ui_quads.push(UiQuadCmd {
                    tex: UiTex::Font,
                    x: cx,
                    y: cy,
                    w: gw,
                    h: gh,
                    u0, v0, u1, v1,
                    angle: 0.0,
                    color,
                });
            }
            cx += adv_x;
        }
    }
}

fn push_wire_box(dst: &mut Vec<LineVertex>, center: Vec3, half: Vec3, color: Color) {
    let min = center - half;
    let max = center + half;
    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];
    push_wire_corners(dst, &corners, color);
}

fn push_wire_aabb_transformed(dst: &mut Vec<LineVertex>, aabb: Aabb, model: Mat4, color: Color) {
    let min = aabb.min;
    let max = aabb.max;
    let corners = [
        model.transform_point(Vec3::new(min.x, min.y, min.z)),
        model.transform_point(Vec3::new(max.x, min.y, min.z)),
        model.transform_point(Vec3::new(max.x, min.y, max.z)),
        model.transform_point(Vec3::new(min.x, min.y, max.z)),
        model.transform_point(Vec3::new(min.x, max.y, min.z)),
        model.transform_point(Vec3::new(max.x, max.y, min.z)),
        model.transform_point(Vec3::new(max.x, max.y, max.z)),
        model.transform_point(Vec3::new(min.x, max.y, max.z)),
    ];
    push_wire_corners(dst, &corners, color);
}

fn push_wire_corners(dst: &mut Vec<LineVertex>, corners: &[Vec3; 8], color: Color) {
    const EDGES: [(usize, usize); 12] = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    let c = color.to_array();
    for (a, b) in EDGES {
        dst.push(LineVertex { pos: corners[a].to_array(), color: c });
        dst.push(LineVertex { pos: corners[b].to_array(), color: c });
    }
}

// ---------------------------------------------------------------------------
// Shaders (GLSL 330 Core)
// ---------------------------------------------------------------------------

const SHADOW_VERT: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
#ifdef INSTANCED
layout(location = 4) in mat4 a_inst;
#endif
uniform mat4 u_light_vp;
uniform mat4 u_model;
void main() {
#ifdef INSTANCED
    mat4 model = u_model * a_inst;
#else
    mat4 model = u_model;
#endif
    gl_Position = u_light_vp * model * vec4(a_pos, 1.0);
}
"#;

const SHADOW_FRAG: &str = r#"#version 330 core
void main() {}
"#;

pub const DEFAULT_PBR_VERT: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_norm;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;
#ifdef INSTANCED
// Per-instance transform (locations 4-7). Compiled with `#define INSTANCED` for mesh_instances.
layout(location = 4) in mat4 a_inst;
#endif

uniform mat4 u_view_proj;
uniform mat4 u_model;
uniform mat4 u_light_vp;

out vec3 v_world_pos;
out vec3 v_world_norm;
out vec2 v_uv;
out vec4 v_color;
out vec4 v_light_pos;

void main() {
#ifdef INSTANCED
    mat4 model = u_model * a_inst;
    vec3 n = mat3(model) * a_norm; // instances use uniform scale
#else
    mat4 model = u_model;
    vec3 n = mat3(transpose(inverse(model))) * a_norm;
#endif
    vec4 wp = model * vec4(a_pos, 1.0);
    v_world_pos = wp.xyz;
    v_world_norm = normalize(n);
    v_uv = a_uv;
    v_color = a_color;
    v_light_pos = u_light_vp * wp;
    gl_Position = u_view_proj * wp;
}
"#;

pub const DEFAULT_PBR_FRAG: &str = r#"#version 330 core
in vec3 v_world_pos;
in vec3 v_world_norm;
in vec2 v_uv;
in vec4 v_color;
in vec4 v_light_pos;

uniform sampler2D u_base_tex;
uniform sampler2D u_mr_tex;
uniform sampler2D u_occ_tex;
uniform sampler2D u_emissive_tex;
uniform sampler2D u_shadow_map;

uniform vec4 u_base_factor;
uniform vec4 u_tint;
uniform float u_metallic_factor;
uniform float u_roughness_factor;
uniform vec3 u_emissive_factor;
uniform float u_has_emissive_tex;
uniform float u_alpha_cutoff;
uniform float u_shadows_enabled;

uniform vec3 u_cam_pos;
uniform vec3 u_sun_dir;
uniform vec3 u_sun_color;
uniform vec3 u_sky_color;
uniform vec3 u_ground_color;
uniform vec3 u_fog_color;
uniform float u_fog_start;
uniform float u_fog_end;

uniform int u_point_count;
uniform vec4 u_point_pos_rad[8];
uniform vec4 u_point_col_int[8];

out vec4 o_color;

const float PI = 3.14159265359;

vec3 to_linear(vec3 c) { return pow(max(c, vec3(0.0)), vec3(2.2)); }
vec3 to_srgb(vec3 c) { return pow(max(c, vec3(0.0)), vec3(1.0 / 2.2)); }

vec3 aces_tonemap(vec3 x) {
    float a = 2.51, b = 0.03, c = 2.43, d = 0.59, e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

float calc_shadow(vec4 lp, vec3 N, vec3 L) {
    if (u_shadows_enabled < 0.5) return 1.0;
    vec3 p = lp.xyz / lp.w * 0.5 + 0.5;
    if (p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 || p.z > 1.0) return 1.0;
    float bias = max(0.0028 * (1.0 - max(dot(N, L), 0.0)), 0.0008);
    vec2 texel = 1.0 / vec2(textureSize(u_shadow_map, 0));
    float lit = 0.0;
    for (int y = -1; y <= 1; ++y) {
        for (int x = -1; x <= 1; ++x) {
            float d = texture(u_shadow_map, p.xy + vec2(x, y) * texel).r;
            lit += (p.z - bias <= d) ? 1.0 : 0.0;
        }
    }
    return lit / 9.0;
}

vec3 eval_pbr_light(vec3 N, vec3 V, vec3 L, vec3 radiance, vec3 albedo, float metallic, float rough, vec3 F0) {
    float NdotL = max(dot(N, L), 0.0);
    if (NdotL <= 0.0) return vec3(0.0);
    vec3 H = normalize(V + L);
    float NdotV = max(dot(N, V), 0.001);
    float NdotH = max(dot(N, H), 0.0);
    float HdotV = max(dot(H, V), 0.0);

    float a = rough * rough;
    float a2 = a * a;
    float denom = NdotH * NdotH * (a2 - 1.0) + 1.0;
    float D = a2 / (PI * denom * denom + 1e-5);
    vec3 F = F0 + (1.0 - F0) * pow(1.0 - HdotV, 5.0);
    float k = (rough + 1.0) * (rough + 1.0) * 0.125;
    float G = (NdotV / (NdotV * (1.0 - k) + k)) * (NdotL / (NdotL * (1.0 - k) + k));

    vec3 spec = (D * G * F) / (4.0 * NdotV * NdotL + 1e-4);
    vec3 kD = (vec3(1.0) - F) * (1.0 - metallic);
    return (kD * albedo / PI + spec) * radiance * NdotL;
}

void main() {
    vec4 tex_base = texture(u_base_tex, v_uv);
    float alpha = tex_base.a * u_base_factor.a * v_color.a * u_tint.a;
    if (u_alpha_cutoff > 0.0 && alpha < u_alpha_cutoff) discard;

    vec3 albedo = to_linear(tex_base.rgb) * to_linear(u_base_factor.rgb) * to_linear(v_color.rgb) * to_linear(u_tint.rgb);
    vec4 mr = texture(u_mr_tex, v_uv);
    float roughness = clamp(mr.g * u_roughness_factor, 0.06, 1.0);
    float metallic = clamp(mr.b * u_metallic_factor, 0.0, 1.0);
    float ao = texture(u_occ_tex, v_uv).r;

    vec3 em_sample = u_has_emissive_tex > 0.5 ? to_linear(texture(u_emissive_tex, v_uv).rgb) : vec3(1.0);
    vec3 emissive = to_linear(u_emissive_factor) * em_sample;

    vec3 N = normalize(gl_FrontFacing ? v_world_norm : -v_world_norm);
    vec3 V = normalize(u_cam_pos - v_world_pos);
    vec3 L = normalize(u_sun_dir);
    vec3 F0 = mix(vec3(0.04), albedo, metallic);

    float shadow = calc_shadow(v_light_pos, N, L);
    vec3 Lo = eval_pbr_light(N, V, L, to_linear(u_sun_color) * 2.6, albedo, metallic, roughness, F0) * shadow;

    for (int i = 0; i < u_point_count; ++i) {
        vec3 to_l = u_point_pos_rad[i].xyz - v_world_pos;
        float dist = length(to_l);
        float rad = max(u_point_pos_rad[i].w, 0.001);
        if (dist < rad) {
            float att = clamp(1.0 - dist / rad, 0.0, 1.0);
            att = att * att * u_point_col_int[i].w;
            Lo += eval_pbr_light(N, V, to_l / max(dist, 1e-4), to_linear(u_point_col_int[i].rgb) * att * 2.0, albedo, metallic, roughness, F0);
        }
    }

    float hemi = N.y * 0.5 + 0.5;
    vec3 amb_light = mix(to_linear(u_ground_color), to_linear(u_sky_color), hemi);
    vec3 amb_diff = amb_light * albedo * (1.0 - metallic * 0.85) * 0.65;
    vec3 R = reflect(-V, N);
    vec3 env_spec = mix(to_linear(u_ground_color), to_linear(u_sky_color), R.y * 0.5 + 0.5) * F0 * (1.0 - roughness * 0.7) * 0.55;
    vec3 color = (amb_diff + env_spec) * ao + Lo + emissive;

    vec3 fog_lin = to_linear(u_fog_color);
    float dist = length(u_cam_pos - v_world_pos);
    float fog = clamp((dist - u_fog_start) / max(u_fog_end - u_fog_start, 0.001), 0.0, 1.0);
    color = mix(aces_tonemap(color), fog_lin, fog);
    o_color = vec4(to_srgb(color), alpha);
}
"#;

const SOLID_VERT: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_norm;

uniform mat4 u_view_proj;
uniform mat4 u_model;
uniform mat4 u_light_vp;

out vec3 v_world_pos;
out vec3 v_world_norm;
out vec4 v_light_pos;

void main() {
    vec4 wp = u_model * vec4(a_pos, 1.0);
    v_world_pos = wp.xyz;
    v_world_norm = normalize(mat3(u_model) * a_norm);
    v_light_pos = u_light_vp * wp;
    gl_Position = u_view_proj * wp;
}
"#;

const SOLID_FRAG: &str = r#"#version 330 core
in vec3 v_world_pos;
in vec3 v_world_norm;
in vec4 v_light_pos;

uniform sampler2D u_shadow_map;
uniform float u_shadows_enabled;
uniform vec4 u_color;
uniform vec3 u_sun_dir;
uniform vec3 u_sun_color;
uniform vec3 u_sky_color;
uniform vec3 u_ground_color;
uniform vec3 u_cam_pos;
uniform vec3 u_fog_color;
uniform float u_fog_start;
uniform float u_fog_end;

out vec4 o_color;

float calc_shadow(vec4 lp, vec3 N, vec3 L) {
    if (u_shadows_enabled < 0.5) return 1.0;
    vec3 p = lp.xyz / lp.w * 0.5 + 0.5;
    if (p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 || p.z > 1.0) return 1.0;
    float bias = max(0.0025 * (1.0 - max(dot(N, L), 0.0)), 0.0008);
    vec2 texel = 1.0 / vec2(textureSize(u_shadow_map, 0));
    float lit = 0.0;
    for (int y = -1; y <= 1; ++y) {
        for (int x = -1; x <= 1; ++x) {
            float d = texture(u_shadow_map, p.xy + vec2(x, y) * texel).r;
            lit += (p.z - bias <= d) ? 1.0 : 0.0;
        }
    }
    return lit / 9.0;
}

void main() {
    vec3 n = normalize(v_world_norm);
    vec3 l = normalize(u_sun_dir);
    vec3 v = normalize(u_cam_pos - v_world_pos);
    vec3 h = normalize(l + v);

    float shadow = calc_shadow(v_light_pos, n, l);
    float ndl = max(dot(n, l), 0.0) * shadow;
    float hemi = n.y * 0.5 + 0.5;
    vec3 ambient = mix(u_ground_color, u_sky_color, hemi);
    float spec = pow(max(dot(n, h), 0.0), 32.0) * 0.18 * step(0.001, ndl) * shadow;

    vec3 lit = u_color.rgb * (ambient * 0.72 + u_sun_color * ndl * 0.92) + u_sun_color * spec;
    float dist = length(u_cam_pos - v_world_pos);
    float fog = clamp((dist - u_fog_start) / max(u_fog_end - u_fog_start, 0.001), 0.0, 1.0);
    o_color = vec4(mix(lit, u_fog_color, fog), u_color.a);
}
"#;

const LINE_VERT: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec4 a_color;
uniform mat4 u_view_proj;
out vec4 v_color;
void main() {
    v_color = a_color;
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
}
"#;

const LINE_FRAG: &str = r#"#version 330 core
in vec4 v_color;
out vec4 o_color;
void main() { o_color = v_color; }
"#;

const UI_VERT: &str = r#"#version 330 core
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;

uniform vec2 u_screen_size;

out vec2 v_uv;
out vec4 v_color;

void main() {
    vec2 ndc = (a_pos / u_screen_size) * 2.0 - 1.0;
    ndc.y = -ndc.y;
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = vec4(ndc, 0.0, 1.0);
}
"#;

const UI_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_tex;
out vec4 o_color;
void main() {
    o_color = texture(u_tex, v_uv) * v_color;
}
"#;

const BILLBOARD_VERT: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;
uniform mat4 u_view_proj;
out vec2 v_uv;
out vec4 v_color;
void main() {
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = u_view_proj * vec4(a_pos, 1.0);
}
"#;

const BILLBOARD_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
uniform sampler2D u_tex;
out vec4 o_color;
void main() {
    vec4 c = texture(u_tex, v_uv) * v_color;
    if (c.a < 0.01) discard;
    o_color = c;
}
"#;

const POST_VERT: &str = r#"#version 330 core
out vec2 v_uv;
void main() {
    vec2 pos[3] = vec2[](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vec2 p = pos[gl_VertexID];
    v_uv = p * 0.5 + 0.5;
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

const SKY_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
uniform vec3 u_sky_top;
uniform vec3 u_sky_horizon;
out vec4 o_color;
void main() {
    float t = clamp(pow(clamp(v_uv.y, 0.0, 1.0), 0.72), 0.0, 1.0);
    vec3 col = mix(u_sky_horizon, u_sky_top, t);
    o_color = vec4(col, 1.0);
}
"#;

pub const DEFAULT_POST_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
uniform sampler2D u_scene_tex;
uniform float u_exposure;
uniform float u_contrast;
uniform float u_saturation;
uniform float u_vignette;
uniform float u_bloom_strength;
uniform float u_bloom_threshold;
uniform float u_chroma;
uniform float u_time;
uniform vec4 u_tint;

out vec4 o_color;

vec3 extract_bright(vec3 c, float thr) {
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    float mx = max(max(c.r, c.g), c.b);
    float energy = max(l, mx * 0.92);
    return c * smoothstep(thr, thr + 0.22, energy);
}

void main() {
    vec2 d = v_uv - 0.5;
    vec3 col;
    if (u_chroma > 0.0001) {
        vec2 off = d * (u_chroma * 0.018);
        col.r = texture(u_scene_tex, v_uv + off).r;
        col.g = texture(u_scene_tex, v_uv).g;
        col.b = texture(u_scene_tex, v_uv - off).b;
    } else {
        col = texture(u_scene_tex, v_uv).rgb;
    }

    // Two-ring soft bright-pass bloom halo
    if (u_bloom_strength > 0.001) {
        vec2 texel = 1.0 / vec2(textureSize(u_scene_tex, 0));
        vec3 bloom = vec3(0.0);
        for (int y = -1; y <= 1; ++y) {
            for (int x = -1; x <= 1; ++x) {
                vec2 o1 = vec2(x, y) * texel * 2.5;
                vec2 o2 = vec2(x, y) * texel * 6.5;
                bloom += extract_bright(texture(u_scene_tex, v_uv + o1).rgb, u_bloom_threshold) * 0.65;
                bloom += extract_bright(texture(u_scene_tex, v_uv + o2).rgb, u_bloom_threshold) * 0.35;
            }
        }
        col += (bloom / 9.0) * u_bloom_strength;
    }

    col *= u_exposure;
    col = (col - 0.5) * u_contrast + 0.5;
    float luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
    col = mix(vec3(luma), col, u_saturation);
    col *= u_tint.rgb;

    float vig = 1.0 - dot(d, d) * (u_vignette * 2.2);
    col *= clamp(vig, 0.2, 1.0);

    o_color = vec4(clamp(col, 0.0, 1.0), 1.0);
}
"#;

// ---------------------------------------------------------------------------
// Gfx Renderer
// ---------------------------------------------------------------------------

struct PbrUniforms {
    view_proj: GLint,
    model: GLint,
    light_vp: GLint,
    base_tex: GLint,
    mr_tex: GLint,
    occ_tex: GLint,
    emissive_tex: GLint,
    shadow_map: GLint,
    base_factor: GLint,
    tint: GLint,
    metallic_factor: GLint,
    roughness_factor: GLint,
    emissive_factor: GLint,
    has_emissive_tex: GLint,
    alpha_cutoff: GLint,
    shadows_enabled: GLint,
    cam_pos: GLint,
    sun_dir: GLint,
    sun_color: GLint,
    sky_color: GLint,
    ground_color: GLint,
    fog_color: GLint,
    fog_start: GLint,
    fog_end: GLint,
    point_count: GLint,
    point_pos_rad: [GLint; MAX_POINT_LIGHTS],
    point_col_int: [GLint; MAX_POINT_LIGHTS],
}

impl PbrUniforms {
    fn query(gl: &GlFns, prog: GLuint) -> Self {
        let mut point_pos_rad = [-1; MAX_POINT_LIGHTS];
        let mut point_col_int = [-1; MAX_POINT_LIGHTS];
        for i in 0..MAX_POINT_LIGHTS {
            point_pos_rad[i] = uniform_loc(gl, prog, &format!("u_point_pos_rad[{i}]"));
            point_col_int[i] = uniform_loc(gl, prog, &format!("u_point_col_int[{i}]"));
        }
        Self {
            view_proj: uniform_loc(gl, prog, "u_view_proj"),
            model: uniform_loc(gl, prog, "u_model"),
            light_vp: uniform_loc(gl, prog, "u_light_vp"),
            base_tex: uniform_loc(gl, prog, "u_base_tex"),
            mr_tex: uniform_loc(gl, prog, "u_mr_tex"),
            occ_tex: uniform_loc(gl, prog, "u_occ_tex"),
            emissive_tex: uniform_loc(gl, prog, "u_emissive_tex"),
            shadow_map: uniform_loc(gl, prog, "u_shadow_map"),
            base_factor: uniform_loc(gl, prog, "u_base_factor"),
            tint: uniform_loc(gl, prog, "u_tint"),
            metallic_factor: uniform_loc(gl, prog, "u_metallic_factor"),
            roughness_factor: uniform_loc(gl, prog, "u_roughness_factor"),
            emissive_factor: uniform_loc(gl, prog, "u_emissive_factor"),
            has_emissive_tex: uniform_loc(gl, prog, "u_has_emissive_tex"),
            alpha_cutoff: uniform_loc(gl, prog, "u_alpha_cutoff"),
            shadows_enabled: uniform_loc(gl, prog, "u_shadows_enabled"),
            cam_pos: uniform_loc(gl, prog, "u_cam_pos"),
            sun_dir: uniform_loc(gl, prog, "u_sun_dir"),
            sun_color: uniform_loc(gl, prog, "u_sun_color"),
            sky_color: uniform_loc(gl, prog, "u_sky_color"),
            ground_color: uniform_loc(gl, prog, "u_ground_color"),
            fog_color: uniform_loc(gl, prog, "u_fog_color"),
            fog_start: uniform_loc(gl, prog, "u_fog_start"),
            fog_end: uniform_loc(gl, prog, "u_fog_end"),
            point_count: uniform_loc(gl, prog, "u_point_count"),
            point_pos_rad,
            point_col_int,
        }
    }
}

pub struct Gfx {
    shadow_prog: GLuint,
    u_shadow_lvp: GLint,
    u_shadow_model: GLint,
    shadow_fbo: GLuint,
    shadow_tex: GLuint,

    pbr_prog: GLuint,
    pbr_u: PbrUniforms,
    pbr_inst_prog: GLuint,
    pbr_inst_u: PbrUniforms,
    shadow_inst_prog: GLuint,
    u_shadow_inst_lvp: GLint,
    u_shadow_inst_model: GLint,

    solid_prog: GLuint,
    u_solid_vp: GLint,
    u_solid_model: GLint,
    u_solid_lvp: GLint,
    u_solid_shadow_map: GLint,
    u_solid_shadows_enabled: GLint,
    u_solid_color: GLint,
    u_solid_sun_dir: GLint,
    u_solid_sun_color: GLint,
    u_solid_sky_color: GLint,
    u_solid_ground_color: GLint,
    u_solid_cam_pos: GLint,
    u_solid_fog_color: GLint,
    u_solid_fog_start: GLint,
    u_solid_fog_end: GLint,

    line_prog: GLuint,
    u_line_vp: GLint,

    ui_prog: GLuint,
    u_ui_screen: GLint,
    u_ui_tex: GLint,
    ui_vao: GLuint,
    ui_vbo: GLuint,
    ui_cap: usize,
    font_tex: GpuTexture,

    bb_prog: GLuint,
    u_bb_vp: GLint,
    u_bb_tex: GLint,
    bb_vao: GLuint,
    bb_vbo: GLuint,
    bb_cap: usize,

    sky_prog: GLuint,
    u_sky_top: GLint,
    u_sky_horizon: GLint,

    post_prog: GLuint,
    u_post_scene: GLint,
    u_post_exposure: GLint,
    u_post_contrast: GLint,
    u_post_saturation: GLint,
    u_post_vignette: GLint,
    u_post_bloom_str: GLint,
    u_post_bloom_thr: GLint,
    u_post_chroma: GLint,
    u_post_time: GLint,
    u_post_tint: GLint,
    post_vao: GLuint,
    scene_fbo: GLuint,
    scene_tex: GLuint,
    scene_depth_rb: GLuint,
    scene_w: u32,
    scene_h: u32,

    cube_vao: GLuint,
    cube_vbo: GLuint,
    line_vao: GLuint,
    line_vbo: GLuint,
    line_cap: usize,
    pub last_stats: FrameStats,
}

impl Gfx {
    pub fn new(gl: &GlFns) -> Result<Self, String> {
        let shadow_prog = compile_program(gl, SHADOW_VERT, SHADOW_FRAG)?;
        let pbr_prog = compile_program(gl, DEFAULT_PBR_VERT, DEFAULT_PBR_FRAG)?;
        let pbr_inst_prog = compile_program(gl, &with_define(DEFAULT_PBR_VERT, "INSTANCED"), DEFAULT_PBR_FRAG)?;
        let shadow_inst_prog = compile_program(gl, &with_define(SHADOW_VERT, "INSTANCED"), SHADOW_FRAG)?;
        let solid_prog = compile_program(gl, SOLID_VERT, SOLID_FRAG)?;
        let line_prog = compile_program(gl, LINE_VERT, LINE_FRAG)?;
        let ui_prog = compile_program(gl, UI_VERT, UI_FRAG)?;
        let bb_prog = compile_program(gl, BILLBOARD_VERT, BILLBOARD_FRAG)?;
        let sky_prog = compile_program(gl, POST_VERT, SKY_FRAG)?;
        let post_prog = compile_program(gl, POST_VERT, DEFAULT_POST_FRAG)?;

        // 2048x2048 depth-only shadow map FBO
        let mut shadow_fbo = 0;
        let mut shadow_tex = 0;
        unsafe {
            (gl.glGenFramebuffers)(1, &mut shadow_fbo);
            (gl.glGenTextures)(1, &mut shadow_tex);
            (gl.glBindTexture)(GL_TEXTURE_2D, shadow_tex);
            (gl.glTexImage2D)(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT24 as GLint,
                SHADOW_MAP_SIZE,
                SHADOW_MAP_SIZE,
                0,
                GL_DEPTH_COMPONENT,
                GL_FLOAT,
                ptr::null(),
            );
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, shadow_fbo);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, shadow_tex, 0);
            (gl.glDrawBuffer)(GL_NONE);
            (gl.glReadBuffer)(GL_NONE);
            let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                return Err(format!("shadow FBO incomplete: 0x{status:x}"));
            }
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
        }

        // Unit cube VAO
        let cube_verts = build_unit_cube_vertices();
        let mut cube_vao = 0;
        let mut cube_vbo = 0;
        unsafe {
            (gl.glGenVertexArrays)(1, &mut cube_vao);
            (gl.glBindVertexArray)(cube_vao);
            (gl.glGenBuffers)(1, &mut cube_vbo);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, cube_vbo);
            (gl.glBufferData)(
                GL_ARRAY_BUFFER,
                (cube_verts.len() * 4) as GLsizeiptr,
                cube_verts.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            (gl.glEnableVertexAttribArray)(0);
            (gl.glVertexAttribPointer)(0, 3, GL_FLOAT, GL_FALSE, 24, ptr::null());
            (gl.glEnableVertexAttribArray)(1);
            (gl.glVertexAttribPointer)(1, 3, GL_FLOAT, GL_FALSE, 24, 12 as *const c_void);
        }

        // Dynamic line VAO
        let mut line_vao = 0;
        let mut line_vbo = 0;
        unsafe {
            (gl.glGenVertexArrays)(1, &mut line_vao);
            (gl.glBindVertexArray)(line_vao);
            (gl.glGenBuffers)(1, &mut line_vbo);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, line_vbo);
            let stride = std::mem::size_of::<LineVertex>() as GLsizei;
            (gl.glEnableVertexAttribArray)(0);
            (gl.glVertexAttribPointer)(0, 3, GL_FLOAT, GL_FALSE, stride, ptr::null());
            (gl.glEnableVertexAttribArray)(1);
            (gl.glVertexAttribPointer)(1, 4, GL_FLOAT, GL_FALSE, stride, 12 as *const c_void);
        }

        // Dynamic 2D UI VAO + built-in bitmap font atlas
        let mut ui_vao = 0;
        let mut ui_vbo = 0;
        unsafe {
            (gl.glGenVertexArrays)(1, &mut ui_vao);
            (gl.glBindVertexArray)(ui_vao);
            (gl.glGenBuffers)(1, &mut ui_vbo);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, ui_vbo);
            let stride = std::mem::size_of::<UiVertex>() as GLsizei;
            (gl.glEnableVertexAttribArray)(0);
            (gl.glVertexAttribPointer)(0, 2, GL_FLOAT, GL_FALSE, stride, ptr::null());
            (gl.glEnableVertexAttribArray)(1);
            (gl.glVertexAttribPointer)(1, 2, GL_FLOAT, GL_FALSE, stride, 8 as *const c_void);
            (gl.glEnableVertexAttribArray)(2);
            (gl.glVertexAttribPointer)(2, 4, GL_FLOAT, GL_FALSE, stride, 16 as *const c_void);
            (gl.glBindVertexArray)(0);
        }
        let font_rgba = font::build_font_atlas_rgba();
        let font_tex = upload_texture_rgba(gl, font::ATLAS_W, font::ATLAS_H, &font_rgba, false);

        // Dynamic 3D billboard VAO + empty VAO for fullscreen post-process triangle
        let mut bb_vao = 0;
        let mut bb_vbo = 0;
        let mut post_vao = 0;
        unsafe {
            (gl.glGenVertexArrays)(1, &mut bb_vao);
            (gl.glBindVertexArray)(bb_vao);
            (gl.glGenBuffers)(1, &mut bb_vbo);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, bb_vbo);
            let stride = std::mem::size_of::<BillboardVertex>() as GLsizei;
            (gl.glEnableVertexAttribArray)(0);
            (gl.glVertexAttribPointer)(0, 3, GL_FLOAT, GL_FALSE, stride, ptr::null());
            (gl.glEnableVertexAttribArray)(1);
            (gl.glVertexAttribPointer)(1, 2, GL_FLOAT, GL_FALSE, stride, 12 as *const c_void);
            (gl.glEnableVertexAttribArray)(2);
            (gl.glVertexAttribPointer)(2, 4, GL_FLOAT, GL_FALSE, stride, 20 as *const c_void);
            (gl.glGenVertexArrays)(1, &mut post_vao);
            (gl.glBindVertexArray)(0);
        }

        Ok(Self {
            u_shadow_lvp: uniform_loc(gl, shadow_prog, "u_light_vp"),
            u_shadow_model: uniform_loc(gl, shadow_prog, "u_model"),
            shadow_prog,
            shadow_fbo,
            shadow_tex,
            pbr_u: PbrUniforms::query(gl, pbr_prog),
            pbr_prog,
            pbr_inst_u: PbrUniforms::query(gl, pbr_inst_prog),
            pbr_inst_prog,
            u_shadow_inst_lvp: uniform_loc(gl, shadow_inst_prog, "u_light_vp"),
            u_shadow_inst_model: uniform_loc(gl, shadow_inst_prog, "u_model"),
            shadow_inst_prog,
            u_solid_vp: uniform_loc(gl, solid_prog, "u_view_proj"),
            u_solid_model: uniform_loc(gl, solid_prog, "u_model"),
            u_solid_lvp: uniform_loc(gl, solid_prog, "u_light_vp"),
            u_solid_shadow_map: uniform_loc(gl, solid_prog, "u_shadow_map"),
            u_solid_shadows_enabled: uniform_loc(gl, solid_prog, "u_shadows_enabled"),
            u_solid_color: uniform_loc(gl, solid_prog, "u_color"),
            u_solid_sun_dir: uniform_loc(gl, solid_prog, "u_sun_dir"),
            u_solid_sun_color: uniform_loc(gl, solid_prog, "u_sun_color"),
            u_solid_sky_color: uniform_loc(gl, solid_prog, "u_sky_color"),
            u_solid_ground_color: uniform_loc(gl, solid_prog, "u_ground_color"),
            u_solid_cam_pos: uniform_loc(gl, solid_prog, "u_cam_pos"),
            u_solid_fog_color: uniform_loc(gl, solid_prog, "u_fog_color"),
            u_solid_fog_start: uniform_loc(gl, solid_prog, "u_fog_start"),
            u_solid_fog_end: uniform_loc(gl, solid_prog, "u_fog_end"),
            solid_prog,
            u_line_vp: uniform_loc(gl, line_prog, "u_view_proj"),
            line_prog,
            u_ui_screen: uniform_loc(gl, ui_prog, "u_screen_size"),
            u_ui_tex: uniform_loc(gl, ui_prog, "u_tex"),
            ui_prog,
            ui_vao,
            ui_vbo,
            ui_cap: 0,
            font_tex,
            u_bb_vp: uniform_loc(gl, bb_prog, "u_view_proj"),
            u_bb_tex: uniform_loc(gl, bb_prog, "u_tex"),
            bb_prog,
            bb_vao,
            bb_vbo,
            bb_cap: 0,
            u_sky_top: uniform_loc(gl, sky_prog, "u_sky_top"),
            u_sky_horizon: uniform_loc(gl, sky_prog, "u_sky_horizon"),
            sky_prog,
            u_post_scene: uniform_loc(gl, post_prog, "u_scene_tex"),
            u_post_exposure: uniform_loc(gl, post_prog, "u_exposure"),
            u_post_contrast: uniform_loc(gl, post_prog, "u_contrast"),
            u_post_saturation: uniform_loc(gl, post_prog, "u_saturation"),
            u_post_vignette: uniform_loc(gl, post_prog, "u_vignette"),
            u_post_bloom_str: uniform_loc(gl, post_prog, "u_bloom_strength"),
            u_post_bloom_thr: uniform_loc(gl, post_prog, "u_bloom_threshold"),
            u_post_chroma: uniform_loc(gl, post_prog, "u_chroma"),
            u_post_time: uniform_loc(gl, post_prog, "u_time"),
            u_post_tint: uniform_loc(gl, post_prog, "u_tint"),
            post_prog,
            post_vao,
            scene_fbo: 0,
            scene_tex: 0,
            scene_depth_rb: 0,
            scene_w: 0,
            scene_h: 0,
            cube_vao,
            cube_vbo,
            line_vao,
            line_vbo,
            line_cap: 0,
            last_stats: FrameStats::default(),
        })
    }

    /// Hot-reloads the PBR shader program from optional disk overrides (falling back to built-in
    /// defaults for whichever stage is `None`). Keeps the previous program active on compile error.
    pub fn reload_pbr_shader(
        &mut self,
        gl: &GlFns,
        vert_override: Option<&str>,
        frag_override: Option<&str>,
    ) -> Result<(), String> {
        let vs = vert_override.unwrap_or(DEFAULT_PBR_VERT);
        let fs = frag_override.unwrap_or(DEFAULT_PBR_FRAG);
        let new_prog = compile_program(gl, vs, fs)?;
        // Custom vertex shaders need the `#ifdef INSTANCED` block of DEFAULT_PBR_VERT for
        // mesh_instances to place instances; without it they all draw at the batch transform.
        let new_inst = match compile_program(gl, &with_define(vs, "INSTANCED"), fs) {
            Ok(p) => p,
            Err(e) => {
                unsafe { (gl.glDeleteProgram)(new_prog) };
                return Err(e);
            }
        };
        let old = std::mem::replace(&mut self.pbr_prog, new_prog);
        self.pbr_u = PbrUniforms::query(gl, new_prog);
        let old_inst = std::mem::replace(&mut self.pbr_inst_prog, new_inst);
        self.pbr_inst_u = PbrUniforms::query(gl, new_inst);
        unsafe {
            (gl.glDeleteProgram)(old);
            (gl.glDeleteProgram)(old_inst);
        }
        Ok(())
    }

    pub fn reload_post_shader(&mut self, gl: &GlFns, frag_override: Option<&str>) -> Result<(), String> {
        let fs = frag_override.unwrap_or(DEFAULT_POST_FRAG);
        let new_prog = compile_program(gl, POST_VERT, fs)?;
        let old = std::mem::replace(&mut self.post_prog, new_prog);
        self.u_post_scene = uniform_loc(gl, new_prog, "u_scene_tex");
        self.u_post_exposure = uniform_loc(gl, new_prog, "u_exposure");
        self.u_post_contrast = uniform_loc(gl, new_prog, "u_contrast");
        self.u_post_saturation = uniform_loc(gl, new_prog, "u_saturation");
        self.u_post_vignette = uniform_loc(gl, new_prog, "u_vignette");
        self.u_post_bloom_str = uniform_loc(gl, new_prog, "u_bloom_strength");
        self.u_post_bloom_thr = uniform_loc(gl, new_prog, "u_bloom_threshold");
        self.u_post_chroma = uniform_loc(gl, new_prog, "u_chroma");
        self.u_post_time = uniform_loc(gl, new_prog, "u_time");
        self.u_post_tint = uniform_loc(gl, new_prog, "u_tint");
        unsafe { (gl.glDeleteProgram)(old) };
        Ok(())
    }

    fn ensure_scene_fbo(&mut self, gl: &GlFns, width: u32, height: u32) -> bool {
        let w = width.max(1);
        let h = height.max(1);
        if self.scene_fbo != 0 && self.scene_w == w && self.scene_h == h {
            return true;
        }
        unsafe {
            if self.scene_fbo != 0 {
                (gl.glDeleteFramebuffers)(1, &self.scene_fbo);
                (gl.glDeleteTextures)(1, &self.scene_tex);
                (gl.glDeleteRenderbuffers)(1, &self.scene_depth_rb);
                self.scene_fbo = 0;
            }
            let mut fbo = 0;
            let mut tex = 0;
            let mut rb = 0;
            (gl.glGenFramebuffers)(1, &mut fbo);
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);

            (gl.glGenTextures)(1, &mut tex);
            (gl.glBindTexture)(GL_TEXTURE_2D, tex);
            (gl.glTexImage2D)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8 as GLint,
                w as GLsizei,
                h as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                ptr::null(),
            );
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);

            (gl.glGenRenderbuffers)(1, &mut rb);
            (gl.glBindRenderbuffer)(GL_RENDERBUFFER, rb);
            (gl.glRenderbufferStorage)(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, w as GLsizei, h as GLsizei);
            (gl.glFramebufferRenderbuffer)(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);

            let ok = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE;
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, 0);
            if ok {
                self.scene_fbo = fbo;
                self.scene_tex = tex;
                self.scene_depth_rb = rb;
                self.scene_w = w;
                self.scene_h = h;
                true
            } else {
                (gl.glDeleteFramebuffers)(1, &fbo);
                (gl.glDeleteTextures)(1, &tex);
                (gl.glDeleteRenderbuffers)(1, &rb);
                false
            }
        }
    }

    pub fn render_frame(
        &mut self,
        gl: &GlFns,
        default_fbo: GLuint,
        width: u32,
        height: u32,
        draw: &mut DrawList,
        assets: &Assets,
    ) {
        let mut stats = FrameStats::default();
        let aspect = width as f32 / height.max(1) as f32;
        let vp = draw.camera.view_proj(aspect);

        if draw.overlay == DebugOverlay::Grid {
            draw.debug_grid(20, 1.0, Color::rgba(0.32, 0.36, 0.42, 0.8), Color::rgba(0.7, 0.72, 0.76, 0.9));
            draw.debug_axes(Vec3::new(0.0, 0.02, 0.0), 2.0);
        }

        // Directional light view-projection snapped around camera target
        let ext = draw.shadow_extent.max(4.0);
        let sun_dir = draw.sun_dir.norm();
        let center = draw.camera.target;
        let light_pos = center + sun_dir * (ext * 2.0);
        let light_up = if sun_dir.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
        let light_vp = Mat4::ortho(-ext, ext, -ext, ext, 0.5, ext * 5.0)
            * Mat4::look_at(light_pos, center, light_up);

        // Frustum culling: (visible in main pass, casts into shadow map)
        let cull = draw.frustum_culling;
        let view_frustum = Frustum::from_view_proj(vp);
        let light_frustum = Frustum::from_view_proj(light_vp);
        let visibility = |b: Aabb| -> (bool, bool) {
            if !cull {
                return (true, true);
            }
            (view_frustum.intersects_aabb(b), draw.shadows_enabled && light_frustum.intersects_aabb(b))
        };

        // Flatten PBR draw items
        struct FlatPrim<'a> {
            prim: &'a GpuPrimitive,
            mat: &'a GpuMaterial,
            model: Mat4,
            tint: Color,
            cam_dist_sq: f32,
            in_view: bool,
            in_shadow: bool,
            instances: Option<&'a GpuInstances>,
        }
        let mut opaque_prims: Vec<FlatPrim<'_>> = Vec::new();
        let mut blend_prims: Vec<FlatPrim<'_>> = Vec::new();
        let mut inst_prims: Vec<FlatPrim<'_>> = Vec::new();

        for cmd in &draw.meshes {
            let Some(gpu_mesh) = assets.mesh(cmd.handle) else { continue };
            let (in_view, in_shadow) = if cmd.node_overrides.is_empty() {
                visibility(gpu_mesh.bounds.transformed(cmd.model))
            } else {
                (true, true)
            };
            if !in_view {
                stats.culled += 1;
                if !in_shadow {
                    continue;
                }
            }
            let node_mats = gpu_mesh.compute_node_matrices(&cmd.node_overrides);

            if draw.overlay == DebugOverlay::Colliders && in_view {
                for col in &gpu_mesh.colliders {
                    push_wire_aabb_transformed(&mut draw.collider_lines, col.bounds, cmd.model, Color::YELLOW);
                }
            }

            for (ni, node) in gpu_mesh.nodes.iter().enumerate() {
                if node.is_collider {
                    continue;
                }
                let Some(mi) = node.mesh else { continue };
                let Some(prims) = gpu_mesh.meshes.get(mi) else { continue };
                let world_m = cmd.model * node_mats[ni];
                let world_pos = Vec3::new(world_m.m[12], world_m.m[13], world_m.m[14]);
                let dist_sq = (world_pos - draw.camera.pos).len_sq();

                for prim in prims {
                    let mat = &gpu_mesh.materials[prim.material_idx];
                    let item = FlatPrim { prim, mat, model: world_m, tint: cmd.tint, cam_dist_sq: dist_sq, in_view, in_shadow, instances: None };
                    if mat.alpha_mode == AlphaMode::Blend || cmd.tint.a < 0.999 {
                        if in_view {
                            blend_prims.push(item);
                        }
                    } else {
                        opaque_prims.push(item);
                    }
                }
            }
        }
        blend_prims.sort_by(|a, b| b.cam_dist_sq.total_cmp(&a.cam_dist_sq));

        for cmd in &draw.instanced {
            let Some(gpu_mesh) = assets.mesh(cmd.handle) else { continue };
            let Some(inst) = assets.instances(cmd.instances) else { continue };
            if inst.count == 0 {
                continue;
            }
            let (in_view, in_shadow) = visibility(inst.world_bounds(gpu_mesh.bounds, cmd.model));
            if !in_view {
                stats.culled += 1;
                if !in_shadow {
                    continue;
                }
            }
            let node_mats = gpu_mesh.compute_node_matrices(&[]);
            for (ni, node) in gpu_mesh.nodes.iter().enumerate() {
                if node.is_collider {
                    continue;
                }
                let Some(mi) = node.mesh else { continue };
                let Some(prims) = gpu_mesh.meshes.get(mi) else { continue };
                for prim in prims {
                    let mat = &gpu_mesh.materials[prim.material_idx];
                    inst_prims.push(FlatPrim {
                        prim, mat, model: cmd.model * node_mats[ni], tint: cmd.tint,
                        cam_dist_sq: 0.0, in_view, in_shadow, instances: Some(inst),
                    });
                }
            }
        }

        unsafe {
            // 0. Directional Shadow Map Pass
            if draw.shadows_enabled
                && (opaque_prims.iter().chain(&inst_prims).any(|p| p.in_shadow) || draw.boxes.iter().any(|b| b.cast_shadow))
            {
                (gl.glBindFramebuffer)(GL_FRAMEBUFFER, self.shadow_fbo);
                (gl.glViewport)(0, 0, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE);
                (gl.glEnable)(GL_DEPTH_TEST);
                (gl.glDepthFunc)(GL_LESS);
                (gl.glDepthMask)(GL_TRUE);
                (gl.glColorMask)(GL_FALSE, GL_FALSE, GL_FALSE, GL_FALSE);
                (gl.glClear)(GL_DEPTH_BUFFER_BIT);
                (gl.glEnable)(GL_CULL_FACE);
                (gl.glCullFace)(GL_FRONT);

                (gl.glUseProgram)(self.shadow_prog);
                (gl.glUniformMatrix4fv)(self.u_shadow_lvp, 1, GL_FALSE, light_vp.m.as_ptr());

                for item in opaque_prims.iter().filter(|p| p.in_shadow) {
                    if item.mat.double_sided {
                        (gl.glDisable)(GL_CULL_FACE);
                    } else {
                        (gl.glEnable)(GL_CULL_FACE);
                    }
                    (gl.glUniformMatrix4fv)(self.u_shadow_model, 1, GL_FALSE, item.model.m.as_ptr());
                    (gl.glBindVertexArray)(item.prim.vao);
                    (gl.glDrawElements)(GL_TRIANGLES, item.prim.index_count, GL_UNSIGNED_INT, ptr::null());
                }

                if inst_prims.iter().any(|p| p.in_shadow) {
                    (gl.glUseProgram)(self.shadow_inst_prog);
                    (gl.glUniformMatrix4fv)(self.u_shadow_inst_lvp, 1, GL_FALSE, light_vp.m.as_ptr());
                    for item in inst_prims.iter().filter(|p| p.in_shadow) {
                        if item.mat.double_sided {
                            (gl.glDisable)(GL_CULL_FACE);
                        } else {
                            (gl.glEnable)(GL_CULL_FACE);
                        }
                        (gl.glUniformMatrix4fv)(self.u_shadow_inst_model, 1, GL_FALSE, item.model.m.as_ptr());
                        (gl.glBindVertexArray)(item.prim.vao);
                        draw_elements_instanced(gl, item.prim, item.instances.unwrap());
                    }
                    (gl.glUseProgram)(self.shadow_prog);
                }

                (gl.glEnable)(GL_CULL_FACE);
                (gl.glBindVertexArray)(self.cube_vao);
                for b in &draw.boxes {
                    if !b.cast_shadow {
                        continue;
                    }
                    (gl.glUniformMatrix4fv)(self.u_shadow_model, 1, GL_FALSE, b.model.m.as_ptr());
                    (gl.glDrawArrays)(GL_TRIANGLES, 0, 36);
                }

                (gl.glCullFace)(GL_BACK);
                (gl.glColorMask)(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
            }

            // Main Scene Framebuffer Setup (render into scene_fbo when post-processing is active)
            let use_post = draw.post.enabled && self.ensure_scene_fbo(gl, width, height);
            let scene_target = if use_post { self.scene_fbo } else { default_fbo };
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, scene_target);
            (gl.glViewport)(0, 0, width as GLsizei, height as GLsizei);
            (gl.glEnable)(GL_DEPTH_TEST);
            (gl.glDepthFunc)(GL_LESS);
            (gl.glDepthMask)(GL_TRUE);
            (gl.glEnable)(GL_CULL_FACE);
            (gl.glCullFace)(GL_BACK);
            (gl.glFrontFace)(GL_CCW);
            (gl.glEnable)(GL_BLEND);
            (gl.glBlendFunc)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

            let c = draw.clear_color;
            (gl.glClearColor)(c.r, c.g, c.b, c.a);
            (gl.glClear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

            // Optional vertical sky gradient backdrop (drawn behind all 3D geometry)
            if let Some((top, hor)) = draw.sky_gradient {
                (gl.glDisable)(GL_DEPTH_TEST);
                (gl.glDepthMask)(GL_FALSE);
                (gl.glDisable)(GL_CULL_FACE);
                (gl.glUseProgram)(self.sky_prog);
                (gl.glUniform3f)(self.u_sky_top, top.r, top.g, top.b);
                (gl.glUniform3f)(self.u_sky_horizon, hor.r, hor.g, hor.b);
                (gl.glBindVertexArray)(self.post_vao);
                (gl.glDrawArrays)(GL_TRIANGLES, 0, 3);
                (gl.glEnable)(GL_DEPTH_TEST);
                (gl.glDepthMask)(GL_TRUE);
                (gl.glEnable)(GL_CULL_FACE);
            }

            // Bind shadow map on texture unit 4 for both solid and PBR passes
            (gl.glActiveTexture)(GL_TEXTURE4);
            (gl.glBindTexture)(GL_TEXTURE_2D, self.shadow_tex);

            let poly_mode = if draw.overlay == DebugOverlay::Wireframe { GL_LINE } else { GL_FILL };
            let sh_flag = if draw.shadows_enabled { 1.0 } else { 0.0 };

            // 1. Solid debug boxes & ground
            if !draw.boxes.is_empty() {
                (gl.glPolygonMode)(GL_FRONT_AND_BACK, poly_mode);
                (gl.glUseProgram)(self.solid_prog);
                (gl.glUniformMatrix4fv)(self.u_solid_vp, 1, GL_FALSE, vp.m.as_ptr());
                (gl.glUniformMatrix4fv)(self.u_solid_lvp, 1, GL_FALSE, light_vp.m.as_ptr());
                (gl.glUniform1i)(self.u_solid_shadow_map, 4);
                (gl.glUniform1f)(self.u_solid_shadows_enabled, sh_flag);
                (gl.glUniform3f)(self.u_solid_sun_dir, sun_dir.x, sun_dir.y, sun_dir.z);
                (gl.glUniform3f)(self.u_solid_sun_color, draw.sun_color.r, draw.sun_color.g, draw.sun_color.b);
                (gl.glUniform3f)(self.u_solid_sky_color, draw.sky_color.r, draw.sky_color.g, draw.sky_color.b);
                (gl.glUniform3f)(self.u_solid_ground_color, draw.ground_color.r, draw.ground_color.g, draw.ground_color.b);
                (gl.glUniform3f)(self.u_solid_cam_pos, draw.camera.pos.x, draw.camera.pos.y, draw.camera.pos.z);
                (gl.glUniform3f)(self.u_solid_fog_color, draw.fog_color.r, draw.fog_color.g, draw.fog_color.b);
                (gl.glUniform1f)(self.u_solid_fog_start, draw.fog_start);
                (gl.glUniform1f)(self.u_solid_fog_end, draw.fog_end);

                (gl.glBindVertexArray)(self.cube_vao);
                for cmd in &draw.boxes {
                    (gl.glUniformMatrix4fv)(self.u_solid_model, 1, GL_FALSE, cmd.model.m.as_ptr());
                    (gl.glUniform4f)(self.u_solid_color, cmd.color.r, cmd.color.g, cmd.color.b, cmd.color.a);
                    (gl.glDrawArrays)(GL_TRIANGLES, 0, 36);
                    stats.draw_calls += 1;
                    stats.triangles += 12;
                }
            }

            // 2. PBR Meshes (opaque then sorted transparent)
            let any_inst = inst_prims.iter().any(|p| p.in_view);
            if opaque_prims.iter().any(|p| p.in_view) || !blend_prims.is_empty() || any_inst {
                (gl.glPolygonMode)(GL_FRONT_AND_BACK, poly_mode);
                self.bind_pbr_frame_uniforms(gl, false, vp, light_vp, draw);

                for item in opaque_prims.iter().filter(|p| p.in_view) {
                    self.draw_pbr_prim(gl, assets, item.prim, item.mat, item.model, item.tint, None);
                    stats.draw_calls += 1;
                    stats.triangles += (item.prim.index_count / 3) as u32;
                }

                if any_inst {
                    self.bind_pbr_frame_uniforms(gl, true, vp, light_vp, draw);
                    for item in inst_prims.iter().filter(|p| p.in_view) {
                        let inst = item.instances.unwrap();
                        self.draw_pbr_prim(gl, assets, item.prim, item.mat, item.model, item.tint, Some(inst));
                        stats.draw_calls += 1;
                        stats.instances += inst.count;
                        stats.triangles = stats.triangles.saturating_add((item.prim.index_count / 3) as u32 * inst.count);
                    }
                    self.bind_pbr_frame_uniforms(gl, false, vp, light_vp, draw);
                }

                if !blend_prims.is_empty() {
                    (gl.glDepthMask)(GL_FALSE);
                    for item in &blend_prims {
                        self.draw_pbr_prim(gl, assets, item.prim, item.mat, item.model, item.tint, None);
                        stats.draw_calls += 1;
                        stats.triangles += (item.prim.index_count / 3) as u32;
                    }
                    (gl.glDepthMask)(GL_TRUE);
                }
                (gl.glEnable)(GL_CULL_FACE);
            }

            (gl.glPolygonMode)(GL_FRONT_AND_BACK, GL_FILL);

            // 2.5 Camera-facing 3D Billboards / Particles
            if !draw.billboards.is_empty() {
                self.render_billboards(gl, vp, draw, assets, &mut stats);
            }

            // 3. Debug & collider lines
            let mut all_lines = Vec::new();
            all_lines.extend_from_slice(&draw.lines);
            if draw.overlay == DebugOverlay::Colliders {
                all_lines.extend_from_slice(&draw.collider_lines);
            }
            if !all_lines.is_empty() {
                (gl.glUseProgram)(self.line_prog);
                (gl.glUniformMatrix4fv)(self.u_line_vp, 1, GL_FALSE, vp.m.as_ptr());
                (gl.glBindVertexArray)(self.line_vao);
                (gl.glBindBuffer)(GL_ARRAY_BUFFER, self.line_vbo);

                let byte_len = (all_lines.len() * std::mem::size_of::<LineVertex>()) as GLsizeiptr;
                if all_lines.len() > self.line_cap {
                    self.line_cap = all_lines.len().next_power_of_two().max(256);
                    let cap_bytes = (self.line_cap * std::mem::size_of::<LineVertex>()) as GLsizeiptr;
                    (gl.glBufferData)(GL_ARRAY_BUFFER, cap_bytes, ptr::null(), GL_DYNAMIC_DRAW);
                }
                (gl.glBufferSubData)(GL_ARRAY_BUFFER, 0, byte_len, all_lines.as_ptr() as *const c_void);
                (gl.glDrawArrays)(GL_LINES, 0, all_lines.len() as GLsizei);
                stats.draw_calls += 1;
                stats.lines += (all_lines.len() / 2) as u32;
            }

            // 3.5 Fullscreen Post-Processing Pass (scene_fbo -> default_fbo, before crisp UI!)
            if use_post {
                (gl.glBindFramebuffer)(GL_FRAMEBUFFER, default_fbo);
                (gl.glViewport)(0, 0, width as GLsizei, height as GLsizei);
                (gl.glDisable)(GL_DEPTH_TEST);
                (gl.glDisable)(GL_CULL_FACE);
                (gl.glUseProgram)(self.post_prog);
                (gl.glActiveTexture)(GL_TEXTURE0);
                (gl.glBindTexture)(GL_TEXTURE_2D, self.scene_tex);
                (gl.glUniform1i)(self.u_post_scene, 0);
                let p = &draw.post;
                (gl.glUniform1f)(self.u_post_exposure, p.exposure);
                (gl.glUniform1f)(self.u_post_contrast, p.contrast);
                (gl.glUniform1f)(self.u_post_saturation, p.saturation);
                (gl.glUniform1f)(self.u_post_vignette, p.vignette);
                (gl.glUniform1f)(self.u_post_bloom_str, p.bloom_strength);
                (gl.glUniform1f)(self.u_post_bloom_thr, p.bloom_threshold);
                (gl.glUniform1f)(self.u_post_chroma, p.chroma);
                (gl.glUniform1f)(self.u_post_time, p.time);
                (gl.glUniform4f)(self.u_post_tint, p.tint.r, p.tint.g, p.tint.b, p.tint.a);
                (gl.glBindVertexArray)(self.post_vao);
                (gl.glDrawArrays)(GL_TRIANGLES, 0, 3);
                stats.draw_calls += 1;
            }

            // 4. Batched 2D / UI Overlay Pass (always razor-sharp on default_fbo)
            if !draw.ui_quads.is_empty() {
                self.render_ui_pass(gl, width, height, &draw.ui_quads, assets, &mut stats);
            }

            (gl.glBindVertexArray)(0);
        }

        self.last_stats = stats;
    }

    unsafe fn render_billboards(
        &mut self,
        gl: &GlFns,
        vp: Mat4,
        draw: &DrawList,
        assets: &Assets,
        stats: &mut FrameStats,
    ) {
        let mut sorted: Vec<(f32, BillboardCmd)> = draw
            .billboards
            .iter()
            .map(|&b| ((b.pos - draw.camera.pos).len_sq(), b))
            .collect();
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));

        let fwd = (draw.camera.target - draw.camera.pos).norm();
        let right = fwd.cross(draw.camera.up).norm();
        let up = right.cross(fwd).norm();

        let mut verts: Vec<BillboardVertex> = Vec::with_capacity(sorted.len() * 6);
        let mut batches: Vec<(GLuint, i32, i32)> = Vec::new();

        for (_, b) in sorted {
            let tex_id = b
                .tex
                .map(|h| assets.texture(h).id)
                .unwrap_or_else(|| assets.texture(assets.white_tex).id);
            let (r_dir, u_dir) = b.axes.unwrap_or((right, up));
            let rx = r_dir * (b.size.x * 0.5);
            let uy = u_dir * (b.size.y * 0.5);
            let p0 = (b.pos - rx + uy).to_array();
            let p1 = (b.pos - rx - uy).to_array();
            let p2 = (b.pos + rx - uy).to_array();
            let p3 = (b.pos + rx + uy).to_array();
            let c = b.color.to_array();

            let first = verts.len() as i32;
            verts.push(BillboardVertex { pos: p0, uv: [0.0, 0.0], color: c });
            verts.push(BillboardVertex { pos: p1, uv: [0.0, 1.0], color: c });
            verts.push(BillboardVertex { pos: p2, uv: [1.0, 1.0], color: c });
            verts.push(BillboardVertex { pos: p0, uv: [0.0, 0.0], color: c });
            verts.push(BillboardVertex { pos: p2, uv: [1.0, 1.0], color: c });
            verts.push(BillboardVertex { pos: p3, uv: [1.0, 0.0], color: c });

            if let Some(last) = batches.last_mut()
                && last.0 == tex_id
            {
                last.2 += 6;
            } else {
                batches.push((tex_id, first, 6));
            }
        }

        unsafe {
            (gl.glDepthMask)(GL_FALSE);
            (gl.glDisable)(GL_CULL_FACE);
            (gl.glUseProgram)(self.bb_prog);
            (gl.glUniformMatrix4fv)(self.u_bb_vp, 1, GL_FALSE, vp.m.as_ptr());
            (gl.glUniform1i)(self.u_bb_tex, 0);
            (gl.glActiveTexture)(GL_TEXTURE0);
            (gl.glBindVertexArray)(self.bb_vao);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, self.bb_vbo);

            let byte_len = (verts.len() * std::mem::size_of::<BillboardVertex>()) as GLsizeiptr;
            if verts.len() > self.bb_cap {
                self.bb_cap = verts.len().next_power_of_two().max(256);
                let cap_bytes = (self.bb_cap * std::mem::size_of::<BillboardVertex>()) as GLsizeiptr;
                (gl.glBufferData)(GL_ARRAY_BUFFER, cap_bytes, ptr::null(), GL_DYNAMIC_DRAW);
            }
            (gl.glBufferSubData)(GL_ARRAY_BUFFER, 0, byte_len, verts.as_ptr() as *const c_void);

            for (tex_id, first, count) in batches {
                (gl.glBindTexture)(GL_TEXTURE_2D, tex_id);
                (gl.glDrawArrays)(GL_TRIANGLES, first, count);
                stats.draw_calls += 1;
                stats.triangles += (count / 3) as u32;
            }
            (gl.glDepthMask)(GL_TRUE);
            (gl.glEnable)(GL_CULL_FACE);
        }
    }

    unsafe fn render_ui_pass(
        &mut self,
        gl: &GlFns,
        width: u32,
        height: u32,
        quads: &[UiQuadCmd],
        assets: &Assets,
        stats: &mut FrameStats,
    ) {
        let mut verts: Vec<UiVertex> = Vec::with_capacity(quads.len() * 6);
        let mut batches: Vec<(GLuint, i32, i32)> = Vec::new();

        for q in quads {
            let tex_id = match q.tex {
                UiTex::White => assets.texture(assets.white_tex).id,
                UiTex::Font => self.font_tex.id,
                UiTex::Asset(h) => assets.texture(h).id,
            };
            let first = verts.len() as i32;
            let c = q.color.to_array();
            let (p0, p1, p2, p3) = if q.angle.abs() < 1e-5 {
                let (x0, y0, x1, y1) = (q.x, q.y, q.x + q.w, q.y + q.h);
                ([x0, y0], [x0, y1], [x1, y1], [x1, y0])
            } else {
                let cx = q.x + q.w * 0.5;
                let cy = q.y + q.h * 0.5;
                let hw = q.w * 0.5;
                let hh = q.h * 0.5;
                let (s, co) = q.angle.sin_cos();
                let rot = |dx: f32, dy: f32| [cx + dx * co - dy * s, cy + dx * s + dy * co];
                (rot(-hw, -hh), rot(-hw, hh), rot(hw, hh), rot(hw, -hh))
            };
            verts.push(UiVertex { pos: p0, uv: [q.u0, q.v0], color: c });
            verts.push(UiVertex { pos: p1, uv: [q.u0, q.v1], color: c });
            verts.push(UiVertex { pos: p2, uv: [q.u1, q.v1], color: c });
            verts.push(UiVertex { pos: p0, uv: [q.u0, q.v0], color: c });
            verts.push(UiVertex { pos: p2, uv: [q.u1, q.v1], color: c });
            verts.push(UiVertex { pos: p3, uv: [q.u1, q.v0], color: c });

            if let Some(last) = batches.last_mut()
                && last.0 == tex_id
            {
                last.2 += 6;
            } else {
                batches.push((tex_id, first, 6));
            }
        }

        unsafe {
            (gl.glDisable)(GL_DEPTH_TEST);
            (gl.glDisable)(GL_CULL_FACE);
            (gl.glUseProgram)(self.ui_prog);
            (gl.glUniform2f)(self.u_ui_screen, width.max(1) as f32, height.max(1) as f32);
            (gl.glUniform1i)(self.u_ui_tex, 0);
            (gl.glActiveTexture)(GL_TEXTURE0);

            (gl.glBindVertexArray)(self.ui_vao);
            (gl.glBindBuffer)(GL_ARRAY_BUFFER, self.ui_vbo);
            let byte_len = (verts.len() * std::mem::size_of::<UiVertex>()) as GLsizeiptr;
            if verts.len() > self.ui_cap {
                self.ui_cap = verts.len().next_power_of_two().max(512);
                let cap_bytes = (self.ui_cap * std::mem::size_of::<UiVertex>()) as GLsizeiptr;
                (gl.glBufferData)(GL_ARRAY_BUFFER, cap_bytes, ptr::null(), GL_DYNAMIC_DRAW);
            }
            (gl.glBufferSubData)(GL_ARRAY_BUFFER, 0, byte_len, verts.as_ptr() as *const c_void);

            for (tex_id, first, count) in batches {
                (gl.glBindTexture)(GL_TEXTURE_2D, tex_id);
                (gl.glDrawArrays)(GL_TRIANGLES, first, count);
                stats.draw_calls += 1;
            }
            stats.ui_quads = quads.len() as u32;
            (gl.glEnable)(GL_DEPTH_TEST);
            (gl.glEnable)(GL_CULL_FACE);
        }
    }

    unsafe fn bind_pbr_frame_uniforms(&self, gl: &GlFns, instanced: bool, vp: Mat4, light_vp: Mat4, draw: &DrawList) {
        let (prog, u) = if instanced { (self.pbr_inst_prog, &self.pbr_inst_u) } else { (self.pbr_prog, &self.pbr_u) };
        let sun_dir = draw.sun_dir.norm();
        unsafe {
            (gl.glUseProgram)(prog);
            (gl.glUniformMatrix4fv)(u.view_proj, 1, GL_FALSE, vp.m.as_ptr());
            (gl.glUniformMatrix4fv)(u.light_vp, 1, GL_FALSE, light_vp.m.as_ptr());
            (gl.glUniform1i)(u.base_tex, 0);
            (gl.glUniform1i)(u.mr_tex, 1);
            (gl.glUniform1i)(u.occ_tex, 2);
            (gl.glUniform1i)(u.emissive_tex, 3);
            (gl.glUniform1i)(u.shadow_map, 4);
            (gl.glUniform1f)(u.shadows_enabled, if draw.shadows_enabled { 1.0 } else { 0.0 });
            (gl.glUniform3f)(u.cam_pos, draw.camera.pos.x, draw.camera.pos.y, draw.camera.pos.z);
            (gl.glUniform3f)(u.sun_dir, sun_dir.x, sun_dir.y, sun_dir.z);
            (gl.glUniform3f)(u.sun_color, draw.sun_color.r, draw.sun_color.g, draw.sun_color.b);
            (gl.glUniform3f)(u.sky_color, draw.sky_color.r, draw.sky_color.g, draw.sky_color.b);
            (gl.glUniform3f)(u.ground_color, draw.ground_color.r, draw.ground_color.g, draw.ground_color.b);
            (gl.glUniform3f)(u.fog_color, draw.fog_color.r, draw.fog_color.g, draw.fog_color.b);
            (gl.glUniform1f)(u.fog_start, draw.fog_start);
            (gl.glUniform1f)(u.fog_end, draw.fog_end);

            let count = draw.point_lights.len().min(MAX_POINT_LIGHTS);
            (gl.glUniform1i)(u.point_count, count as GLint);
            for (i, pl) in draw.point_lights.iter().take(count).enumerate() {
                (gl.glUniform4f)(u.point_pos_rad[i], pl.pos.x, pl.pos.y, pl.pos.z, pl.radius);
                (gl.glUniform4f)(u.point_col_int[i], pl.color.r, pl.color.g, pl.color.b, pl.intensity);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_pbr_prim(
        &self,
        gl: &GlFns,
        assets: &Assets,
        prim: &GpuPrimitive,
        mat: &GpuMaterial,
        model: Mat4,
        tint: Color,
        instances: Option<&GpuInstances>,
    ) {
        let u = if instances.is_some() { &self.pbr_inst_u } else { &self.pbr_u };
        unsafe {
            if mat.double_sided {
                (gl.glDisable)(GL_CULL_FACE);
            } else {
                (gl.glEnable)(GL_CULL_FACE);
            }

            (gl.glActiveTexture)(GL_TEXTURE0);
            (gl.glBindTexture)(GL_TEXTURE_2D, assets.texture(mat.base_color_tex).id);
            (gl.glActiveTexture)(GL_TEXTURE1);
            (gl.glBindTexture)(GL_TEXTURE_2D, assets.texture(mat.metallic_roughness_tex).id);
            (gl.glActiveTexture)(GL_TEXTURE2);
            (gl.glBindTexture)(GL_TEXTURE_2D, assets.texture(mat.occlusion_tex).id);
            (gl.glActiveTexture)(GL_TEXTURE3);
            (gl.glBindTexture)(GL_TEXTURE_2D, assets.texture(mat.emissive_tex).id);

            (gl.glUniformMatrix4fv)(u.model, 1, GL_FALSE, model.m.as_ptr());
            let bc = mat.base_color_factor;
            (gl.glUniform4f)(u.base_factor, bc[0], bc[1], bc[2], bc[3]);
            (gl.glUniform4f)(u.tint, tint.r, tint.g, tint.b, tint.a);
            (gl.glUniform1f)(u.metallic_factor, mat.metallic_factor);
            (gl.glUniform1f)(u.roughness_factor, mat.roughness_factor);
            let ef = mat.emissive_factor;
            (gl.glUniform3f)(u.emissive_factor, ef[0], ef[1], ef[2]);
            (gl.glUniform1f)(u.has_emissive_tex, if mat.has_emissive_tex { 1.0 } else { 0.0 });
            let cutoff = if mat.alpha_mode == AlphaMode::Mask { mat.alpha_cutoff } else { 0.0 };
            (gl.glUniform1f)(u.alpha_cutoff, cutoff);

            (gl.glBindVertexArray)(prim.vao);
            match instances {
                Some(inst) => draw_elements_instanced(gl, prim, inst),
                None => (gl.glDrawElements)(GL_TRIANGLES, prim.index_count, GL_UNSIGNED_INT, ptr::null()),
            }
        }
    }

    pub fn save_screenshot<P: AsRef<Path>>(
        &self,
        gl: &GlFns,
        default_fbo: GLuint,
        width: u32,
        height: u32,
        path: P,
    ) -> Result<(), String> {
        let row_bytes = (width as usize) * 4;
        let mut flipped = vec![0u8; row_bytes * (height as usize)];
        unsafe {
            (gl.glBindFramebuffer)(GL_FRAMEBUFFER, default_fbo);
            (gl.glPixelStorei)(GL_PACK_ALIGNMENT, 1);
            (gl.glReadPixels)(
                0,
                0,
                width as GLsizei,
                height as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                flipped.as_mut_ptr() as *mut c_void,
            );
        }

        let mut rgba = vec![0u8; flipped.len()];
        for y in 0..height as usize {
            let src_row = (height as usize - 1 - y) * row_bytes;
            let dst_row = y * row_bytes;
            rgba[dst_row..dst_row + row_bytes].copy_from_slice(&flipped[src_row..src_row + row_bytes]);
        }

        png::write_rgba_png(path, width, height, &rgba).map_err(|e| format!("PNG write failed: {e}"))
    }

    pub fn destroy(&self, gl: &GlFns) {
        unsafe {
            (gl.glDeleteFramebuffers)(1, &self.shadow_fbo);
            (gl.glDeleteTextures)(1, &self.shadow_tex);
            if self.scene_fbo != 0 {
                (gl.glDeleteFramebuffers)(1, &self.scene_fbo);
                (gl.glDeleteTextures)(1, &self.scene_tex);
                (gl.glDeleteRenderbuffers)(1, &self.scene_depth_rb);
            }
            (gl.glDeleteTextures)(1, &self.font_tex.id);
            (gl.glDeleteVertexArrays)(1, &self.cube_vao);
            (gl.glDeleteBuffers)(1, &self.cube_vbo);
            (gl.glDeleteVertexArrays)(1, &self.line_vao);
            (gl.glDeleteBuffers)(1, &self.line_vbo);
            (gl.glDeleteVertexArrays)(1, &self.ui_vao);
            (gl.glDeleteBuffers)(1, &self.ui_vbo);
            (gl.glDeleteVertexArrays)(1, &self.bb_vao);
            (gl.glDeleteBuffers)(1, &self.bb_vbo);
            (gl.glDeleteVertexArrays)(1, &self.post_vao);
            (gl.glDeleteProgram)(self.shadow_prog);
            (gl.glDeleteProgram)(self.pbr_prog);
            (gl.glDeleteProgram)(self.pbr_inst_prog);
            (gl.glDeleteProgram)(self.shadow_inst_prog);
            (gl.glDeleteProgram)(self.solid_prog);
            (gl.glDeleteProgram)(self.line_prog);
            (gl.glDeleteProgram)(self.ui_prog);
            (gl.glDeleteProgram)(self.bb_prog);
            (gl.glDeleteProgram)(self.sky_prog);
            (gl.glDeleteProgram)(self.post_prog);
        }
    }
}

fn compile_shader(gl: &GlFns, kind: GLenum, src: &str) -> Result<GLuint, String> {
    let c_src = CString::new(src).map_err(|_| "shader contains interior NUL")?;
    unsafe {
        let shader = (gl.glCreateShader)(kind);
        let ptr = c_src.as_ptr();
        (gl.glShaderSource)(shader, 1, &ptr, ptr::null());
        (gl.glCompileShader)(shader);
        let mut ok: GLint = 0;
        (gl.glGetShaderiv)(shader, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut log_len: GLint = 0;
            (gl.glGetShaderiv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);
            let mut buf = vec![0u8; log_len.max(1) as usize];
            (gl.glGetShaderInfoLog)(shader, buf.len() as GLsizei, ptr::null_mut(), buf.as_mut_ptr() as *mut c_char);
            (gl.glDeleteShader)(shader);
            let msg = String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string();
            return Err(format!("shader compile error: {msg}"));
        }
        Ok(shader)
    }
}

fn compile_program(gl: &GlFns, vert_src: &str, frag_src: &str) -> Result<GLuint, String> {
    let vs = compile_shader(gl, GL_VERTEX_SHADER, vert_src)?;
    let fs = match compile_shader(gl, GL_FRAGMENT_SHADER, frag_src) {
        Ok(s) => s,
        Err(e) => {
            unsafe { (gl.glDeleteShader)(vs) };
            return Err(e);
        }
    };
    unsafe {
        let prog = (gl.glCreateProgram)();
        (gl.glAttachShader)(prog, vs);
        (gl.glAttachShader)(prog, fs);
        (gl.glLinkProgram)(prog);
        (gl.glDeleteShader)(vs);
        (gl.glDeleteShader)(fs);

        let mut ok: GLint = 0;
        (gl.glGetProgramiv)(prog, GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut log_len: GLint = 0;
            (gl.glGetProgramiv)(prog, GL_INFO_LOG_LENGTH, &mut log_len);
            let mut buf = vec![0u8; log_len.max(1) as usize];
            (gl.glGetProgramInfoLog)(prog, buf.len() as GLsizei, ptr::null_mut(), buf.as_mut_ptr() as *mut c_char);
            (gl.glDeleteProgram)(prog);
            let msg = String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string();
            return Err(format!("program link error: {msg}"));
        }
        Ok(prog)
    }
}

fn uniform_loc(gl: &GlFns, prog: GLuint, name: &str) -> GLint {
    let c = CString::new(name).unwrap();
    unsafe { (gl.glGetUniformLocation)(prog, c.as_ptr()) }
}

fn build_unit_cube_vertices() -> Vec<f32> {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
    ];

    let mut out = Vec::with_capacity(36 * 6);
    for (n, q) in faces {
        for idx in [0usize, 1, 2, 0, 2, 3] {
            out.extend_from_slice(&q[idx]);
            out.extend_from_slice(&n);
        }
    }
    out
}
