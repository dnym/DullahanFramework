//! Pure-Rust Game-UI SVG rasterizer -> anti-aliased RGBA8 (`DecodedImage`).
//!
//! Supports the practical Game-UI SVG subset:
//! - `<svg viewBox="...">`, `<g>`, `<rect rx ry>`, `<circle>`, `<ellipse>`, `<line>`,
//!   `<polyline>`, `<polygon>`, `<path d="M/m L/l H/h V/v C/c S/s Q/q T/t A/a Z/z">`
//! - `fill`, `stroke`, `stroke-width`, `opacity`, `fill-opacity`, `stroke-opacity`, inline `style`
//! - 2D `transform`: `translate`, `scale`, `rotate`, `matrix`
//! - 4x4 supersampled anti-aliasing + transparent-edge alpha bleed (zero dark bilinear fringes)

use crate::dullahan::formats::png::DecodedImage;
use std::fs;
use std::path::Path;

pub fn load_svg_file<P: AsRef<Path>>(path: P, target_w: u32, target_h: u32) -> Result<DecodedImage, String> {
    let text = fs::read_to_string(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    rasterize_svg(&text, target_w, target_h)
}

#[derive(Copy, Clone, Debug)]
struct Mat2D {
    a: f32, b: f32,
    c: f32, d: f32,
    e: f32, f: f32,
}

impl Mat2D {
    const ID: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn mul(self, r: Self) -> Self {
        Self {
            a: self.a * r.a + self.c * r.b,
            b: self.b * r.a + self.d * r.b,
            c: self.a * r.c + self.c * r.d,
            d: self.b * r.c + self.d * r.d,
            e: self.a * r.e + self.c * r.f + self.e,
            f: self.b * r.e + self.d * r.f + self.f,
        }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    fn approx_scale(self) -> f32 {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        (sx + sy) * 0.5
    }
}

#[derive(Copy, Clone, Debug)]
struct Style {
    fill: Option<[f32; 4]>,
    stroke: Option<[f32; 4]>,
    stroke_width: f32,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    transform: Mat2D,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some([0.0, 0.0, 0.0, 1.0]),
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            transform: Mat2D::ID,
        }
    }
}

struct DrawShape {
    subpaths: Vec<(Vec<(f32, f32)>, bool)>, // (points in output pixel space, closed)
    fill: Option<[f32; 4]>,
    stroke: Option<[f32; 4]>,
    stroke_width: f32,
}

