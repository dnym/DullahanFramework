//! Pure-Rust glTF 2.0 loader supporting `.glb` (binary) and `.gltf` (+ external `.bin`/PNGs).
//!
//! Extracts:
//! - Indexed triangle primitives (pos, normal, uv, vertex color)
//! - PBR metallic-roughness + occlusion (ORM) + emissive materials & textures
//! - Named node hierarchy (for procedural sub-part animation via local TRS overrides)
//! - Collider nodes (`col`, `col_*`, `*_col`, `*-colonly`) excluded from visual draws

use crate::dullahan::formats::json::Json;
use crate::dullahan::formats::png::{self, DecodedImage};
use crate::dullahan::math::{Aabb, Mat4, Quat, Transform, Vec2, Vec3};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Debug)]
pub struct GltfMaterial {
    pub name: String,
    pub base_color_factor: [f32; 4],
    pub base_color_tex: Option<usize>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub metallic_roughness_tex: Option<usize>,
    pub occlusion_tex: Option<usize>,
    pub emissive_factor: [f32; 3],
    pub emissive_tex: Option<usize>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for GltfMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            base_color_tex: None,
            metallic_factor: 0.0,
            roughness_factor: 0.85,
            metallic_roughness_tex: None,
            occlusion_tex: None,
            emissive_factor: [0.0, 0.0, 0.0],
            emissive_tex: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GltfPrimitive {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct GltfMesh {
    pub name: String,
    pub primitives: Vec<GltfPrimitive>,
}

#[derive(Clone, Debug)]
pub struct GltfNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub transform: Transform,
    pub raw_matrix: Option<Mat4>,
    pub is_collider: bool,
}

#[derive(Clone, Debug)]
pub struct GltfCollider {
    pub name: String,
    pub node_index: usize,
    pub bounds: Aabb,
}

#[derive(Clone, Debug)]
pub enum TextureSource {
    Embedded(DecodedImage),
    External(PathBuf),
}

#[derive(Clone, Debug)]
pub struct GltfImage {
    pub source: TextureSource,
    pub linear_filter: bool,
}

#[derive(Clone, Debug)]
pub struct GltfModel {
    pub images: Vec<GltfImage>,
    pub materials: Vec<GltfMaterial>,
    pub meshes: Vec<GltfMesh>,
    pub nodes: Vec<GltfNode>,
    pub roots: Vec<usize>,
    pub colliders: Vec<GltfCollider>,
    pub bounds: Aabb,
    /// External files referenced by this model (`.bin` and external `.png`s) for hot-reload tracking.
    pub dependencies: Vec<PathBuf>,
}

impl GltfModel {
    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Computes model-space world matrices for every node, applying optional local TRS overrides.
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

pub fn load_gltf_file<P: AsRef<Path>>(path: P) -> Result<GltfModel, String> {
    let path = path.as_ref();
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;

    let (json_text, glb_bin) = if bytes.len() >= 12 && &bytes[0..4] == b"glTF" {
        parse_glb_chunks(&bytes)?
    } else {
        let s = String::from_utf8(bytes).map_err(|_| "glTF file is not valid UTF-8")?;
        (s, None)
    };

    let root = Json::parse(&json_text).map_err(|e| format!("glTF JSON error: {e}"))?;
    parse_gltf_json(&root, base_dir, glb_bin)
}

fn parse_glb_chunks(bytes: &[u8]) -> Result<(String, Option<Vec<u8>>), String> {
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 2 {
        return Err(format!("unsupported GLB version {version}"));
    }
    let total_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if total_len > bytes.len() {
        return Err("truncated GLB file".into());
    }

    let mut pos = 12usize;
    let mut json_chunk: Option<String> = None;
    let mut bin_chunk: Option<Vec<u8>> = None;

    while pos + 8 <= total_len {
        let clen = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let ctype = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
        pos += 8;
        if pos + clen > total_len {
            return Err("truncated GLB chunk".into());
        }
        let slice = &bytes[pos..pos + clen];
        pos += clen;
        match ctype {
            0x4E4F_534A => {
                // "JSON"
                let s = std::str::from_utf8(slice).map_err(|_| "invalid UTF-8 in GLB JSON")?;
                json_chunk = Some(s.trim_end_matches('\0').to_string());
            }
            0x004E_4942 => {
                // "BIN\0"
                bin_chunk = Some(slice.to_vec());
            }
            _ => {}
        }
    }

    let json = json_chunk.ok_or("GLB missing JSON chunk")?;
    Ok((json, bin_chunk))
}

struct BufferView {
    buffer: usize,
    byte_offset: usize,
    byte_length: usize,
    byte_stride: Option<usize>,
}

struct Accessor {
    buffer_view: usize,
    byte_offset: usize,
    component_type: u32,
    count: usize,
    elem_type: String,
    normalized: bool,
}

fn parse_gltf_json(root: &Json, base_dir: &Path, glb_bin: Option<Vec<u8>>) -> Result<GltfModel, String> {
    let mut dependencies = Vec::new();

    // 1. Buffers
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    if let Some(arr) = root.get("buffers").and_then(Json::as_arr) {
        for (i, b) in arr.iter().enumerate() {
            if let Some(uri) = b.get("uri").and_then(Json::as_str) {
                if let Some(b64) = strip_data_uri(uri) {
                    buffers.push(decode_base64(b64)?);
                } else {
                    let full = base_dir.join(uri);
                    let data = fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))?;
                    dependencies.push(full);
                    buffers.push(data);
                }
            } else if i == 0 && let Some(ref bin) = glb_bin {
                buffers.push(bin.clone());
            } else {
                return Err(format!("buffer {i} missing uri and GLB BIN chunk"));
            }
        }
    } else if let Some(bin) = glb_bin {
        buffers.push(bin);
    }