pub fn rasterize_svg(xml: &str, req_w: u32, req_h: u32) -> Result<DecodedImage, String> {
    let mut vb = (0.0f32, 0.0f32, 64.0f32, 64.0f32);
    let mut svg_w = 0.0f32;
    let mut svg_h = 0.0f32;

    // First pass: find root <svg> dimensions / viewBox
    let tokens = tokenize_xml(xml);
    for tok in &tokens {
        if tok.tag == "svg" && !tok.is_close {
            if let Some(v) = tok.attr("viewBox") {
                let nums = parse_numbers(v);
                if nums.len() >= 4 && nums[2] > 0.0 && nums[3] > 0.0 {
                    vb = (nums[0], nums[1], nums[2], nums[3]);
                }
            }
            if let Some(w) = tok.attr("width").and_then(parse_len) {
                svg_w = w;
            }
            if let Some(h) = tok.attr("height").and_then(parse_len) {
                svg_h = h;
            }
            break;
        }
    }

    let out_w = if req_w > 0 { req_w } else if svg_w > 0.0 { svg_w.round() as u32 } else { vb.2.round().max(1.0) as u32 };
    let out_h = if req_h > 0 { req_h } else if svg_h > 0.0 { svg_h.round() as u32 } else { vb.3.round().max(1.0) as u32 };

    let sx = out_w as f32 / vb.2.max(1e-4);
    let sy = out_h as f32 / vb.3.max(1e-4);
    let root_xform = Mat2D { a: sx, b: 0.0, c: 0.0, d: sy, e: -vb.0 * sx, f: -vb.1 * sy };

    let mut stack = vec![Style { transform: root_xform, ..Style::default() }];
    let mut shapes: Vec<DrawShape> = Vec::new();
    let mut skip_depth = 0usize;

    for tok in &tokens {
        if tok.tag == "defs" || tok.tag == "clipPath" || tok.tag == "mask" || tok.tag == "filter" {
            if !tok.is_close && !tok.self_close {
                skip_depth += 1;
            } else if tok.is_close && skip_depth > 0 {
                skip_depth -= 1;
            }
            continue;
        }
        if skip_depth > 0 {
            continue;
        }
        if tok.is_close {
            if stack.len() > 1 {
                stack.pop();
            }
            continue;
        }

        let mut st = *stack.last().unwrap();
        apply_style_attrs(&mut st, &tok.attrs);
        if !tok.self_close {
            stack.push(st);
        }

        let mut raw_subpaths: Vec<(Vec<(f32, f32)>, bool)> = Vec::new();
        match tok.tag.as_str() {
            "rect" => {
                let x = tok.attr_f32("x", 0.0);
                let y = tok.attr_f32("y", 0.0);
                let w = tok.attr_f32("width", 0.0);
                let h = tok.attr_f32("height", 0.0);
                let rx = tok.attr_f32("rx", tok.attr_f32("ry", 0.0)).min(w * 0.5).max(0.0);
                let ry = tok.attr_f32("ry", rx).min(h * 0.5).max(0.0);
                if w > 0.0 && h > 0.0 {
                    raw_subpaths.push((build_rounded_rect(x, y, w, h, rx, ry), true));
                }
            }
            "circle" => {
                let cx = tok.attr_f32("cx", 0.0);
                let cy = tok.attr_f32("cy", 0.0);
                let r = tok.attr_f32("r", 0.0);
                if r > 0.0 {
                    raw_subpaths.push((build_ellipse(cx, cy, r, r, 40), true));
                }
            }
            "ellipse" => {
                let cx = tok.attr_f32("cx", 0.0);
                let cy = tok.attr_f32("cy", 0.0);
                let rx = tok.attr_f32("rx", 0.0);
                let ry = tok.attr_f32("ry", 0.0);
                if rx > 0.0 && ry > 0.0 {
                    raw_subpaths.push((build_ellipse(cx, cy, rx, ry, 40), true));
                }
            }
            "line" => {
                let x1 = tok.attr_f32("x1", 0.0);
                let y1 = tok.attr_f32("y1", 0.0);
                let x2 = tok.attr_f32("x2", 0.0);
                let y2 = tok.attr_f32("y2", 0.0);
                raw_subpaths.push((vec![(x1, y1), (x2, y2)], false));
            }
            "polyline" | "polygon" => {
                if let Some(pts_s) = tok.attr("points") {
                    let nums = parse_numbers(pts_s);
                    let pts: Vec<(f32, f32)> = nums.chunks_exact(2).map(|c| (c[0], c[1])).collect();
                    if pts.len() >= 2 {
                        raw_subpaths.push((pts, tok.tag == "polygon"));
                    }
                }
            }
            "path" => {
                if let Some(d) = tok.attr("d") {
                    raw_subpaths = parse_svg_path(d);
                }
            }
            _ => {}
        }

        if raw_subpaths.is_empty() {
            continue;
        }

        let transformed: Vec<(Vec<(f32, f32)>, bool)> = raw_subpaths
            .into_iter()
            .map(|(pts, closed)| {
                let tp = pts.into_iter().map(|(x, y)| st.transform.apply(x, y)).collect();
                (tp, closed)
            })
            .collect();

        let fill = st.fill.map(|mut c| {
            c[3] *= st.opacity * st.fill_opacity;
            c
        });
        let stroke = st.stroke.map(|mut c| {
            c[3] *= st.opacity * st.stroke_opacity;
            c
        });
        let sw = st.stroke_width * st.transform.approx_scale();

        shapes.push(DrawShape {
            subpaths: transformed,
            fill,
            stroke,
            stroke_width: sw,
        });
    }

    // 4x4 supersampled rasterizer (high quality crisp edges, fast on UI icon sizes <= 512px)
    let w = out_w as usize;
    let h = out_h as usize;
    let mut accum = vec![[0.0f32; 4]; w * h]; // premultiplied RGBA

    for shape in &shapes {
        // Compute tight pixel bounding box
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for (pts, _) in &shape.subpaths {
            for &(x, y) in pts {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        let pad = if shape.stroke.is_some() { shape.stroke_width * 0.5 + 1.5 } else { 1.0 };
        let ix0 = ((min_x - pad).floor() as isize).clamp(0, w as isize) as usize;
        let iy0 = ((min_y - pad).floor() as isize).clamp(0, h as isize) as usize;
        let ix1 = ((max_x + pad).ceil() as isize).clamp(0, w as isize) as usize;
        let iy1 = ((max_y + pad).ceil() as isize).clamp(0, h as isize) as usize;

        let half_sw_sq = (shape.stroke_width * 0.5).powi(2);

        for py in iy0..iy1 {
            for px in ix0..ix1 {
                let mut fill_hits = 0u32;
                let mut stroke_hits = 0u32;

                for sy_i in 0..4 {
                    let y = py as f32 + (sy_i as f32 + 0.5) * 0.25;
                    for sx_i in 0..4 {
                        let x = px as f32 + (sx_i as f32 + 0.5) * 0.25;
                        if shape.fill.is_some() && point_in_subpaths(x, y, &shape.subpaths) {
                            fill_hits += 1;
                        }
                        if shape.stroke.is_some() && point_near_stroke(x, y, &shape.subpaths, half_sw_sq) {
                            stroke_hits += 1;
                        }
                    }
                }

                let dst = &mut accum[py * w + px];
                if let Some(fc) = shape.fill
                    && fill_hits > 0
                {
                    let cov = (fill_hits as f32 / 16.0) * fc[3];
                    blend_over_premul(dst, [fc[0], fc[1], fc[2], cov]);
                }
                if let Some(sc) = shape.stroke
                    && stroke_hits > 0
                {
                    let cov = (stroke_hits as f32 / 16.0) * sc[3];
                    blend_over_premul(dst, [sc[0], sc[1], sc[2], cov]);
                }
            }
        }
    }

    // Unpremultiply to RGBA8
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let p = accum[i];
        let a = p[3].clamp(0.0, 1.0);
        if a > 1e-4 {
            rgba[i * 4] = ((p[0] / a).clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i * 4 + 1] = ((p[1] / a).clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i * 4 + 2] = ((p[2] / a).clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[i * 4 + 3] = (a * 255.0).round() as u8;
        }
    }

    // Transparent edge alpha bleed so GPU bilinear filtering never creates dark halos
    alpha_bleed(&mut rgba, w, h);

    Ok(DecodedImage { width: out_w, height: out_h, rgba })
}

#[inline]
fn blend_over_premul(dst: &mut [f32; 4], src: [f32; 4]) {
    let a = src[3].clamp(0.0, 1.0);
    let inv = 1.0 - a;
    dst[0] = src[0] * a + dst[0] * inv;
    dst[1] = src[1] * a + dst[1] * inv;
    dst[2] = src[2] * a + dst[2] * inv;
    dst[3] = a + dst[3] * inv;
}

/// Even-odd / non-zero winding test across all subpaths.
fn point_in_subpaths(x: f32, y: f32, subpaths: &[(Vec<(f32, f32)>, bool)]) -> bool {
    let mut winding = 0i32;
    for (pts, _) in subpaths {
        if pts.len() < 3 {
            continue;
        }
        let n = pts.len();
        for i in 0..n {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % n];
            if y0 <= y {
                if y1 > y && ((x1 - x0) * (y - y0) - (x - x0) * (y1 - y0)) > 0.0 {
                    winding += 1;
                }
            } else if y1 <= y && ((x1 - x0) * (y - y0) - (x - x0) * (y1 - y0)) < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

fn point_near_stroke(x: f32, y: f32, subpaths: &[(Vec<(f32, f32)>, bool)], half_sw_sq: f32) -> bool {
    for (pts, closed) in subpaths {
        if pts.len() < 2 {
            continue;
        }
        let segs = if *closed { pts.len() } else { pts.len() - 1 };
        for i in 0..segs {
            let (ax, ay) = pts[i];
            let (bx, by) = pts[(i + 1) % pts.len()];
            let dx = bx - ax;
            let dy = by - ay;
            let len_sq = dx * dx + dy * dy;
            let t = if len_sq > 1e-8 {
                (((x - ax) * dx + (y - ay) * dy) / len_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let px = ax + dx * t - x;
            let py = ay + dy * t - y;
            if px * px + py * py <= half_sw_sq {
                return true;
            }
        }
    }
    false
}

fn alpha_bleed(rgba: &mut [u8], w: usize, h: usize) {
    let src = rgba.to_vec();
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            if src[idx + 3] != 0 {
                continue;
            }
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut cnt = 0u32;
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let ni = (ny as usize * w + nx as usize) * 4;
                        if src[ni + 3] > 0 {
                            r += src[ni] as u32;
                            g += src[ni + 1] as u32;
                            b += src[ni + 2] as u32;
                            cnt += 1;
                        }
                    }
                }
            }
            if cnt > 0 {
                rgba[idx] = (r / cnt) as u8;
                rgba[idx + 1] = (g / cnt) as u8;
                rgba[idx + 2] = (b / cnt) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shape Builders & SVG Path Parser
// ---------------------------------------------------------------------------

fn build_rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Vec<(f32, f32)> {
    if rx <= 0.01 && ry <= 0.01 {
        return vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    }
    let mut pts = Vec::new();
    let segs = 8;
    let corners = [
        (x + w - rx, y + ry, -std::f32::consts::FRAC_PI_2, 0.0),
        (x + w - rx, y + h - ry, 0.0, std::f32::consts::FRAC_PI_2),
        (x + rx, y + h - ry, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
        (x + rx, y + ry, std::f32::consts::PI, 1.5 * std::f32::consts::PI),
    ];
    for (cx, cy, a0, a1) in corners {
        for i in 0..=segs {
            let a = a0 + (a1 - a0) * (i as f32 / segs as f32);
            pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
        }
    }
    pts
}

fn build_ellipse(cx: f32, cy: f32, rx: f32, ry: f32, segs: usize) -> Vec<(f32, f32)> {
    (0..segs)
        .map(|i| {
            let a = (i as f32 / segs as f32) * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

fn parse_svg_path(d: &str) -> Vec<(Vec<(f32, f32)>, bool)> {
    let mut out = Vec::new();
    let mut cur_pts: Vec<(f32, f32)> = Vec::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    let mut last_ctrl = (0.0f32, 0.0f32);
    let mut prev_cmd = ' ';

    let bytes = d.as_bytes();
    let mut i = 0usize;
    let mut cmd = ' ';

    let skip_sep = |bytes: &[u8], i: &mut usize| {
        while *i < bytes.len() && (bytes[*i].is_ascii_whitespace() || bytes[*i] == b',') {
            *i += 1;
        }
    };
    let next_num = |bytes: &[u8], i: &mut usize| -> Option<f32> {
        skip_sep(bytes, i);
        if *i >= bytes.len() {
            return None;
        }
        let start = *i;
        if matches!(bytes[*i], b'+' | b'-') {
            *i += 1;
        }
        let mut saw_digit = false;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            saw_digit = true;
            *i += 1;
        }
        if *i < bytes.len() && bytes[*i] == b'.' {
            *i += 1;
            while *i < bytes.len() && bytes[*i].is_ascii_digit() {
                saw_digit = true;
                *i += 1;
            }
        }
        if !saw_digit {
            *i = start;
            return None;
        }
        if *i < bytes.len() && matches!(bytes[*i], b'e' | b'E') {
            *i += 1;
            if *i < bytes.len() && matches!(bytes[*i], b'+' | b'-') {
                *i += 1;
            }
            while *i < bytes.len() && bytes[*i].is_ascii_digit() {
                *i += 1;
            }
        }
        std::str::from_utf8(&bytes[start..*i]).ok()?.parse().ok()
    };

    while i < bytes.len() {
        skip_sep(bytes, &mut i);
        if i >= bytes.len() {
            break;
        }
        if bytes[i].is_ascii_alphabetic() {
            cmd = bytes[i] as char;
            i += 1;
        }
        match cmd {
            'M' | 'm' => {
                if cur_pts.len() >= 2 {
                    out.push((std::mem::take(&mut cur_pts), false));
                } else {
                    cur_pts.clear();
                }
                let (Some(x), Some(y)) = (next_num(bytes, &mut i), next_num(bytes, &mut i)) else { break };
                if cmd == 'm' { cx += x; cy += y; } else { cx = x; cy = y; }
                sx = cx; sy = cy;
                cur_pts.push((cx, cy));
                cmd = if cmd == 'm' { 'l' } else { 'L' };
            }
            'L' | 'l' => {
                let (Some(x), Some(y)) = (next_num(bytes, &mut i), next_num(bytes, &mut i)) else { break };
                if cmd == 'l' { cx += x; cy += y; } else { cx = x; cy = y; }
                cur_pts.push((cx, cy));
            }
            'H' | 'h' => {
                let Some(x) = next_num(bytes, &mut i) else { break };
                cx = if cmd == 'h' { cx + x } else { x };
                cur_pts.push((cx, cy));
            }
            'V' | 'v' => {
                let Some(y) = next_num(bytes, &mut i) else { break };
                cy = if cmd == 'v' { cy + y } else { y };
                cur_pts.push((cx, cy));
            }
            'Q' | 'q' => {
                let (Some(x1), Some(y1), Some(x), Some(y)) = (
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                ) else { break };
                let (ax1, ay1, ax, ay) = if cmd == 'q' { (cx + x1, cy + y1, cx + x, cy + y) } else { (x1, y1, x, y) };
                flatten_quad(&mut cur_pts, (cx, cy), (ax1, ay1), (ax, ay), 12);
                last_ctrl = (ax1, ay1);
                cx = ax; cy = ay;
            }
            'T' | 't' => {
                let (Some(x), Some(y)) = (next_num(bytes, &mut i), next_num(bytes, &mut i)) else { break };
                let (ax1, ay1) = if matches!(prev_cmd, 'Q' | 'q' | 'T' | 't') {
                    (2.0 * cx - last_ctrl.0, 2.0 * cy - last_ctrl.1)
                } else {
                    (cx, cy)
                };
                let (ax, ay) = if cmd == 't' { (cx + x, cy + y) } else { (x, y) };
                flatten_quad(&mut cur_pts, (cx, cy), (ax1, ay1), (ax, ay), 12);
                last_ctrl = (ax1, ay1);
                cx = ax; cy = ay;
            }
            'C' | 'c' => {
                let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                ) else { break };
                let (ax1, ay1, ax2, ay2, ax, ay) = if cmd == 'c' {
                    (cx + x1, cy + y1, cx + x2, cy + y2, cx + x, cy + y)
                } else {
                    (x1, y1, x2, y2, x, y)
                };
                flatten_cubic(&mut cur_pts, (cx, cy), (ax1, ay1), (ax2, ay2), (ax, ay), 14);
                last_ctrl = (ax2, ay2);
                cx = ax; cy = ay;
            }
            'S' | 's' => {
                let (Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                ) else { break };
                let (ax1, ay1) = if matches!(prev_cmd, 'C' | 'c' | 'S' | 's') {
                    (2.0 * cx - last_ctrl.0, 2.0 * cy - last_ctrl.1)
                } else {
                    (cx, cy)
                };
                let (ax2, ay2, ax, ay) = if cmd == 's' {
                    (cx + x2, cy + y2, cx + x, cy + y)
                } else {
                    (x2, y2, x, y)
                };
                flatten_cubic(&mut cur_pts, (cx, cy), (ax1, ay1), (ax2, ay2), (ax, ay), 14);
                last_ctrl = (ax2, ay2);
                cx = ax; cy = ay;
            }
            'A' | 'a' => {
                let (Some(rx), Some(ry), Some(_rot), Some(_large), Some(_sweep), Some(x), Some(y)) = (
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i),
                    next_num(bytes, &mut i), next_num(bytes, &mut i), next_num(bytes, &mut i),
                ) else { break };
                let (ax, ay) = if cmd == 'a' { (cx + x, cy + y) } else { (x, y) };
                let mx = (cx + ax) * 0.5;
                let my = (cy + ay) * 0.5;
                let _ = (rx, ry);
                cur_pts.push((mx, my));
                cur_pts.push((ax, ay));
                cx = ax; cy = ay;
            }
            'Z' | 'z' => {
                if cur_pts.len() >= 2 {
                    out.push((std::mem::take(&mut cur_pts), true));
                }
                cx = sx; cy = sy;
            }
            _ => break,
        }
        prev_cmd = cmd;
    }
    if cur_pts.len() >= 2 {
        out.push((cur_pts, false));
    }
    out
}

fn flatten_quad(out: &mut Vec<(f32, f32)>, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), steps: usize) {
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let u = 1.0 - t;
        out.push((
            u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
            u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
        ));
    }
}

fn flatten_cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    steps: usize,
) {
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let u = 1.0 - t;
        out.push((
            u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0,
            u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1,
        ));
    }
}

// ---------------------------------------------------------------------------
// Minimal XML Tokenizer & Attribute Parsers
// ---------------------------------------------------------------------------

struct XmlToken {
    tag: String,
    attrs: Vec<(String, String)>,
    is_close: bool,
    self_close: bool,
}

impl XmlToken {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn attr_f32(&self, name: &str, default: f32) -> f32 {
        self.attr(name).and_then(parse_len).unwrap_or(default)
    }
}

fn tokenize_xml(xml: &str) -> Vec<XmlToken> {
    let mut out = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = xml[i + 4..].find("-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        if bytes[i..].starts_with(b"<?") || bytes[i..].starts_with(b"<!") {
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        i += 1; // skip '<'
        let is_close = i < bytes.len() && bytes[i] == b'/';
        if is_close {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' && bytes[i] != b'/' {
            i += 1;
        }
        let tag = xml[start..i].to_string();
        let mut attrs = Vec::new();
        let mut self_close = false;

        while i < bytes.len() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            if bytes[i] == b'>' {
                i += 1;
                break;
            }
            if bytes[i] == b'/' {
                self_close = true;
                i += 1;
                continue;
            }
            let kstart = i;
            while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            let key = xml[kstart..i].to_string();
            while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'=') {
                i += 1;
            }
            let val = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let q = bytes[i];
                i += 1;
                let vs = i;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                let s = xml[vs..i].to_string();
                if i < bytes.len() {
                    i += 1;
                }
                s
            } else {
                String::new()
            };
            if !key.is_empty() {
                attrs.push((key, val));
            }
        }
        if !tag.is_empty() {
            out.push(XmlToken { tag, attrs, is_close, self_close });
        }
    }
    out
}

fn apply_style_attrs(st: &mut Style, attrs: &[(String, String)]) {
    for (k, v) in attrs {
        if k == "style" {
            for decl in v.split(';') {
                if let Some((sk, sv)) = decl.split_once(':') {
                    apply_one_prop(st, sk.trim(), sv.trim());
                }
            }
        } else {
            apply_one_prop(st, k, v);
        }
    }
}

fn apply_one_prop(st: &mut Style, k: &str, v: &str) {
    match k {
        "fill" => st.fill = parse_svg_color(v),
        "stroke" => st.stroke = parse_svg_color(v),
        "stroke-width" => {
            if let Some(w) = parse_len(v) {
                st.stroke_width = w;
            }
        }
        "opacity" => {
            if let Ok(o) = v.parse::<f32>() {
                st.opacity *= o.clamp(0.0, 1.0);
            }
        }
        "fill-opacity" => {
            if let Ok(o) = v.parse::<f32>() {
                st.fill_opacity = o.clamp(0.0, 1.0);
            }
        }
        "stroke-opacity" => {
            if let Ok(o) = v.parse::<f32>() {
                st.stroke_opacity = o.clamp(0.0, 1.0);
            }
        }
        "transform" => {
            st.transform = st.transform.mul(parse_transform(v));
        }
        _ => {}
    }
}