    // 2. BufferViews
    let mut views = Vec::new();
    if let Some(arr) = root.get("bufferViews").and_then(Json::as_arr) {
        for v in arr {
            views.push(BufferView {
                buffer: v.get("buffer").and_then(Json::as_usize).unwrap_or(0),
                byte_offset: v.get("byteOffset").and_then(Json::as_usize).unwrap_or(0),
                byte_length: v.get("byteLength").and_then(Json::as_usize).unwrap_or(0),
                byte_stride: v.get("byteStride").and_then(Json::as_usize),
            });
        }
    }

    // 3. Accessors
    let mut accessors = Vec::new();
    if let Some(arr) = root.get("accessors").and_then(Json::as_arr) {
        for a in arr {
            accessors.push(Accessor {
                buffer_view: a.get("bufferView").and_then(Json::as_usize).unwrap_or(0),
                byte_offset: a.get("byteOffset").and_then(Json::as_usize).unwrap_or(0),
                component_type: a.get("componentType").and_then(Json::as_usize).unwrap_or(5126) as u32,
                count: a.get("count").and_then(Json::as_usize).unwrap_or(0),
                elem_type: a.get("type").and_then(Json::as_str).unwrap_or("SCALAR").to_string(),
                normalized: a.get("normalized").and_then(Json::as_bool).unwrap_or(false),
            });
        }
    }

    // 4. Samplers -> Images -> Textures
    let mut sampler_linear = Vec::new();
    if let Some(arr) = root.get("samplers").and_then(Json::as_arr) {
        for s in arr {
            let mag = s.get("magFilter").and_then(Json::as_usize).unwrap_or(9729);
            sampler_linear.push(mag != 9728); // 9728 = GL_NEAREST
        }
    }

    let mut raw_images: Vec<TextureSource> = Vec::new();
    if let Some(arr) = root.get("images").and_then(Json::as_arr) {
        for img in arr {
            if let Some(bv_idx) = img.get("bufferView").and_then(Json::as_usize) {
                let bv = views.get(bv_idx).ok_or("image bufferView out of range")?;
                let buf = buffers.get(bv.buffer).ok_or("image buffer out of range")?;
                let end = bv.byte_offset + bv.byte_length;
                if end > buf.len() {
                    return Err("image bufferView exceeds buffer".into());
                }
                let decoded = png::decode_png_rgba(&buf[bv.byte_offset..end])?;
                raw_images.push(TextureSource::Embedded(decoded));
            } else if let Some(uri) = img.get("uri").and_then(Json::as_str) {
                if let Some(b64) = strip_data_uri(uri) {
                    let bytes = decode_base64(b64)?;
                    let decoded = png::decode_png_rgba(&bytes)?;
                    raw_images.push(TextureSource::Embedded(decoded));
                } else {
                    let full = base_dir.join(uri);
                    dependencies.push(full.clone());
                    raw_images.push(TextureSource::External(full));
                }
            }
        }
    }