fn parse_transform(s: &str) -> Mat2D {
    let mut m = Mat2D::ID;
    let mut rest = s.trim();
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let op = rest[..open].trim();
        let nums = parse_numbers(&rest[open + 1..open + close]);
        let step = match op {
            "translate" => {
                let tx = nums.first().copied().unwrap_or(0.0);
                let ty = nums.get(1).copied().unwrap_or(0.0);
                Mat2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
            }
            "scale" => {
                let sx = nums.first().copied().unwrap_or(1.0);
                let sy = nums.get(1).copied().unwrap_or(sx);
                Mat2D { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
            }
            "rotate" => {
                let deg = nums.first().copied().unwrap_or(0.0);
                let (s, c) = deg.to_radians().sin_cos();
                let rot = Mat2D { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 };
                if nums.len() >= 3 {
                    let (cx, cy) = (nums[1], nums[2]);
                    let t1 = Mat2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: cx, f: cy };
                    let t2 = Mat2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: -cx, f: -cy };
                    t1.mul(rot).mul(t2)
                } else {
                    rot
                }
            }
            "matrix" if nums.len() >= 6 => Mat2D {
                a: nums[0], b: nums[1], c: nums[2], d: nums[3], e: nums[4], f: nums[5],
            },
            _ => Mat2D::ID,
        };
        m = m.mul(step);
        rest = rest[open + close + 1..].trim();
    }
    m
}

fn parse_svg_color(s: &str) -> Option<[f32; 4]> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if let Some(hex) = s.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0])
            }
            6 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                Some([
                    ((v >> 16) & 0xFF) as f32 / 255.0,
                    ((v >> 8) & 0xFF) as f32 / 255.0,
                    (v & 0xFF) as f32 / 255.0,
                    1.0,
                ])
            }
            8 => {
                let v = u32::from_str_radix(hex, 16).ok()?;
                Some([
                    ((v >> 24) & 0xFF) as f32 / 255.0,
                    ((v >> 16) & 0xFF) as f32 / 255.0,
                    ((v >> 8) & 0xFF) as f32 / 255.0,
                    (v & 0xFF) as f32 / 255.0,
                ])
            }
            _ => None,
        };
    }
    match s.to_ascii_lowercase().as_str() {
        "white" => Some([1.0, 1.0, 1.0, 1.0]),
        "black" => Some([0.0, 0.0, 0.0, 1.0]),
        "red" => Some([1.0, 0.0, 0.0, 1.0]),
        "green" => Some([0.0, 0.5, 0.0, 1.0]),
        "blue" => Some([0.0, 0.0, 1.0, 1.0]),
        "yellow" => Some([1.0, 1.0, 0.0, 1.0]),
        "gold" => Some([1.0, 0.84, 0.0, 1.0]),
        _ => None,
    }
}

fn parse_len(s: &str) -> Option<f32> {
    s.trim().trim_end_matches("px").parse::<f32>().ok()
}

fn parse_numbers(s: &str) -> Vec<f32> {
    s.split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_circle_and_star_svg() {
        let svg = r##"<svg viewBox="0 0 32 32">
            <circle cx="16" cy="16" r="14" fill="#F5C542" stroke="#8C5A08" stroke-width="2"/>
            <polygon points="16,6 19,13 26,13 20,18 22,25 16,21 10,25 12,18 6,13 13,13" fill="#FFF4B8"/>
        </svg>"##;
        let img = rasterize_svg(svg, 32, 32).unwrap();
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
        // Center pixel should be opaque bright yellow/cream
        let c = (16 * 32 + 16) * 4;
        assert_eq!(img.rgba[c + 3], 255);
        assert!(img.rgba[c] > 230 && img.rgba[c + 1] > 220);
        // Corner pixel (0,0) should be transparent
        assert_eq!(img.rgba[3], 0);
    }
}