    let mut images = Vec::new();
    if let Some(arr) = root.get("textures").and_then(Json::as_arr) {
        for t in arr {
            let src_idx = t.get("source").and_then(Json::as_usize).unwrap_or(0);
            let samp_idx = t.get("sampler").and_then(Json::as_usize);
            let linear_filter = samp_idx.and_then(|i| sampler_linear.get(i).copied()).unwrap_or(true);
            if let Some(src) = raw_images.get(src_idx) {
                images.push(GltfImage { source: src.clone(), linear_filter });
            }
        }
    }

    // 5. Materials
    let mut materials = Vec::new();
    if let Some(arr) = root.get("materials").and_then(Json::as_arr) {
        for m in arr {
            let mut mat = GltfMaterial::default();
            if let Some(name) = m.get("name").and_then(Json::as_str) {
                mat.name = name.to_string();
            }
            if let Some(pbr) = m.get("pbrMetallicRoughness") {
                mat.metallic_factor = pbr.get("metallicFactor").and_then(Json::as_f32).unwrap_or(1.0);
                mat.roughness_factor = pbr.get("roughnessFactor").and_then(Json::as_f32).unwrap_or(1.0);
                if let Some(f) = pbr.get("baseColorFactor").and_then(Json::as_arr)
                    && f.len() == 4
                {
                    mat.base_color_factor = [
                        f[0].as_f32().unwrap_or(1.0),
                        f[1].as_f32().unwrap_or(1.0),
                        f[2].as_f32().unwrap_or(1.0),
                        f[3].as_f32().unwrap_or(1.0),
                    ];
                }
                mat.base_color_tex = pbr
                    .get("baseColorTexture")
                    .and_then(|t| t.get("index"))
                    .and_then(Json::as_usize);
                mat.metallic_roughness_tex = pbr
                    .get("metallicRoughnessTexture")
                    .and_then(|t| t.get("index"))
                    .and_then(Json::as_usize);
                // If no metallicRoughnessTexture and factors weren't specified, keep matte default reasonable.
                if mat.metallic_roughness_tex.is_none() && pbr.get("metallicFactor").is_none() {
                    mat.metallic_factor = 0.0;
                }
            }
            mat.occlusion_tex = m
                .get("occlusionTexture")
                .and_then(|t| t.get("index"))
                .and_then(Json::as_usize);
            mat.emissive_tex = m
                .get("emissiveTexture")
                .and_then(|t| t.get("index"))
                .and_then(Json::as_usize);
            if let Some(ef) = m.get("emissiveFactor").and_then(Json::as_arr)
                && ef.len() == 3
            {
                mat.emissive_factor = [
                    ef[0].as_f32().unwrap_or(0.0),
                    ef[1].as_f32().unwrap_or(0.0),
                    ef[2].as_f32().unwrap_or(0.0),
                ];
            }
            if let Some(am) = m.get("alphaMode").and_then(Json::as_str) {
                mat.alpha_mode = match am {
                    "MASK" => AlphaMode::Mask,
                    "BLEND" => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                };
            }
            mat.alpha_cutoff = m.get("alphaCutoff").and_then(Json::as_f32).unwrap_or(0.5);
            mat.double_sided = m.get("doubleSided").and_then(Json::as_bool).unwrap_or(false);
            materials.push(mat);
        }
    }

    // 6. Meshes & Primitives
    let mut meshes = Vec::new();
    if let Some(arr) = root.get("meshes").and_then(Json::as_arr) {
        for m in arr {
            let name = m.get("name").and_then(Json::as_str).unwrap_or("").to_string();
            let mut primitives = Vec::new();
            if let Some(prims) = m.get("primitives").and_then(Json::as_arr) {
                for p in prims {
                    let mode = p.get("mode").and_then(Json::as_usize).unwrap_or(4);
                    if mode != 4 {
                        continue; // Only triangle lists (mode 4)
                    }
                    let attrs = p.get("attributes").ok_or("primitive missing attributes")?;
                    let pos_idx = attrs.get("POSITION").and_then(Json::as_usize).ok_or("missing POSITION")?;
                    let positions = read_vec3(&buffers, &views, &accessors, pos_idx)?;

                    let indices = if let Some(idx_acc) = p.get("indices").and_then(Json::as_usize) {
                        read_indices(&buffers, &views, &accessors, idx_acc)?
                    } else {
                        (0..positions.len() as u32).collect()
                    };

                    let normals = if let Some(n_idx) = attrs.get("NORMAL").and_then(Json::as_usize) {
                        read_vec3(&buffers, &views, &accessors, n_idx)?
                    } else {
                        compute_flat_normals(&positions, &indices)
                    };

                    let uvs = if let Some(uv_idx) = attrs.get("TEXCOORD_0").and_then(Json::as_usize) {
                        read_vec2(&buffers, &views, &accessors, uv_idx)?
                    } else {
                        vec![Vec2::ZERO; positions.len()]
                    };

                    let colors = if let Some(c_idx) = attrs.get("COLOR_0").and_then(Json::as_usize) {
                        read_colors(&buffers, &views, &accessors, c_idx)?
                    } else {
                        vec![[1.0, 1.0, 1.0, 1.0]; positions.len()]
                    };

                    let material = p.get("material").and_then(Json::as_usize);
                    primitives.push(GltfPrimitive { positions, normals, uvs, colors, indices, material });
                }
            }
            meshes.push(GltfMesh { name, primitives });
        }
    }

    // 7. Nodes & Scene roots
    let mut nodes = Vec::new();
    if let Some(arr) = root.get("nodes").and_then(Json::as_arr) {
        for n in arr {
            let name = n.get("name").and_then(Json::as_str).unwrap_or("").to_string();
            let mesh = n.get("mesh").and_then(Json::as_usize);
            let children: Vec<usize> = n
                .get("children")
                .and_then(Json::as_arr)
                .map(|a| a.iter().filter_map(Json::as_usize).collect())
                .unwrap_or_default();

            let mut transform = Transform::IDENTITY;
            if let Some(t) = n.get("translation").and_then(Json::as_arr)
                && t.len() == 3
            {
                transform.pos = Vec3::new(
                    t[0].as_f32().unwrap_or(0.0),
                    t[1].as_f32().unwrap_or(0.0),
                    t[2].as_f32().unwrap_or(0.0),
                );
            }
            if let Some(r) = n.get("rotation").and_then(Json::as_arr)
                && r.len() == 4
            {
                transform.rot = Quat::new(
                    r[0].as_f32().unwrap_or(0.0),
                    r[1].as_f32().unwrap_or(0.0),
                    r[2].as_f32().unwrap_or(0.0),
                    r[3].as_f32().unwrap_or(1.0),
                )
                .norm();
            }
            if let Some(s) = n.get("scale").and_then(Json::as_arr)
                && s.len() == 3
            {
                transform.scale = Vec3::new(
                    s[0].as_f32().unwrap_or(1.0),
                    s[1].as_f32().unwrap_or(1.0),
                    s[2].as_f32().unwrap_or(1.0),
                );
            }
            let raw_matrix = n.get("matrix").and_then(Json::as_arr).and_then(|arr| {
                if arr.len() == 16 {
                    let mut m = [0.0f32; 16];
                    for i in 0..16 {
                        m[i] = arr[i].as_f32().unwrap_or(0.0);
                    }
                    Some(Mat4 { m })
                } else {
                    None
                }
            });

            let is_collider = is_collider_name(&name);
            nodes.push(GltfNode { name, parent: None, children, mesh, transform, raw_matrix, is_collider });
        }
    }

    // Wire parent pointers & propagate collider tag to child nodes of collider groups
    for parent_idx in 0..nodes.len() {
        let kids = nodes[parent_idx].children.clone();
        let parent_is_col = nodes[parent_idx].is_collider;
        for c in kids {
            if c < nodes.len() {
                nodes[c].parent = Some(parent_idx);
                if parent_is_col {
                    nodes[c].is_collider = true;
                }
            }
        }
    }

    let scene_idx = root.get("scene").and_then(Json::as_usize).unwrap_or(0);
    let mut roots: Vec<usize> = root
        .get("scenes")
        .and_then(Json::as_arr)
        .and_then(|scenes| scenes.get(scene_idx))
        .and_then(|s| s.get("nodes"))
        .and_then(Json::as_arr)
        .map(|a| a.iter().filter_map(Json::as_usize).collect())
        .unwrap_or_default();

    if roots.is_empty() {
        roots = (0..nodes.len()).filter(|&i| nodes[i].parent.is_none()).collect();
    }

    let mut model = GltfModel {
        images,
        materials,
        meshes,
        nodes,
        roots,
        colliders: Vec::new(),
        bounds: Aabb::new(Vec3::ZERO, Vec3::ZERO),
        dependencies,
    };

    // Compute model-space bounds and extract collider AABBs
    let world_mats = model.compute_node_matrices(&[]);
    let mut bmin = Vec3::splat(f32::INFINITY);
    let mut bmax = Vec3::splat(f32::NEG_INFINITY);

    for (ni, node) in model.nodes.iter().enumerate() {
        let Some(mi) = node.mesh else { continue };
        let Some(mesh) = model.meshes.get(mi) else { continue };
        let wm = world_mats[ni];

        let mut cmin = Vec3::splat(f32::INFINITY);
        let mut cmax = Vec3::splat(f32::NEG_INFINITY);
        for prim in &mesh.primitives {
            for &p in &prim.positions {
                let wp = wm.transform_point(p);
                cmin = cmin.min(wp);
                cmax = cmax.max(wp);
                if !node.is_collider {
                    bmin = bmin.min(wp);
                    bmax = bmax.max(wp);
                }
            }
        }

        if node.is_collider && cmin.x.is_finite() {
            model.colliders.push(GltfCollider {
                name: node.name.clone(),
                node_index: ni,
                bounds: Aabb::new(cmin, cmax),
            });
        }
    }

    if bmin.x.is_finite() {
        model.bounds = Aabb::new(bmin, bmax);
    }

    Ok(model)
}

fn is_collider_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "col"
        || lower.starts_with("col_")
        || lower.starts_with("col-")
        || lower.ends_with("_col")
        || lower.ends_with("-col")
        || lower.ends_with("-colonly")
        || lower.ends_with("_colonly")
}

fn accessor_slice<'a>(
    buffers: &'a [Vec<u8>],
    views: &[BufferView],
    accessors: &'a [Accessor],
    acc_idx: usize,
) -> Result<(&'a [u8], &'a Accessor, usize), String> {
    let acc = accessors.get(acc_idx).ok_or("accessor index out of range")?;
    let bv = views.get(acc.buffer_view).ok_or("bufferView index out of range")?;
    let buf = buffers.get(bv.buffer).ok_or("buffer index out of range")?;
    let start = bv.byte_offset + acc.byte_offset;
    let comp_size = match acc.component_type {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        c => return Err(format!("unsupported accessor componentType {c}")),
    };
    let num_comps = match acc.elem_type.as_str() {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        t => return Err(format!("unsupported accessor type {t}")),
    };
    let elem_size = comp_size * num_comps;
    let stride = bv.byte_stride.unwrap_or(elem_size);
    if acc.count > 0 {
        let last_end = start + (acc.count - 1) * stride + elem_size;
        if last_end > buf.len() {
            return Err("accessor slice exceeds buffer bounds".into());
        }
    }
    Ok((&buf[start..], acc, stride))
}

fn read_f32_comp(data: &[u8], comp_type: u32, normalized: bool) -> f32 {
    match comp_type {
        5126 => f32::from_le_bytes(data[0..4].try_into().unwrap()),
        5121 => {
            let v = data[0] as f32;
            if normalized { v / 255.0 } else { v }
        }
        5123 => {
            let v = u16::from_le_bytes([data[0], data[1]]) as f32;
            if normalized { v / 65535.0 } else { v }
        }
        5120 => {
            let v = (data[0] as i8) as f32;
            if normalized { (v / 127.0).max(-1.0) } else { v }
        }
        5122 => {
            let v = i16::from_le_bytes([data[0], data[1]]) as f32;
            if normalized { (v / 32767.0).max(-1.0) } else { v }
        }
        _ => 0.0,
    }
}

fn comp_byte_size(comp_type: u32) -> usize {
    match comp_type {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        _ => 4,
    }
}

fn read_vec3(buffers: &[Vec<u8>], views: &[BufferView], accessors: &[Accessor], idx: usize) -> Result<Vec<Vec3>, String> {
    let (data, acc, stride) = accessor_slice(buffers, views, accessors, idx)?;
    let cs = comp_byte_size(acc.component_type);
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let b = &data[i * stride..];
        out.push(Vec3::new(
            read_f32_comp(&b[0..], acc.component_type, acc.normalized),
            read_f32_comp(&b[cs..], acc.component_type, acc.normalized),
            read_f32_comp(&b[cs * 2..], acc.component_type, acc.normalized),
        ));
    }
    Ok(out)
}

fn read_vec2(buffers: &[Vec<u8>], views: &[BufferView], accessors: &[Accessor], idx: usize) -> Result<Vec<Vec2>, String> {
    let (data, acc, stride) = accessor_slice(buffers, views, accessors, idx)?;
    let cs = comp_byte_size(acc.component_type);
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let b = &data[i * stride..];
        out.push(Vec2::new(
            read_f32_comp(&b[0..], acc.component_type, acc.normalized),
            read_f32_comp(&b[cs..], acc.component_type, acc.normalized),
        ));
    }
    Ok(out)
}

fn read_colors(buffers: &[Vec<u8>], views: &[BufferView], accessors: &[Accessor], idx: usize) -> Result<Vec<[f32; 4]>, String> {
    let (data, acc, stride) = accessor_slice(buffers, views, accessors, idx)?;
    let cs = comp_byte_size(acc.component_type);
    let has_alpha = acc.elem_type == "VEC4";
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let b = &data[i * stride..];
        let r = read_f32_comp(&b[0..], acc.component_type, acc.normalized);
        let g = read_f32_comp(&b[cs..], acc.component_type, acc.normalized);
        let bl = read_f32_comp(&b[cs * 2..], acc.component_type, acc.normalized);
        let a = if has_alpha {
            read_f32_comp(&b[cs * 3..], acc.component_type, acc.normalized)
        } else {
            1.0
        };
        out.push([r, g, bl, a]);
    }
    Ok(out)
}

fn read_indices(buffers: &[Vec<u8>], views: &[BufferView], accessors: &[Accessor], idx: usize) -> Result<Vec<u32>, String> {
    let (data, acc, stride) = accessor_slice(buffers, views, accessors, idx)?;
    let mut out = Vec::with_capacity(acc.count);
    for i in 0..acc.count {
        let b = &data[i * stride..];
        let v = match acc.component_type {
            5121 => b[0] as u32,
            5123 => u16::from_le_bytes([b[0], b[1]]) as u32,
            5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            c => return Err(format!("unsupported index componentType {c}")),
        };
        out.push(v);
    }
    Ok(out)
}

fn compute_flat_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut acc = vec![Vec3::ZERO; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 < positions.len() && i1 < positions.len() && i2 < positions.len() {
            let n = (positions[i1] - positions[i0]).cross(positions[i2] - positions[i0]);
            acc[i0] += n;
            acc[i1] += n;
            acc[i2] += n;
        }
    }
    acc.into_iter().map(|n| if n.len_sq() > 1e-10 { n.norm() } else { Vec3::Y }).collect()
}

fn strip_data_uri(uri: &str) -> Option<&str> {
    if uri.starts_with("data:") {
        uri.split_once(";base64,").map(|(_, b64)| b64)
    } else {
        None
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => return Err("invalid base64 byte in data URI".into()),
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}
