//! Flat, public-field 3D math primitives with zero trait/generic gymnastics.

use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;

#[inline]
pub fn deg_to_rad(deg: f32) -> f32 {
    deg * (PI / 180.0)
}

#[inline]
pub fn rad_to_deg(rad: f32) -> f32 {
    rad * (180.0 / PI)
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    x.max(lo).min(hi)
}

#[inline]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0).max(1e-6), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - clamp(t, 0.0, 1.0);
    1.0 - u * u * u
}

#[inline]
pub fn ease_in_out_quad(t: f32) -> f32 {
    let t = clamp(t, 0.0, 1.0);
    if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) * 0.5 }
}

/// Fast deterministic xorshift64 PRNG (zero external `rand` crate needed).
#[derive(Copy, Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub const fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed } }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Uniform `f32` in `[0.0, 1.0)`.
    #[inline]
    pub fn f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) * (1.0 / 16_777_216.0)
    }

    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    #[inline]
    pub fn usize(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive == 0 { 0 } else { (self.next_u64() as usize) % upper_exclusive }
    }
}

// ---------------------------------------------------------------------------
// Vec2
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn dot(self, b: Self) -> f32 {
        self.x * b.x + self.y * b.y
    }

    #[inline]
    pub fn len_sq(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }

    #[inline]
    pub fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-8 { self * (1.0 / l) } else { Self::ZERO }
    }

    #[inline]
    pub fn lerp(self, b: Self, t: f32) -> Self {
        Self::new(lerp(self.x, b.x, t), lerp(self.y, b.y, t))
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Self;
    #[inline]
    fn add(self, r: Self) -> Self {
        Self::new(self.x + r.x, self.y + r.y)
    }
}
impl Sub for Vec2 {
    type Output = Self;
    #[inline]
    fn sub(self, r: Self) -> Self {
        Self::new(self.x - r.x, self.y - r.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s)
    }
}
impl Div<f32> for Vec2 {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        Self::new(self.x / s, self.y / s)
    }
}
impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, r: Self) {
        self.x += r.x;
        self.y += r.y;
    }
}
impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, r: Self) {
        self.x -= r.x;
        self.y -= r.y;
    }
}
impl MulAssign<f32> for Vec2 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
    }
}
impl Neg for Vec2 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

// ---------------------------------------------------------------------------
// Vec3
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0, z: 1.0 };
    pub const X: Self = Self { x: 1.0, y: 0.0, z: 0.0 };
    pub const Y: Self = Self { x: 0.0, y: 1.0, z: 0.0 };
    pub const Z: Self = Self { x: 0.0, y: 0.0, z: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v, z: v }
    }

    #[inline]
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    #[inline]
    pub fn dot(self, b: Self) -> f32 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    #[inline]
    pub fn cross(self, b: Self) -> Self {
        Self::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }

    #[inline]
    pub fn len_sq(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn len(self) -> f32 {
        self.len_sq().sqrt()
    }

    #[inline]
    pub fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-8 { self * (1.0 / l) } else { Self::ZERO }
    }

    #[inline]
    pub fn dist(self, b: Self) -> f32 {
        (self - b).len()
    }

    #[inline]
    pub fn lerp(self, b: Self, t: f32) -> Self {
        Self::new(
            lerp(self.x, b.x, t),
            lerp(self.y, b.y, t),
            lerp(self.z, b.z, t),
        )
    }

    #[inline]
    pub fn min(self, b: Self) -> Self {
        Self::new(self.x.min(b.x), self.y.min(b.y), self.z.min(b.z))
    }

    #[inline]
    pub fn max(self, b: Self) -> Self {
        Self::new(self.x.max(b.x), self.y.max(b.y), self.z.max(b.z))
    }

    #[inline]
    pub fn mul_elem(self, b: Self) -> Self {
        Self::new(self.x * b.x, self.y * b.y, self.z * b.z)
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Self;
    #[inline]
    fn add(self, r: Self) -> Self {
        Self::new(self.x + r.x, self.y + r.y, self.z + r.z)
    }
}
impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, r: Self) {
        self.x += r.x;
        self.y += r.y;
        self.z += r.z;
    }
}
impl Sub for Vec3 {
    type Output = Self;
    #[inline]
    fn sub(self, r: Self) -> Self {
        Self::new(self.x - r.x, self.y - r.y, self.z - r.z)
    }
}
impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, r: Self) {
        self.x -= r.x;
        self.y -= r.y;
        self.z -= r.z;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Mul<Vec3> for f32 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(self * v.x, self * v.y, self * v.z)
    }
}
impl MulAssign<f32> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
        self.z *= s;
    }
}
impl Div<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn div(self, s: f32) -> Self {
        self * (1.0 / s)
    }
}
impl Neg for Vec3 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

// ---------------------------------------------------------------------------
// Vec4
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0, w: 0.0 };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    #[inline]
    pub const fn from_vec3(v: Vec3, w: f32) -> Self {
        Self { x: v.x, y: v.y, z: v.z, w }
    }

    #[inline]
    pub fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }
}

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const RED: Self = Self::rgb(0.92, 0.24, 0.22);
    pub const GREEN: Self = Self::rgb(0.24, 0.82, 0.36);
    pub const BLUE: Self = Self::rgb(0.24, 0.52, 0.96);
    pub const YELLOW: Self = Self::rgb(0.96, 0.82, 0.22);
    pub const GRAY: Self = Self::rgb(0.5, 0.5, 0.5);

    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub fn from_hex(hex: u32) -> Self {
        let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
        let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
        let b = (hex & 0xFF) as f32 / 255.0;
        Self::rgb(r, g, b)
    }

    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

// ---------------------------------------------------------------------------
// Quat (unit quaternion, x/y/z imaginary + w scalar)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Self = Self { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Self {
        let a = axis.norm();
        let (s, c) = (radians * 0.5).sin_cos();
        Self::new(a.x * s, a.y * s, a.z * s, c)
    }

    /// Yaw (Y), then Pitch (X), then Roll (Z) in radians.
    pub fn from_euler_yxz(yaw: f32, pitch: f32, roll: f32) -> Self {
        let qy = Self::from_axis_angle(Vec3::Y, yaw);
        let qx = Self::from_axis_angle(Vec3::X, pitch);
        let qz = Self::from_axis_angle(Vec3::Z, roll);
        qy * qx * qz
    }

    #[inline]
    pub fn norm(self) -> Self {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if l > 1e-8 {
            let inv = 1.0 / l;
            Self::new(self.x * inv, self.y * inv, self.z * inv, self.w * inv)
        } else {
            Self::IDENTITY
        }
    }

    #[inline]
    pub fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }

    #[inline]
    pub fn mul_vec3(self, v: Vec3) -> Vec3 {
        let qv = Vec3::new(self.x, self.y, self.z);
        let t = qv.cross(v) * 2.0;
        v + t * self.w + qv.cross(t)
    }

    pub fn slerp(self, mut b: Self, t: f32) -> Self {
        let mut dot = self.x * b.x + self.y * b.y + self.z * b.z + self.w * b.w;
        if dot < 0.0 {
            dot = -dot;
            b = Self::new(-b.x, -b.y, -b.z, -b.w);
        }
        if dot > 0.9995 {
            return Self::new(
                lerp(self.x, b.x, t),
                lerp(self.y, b.y, t),
                lerp(self.z, b.z, t),
                lerp(self.w, b.w, t),
            )
            .norm();
        }
        let theta = clamp(dot, -1.0, 1.0).acos();
        let sin_t = theta.sin();
        let w0 = ((1.0 - t) * theta).sin() / sin_t;
        let w1 = (t * theta).sin() / sin_t;
        Self::new(
            self.x * w0 + b.x * w1,
            self.y * w0 + b.y * w1,
            self.z * w0 + b.z * w1,
            self.w * w0 + b.w * w1,
        )
    }
}

impl Mul for Quat {
    type Output = Self;
    #[inline]
    fn mul(self, b: Self) -> Self {
        Self::new(
            self.w * b.x + self.x * b.w + self.y * b.z - self.z * b.y,
            self.w * b.y - self.x * b.z + self.y * b.w + self.z * b.x,
            self.w * b.z + self.x * b.y - self.y * b.x + self.z * b.w,
            self.w * b.w - self.x * b.x - self.y * b.y - self.z * b.z,
        )
    }
}

// ---------------------------------------------------------------------------
// Mat4 (column-major, OpenGL convention: m[col * 4 + row])
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Mat4 {
    pub m: [f32; 16],
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        m: [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ],
    };

    #[inline]
    pub fn translation(t: Vec3) -> Self {
        Self {
            m: [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                t.x, t.y, t.z, 1.0,
            ],
        }
    }

    #[inline]
    pub fn scale(s: Vec3) -> Self {
        Self {
            m: [
                s.x, 0.0, 0.0, 0.0,
                0.0, s.y, 0.0, 0.0,
                0.0, 0.0, s.z, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn from_quat(q: Quat) -> Self {
        let q = q.norm();
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;
        let xy = x * y;
        let xz = x * z;
        let yz = y * z;
        let wx = w * x;
        let wy = w * y;
        let wz = w * z;

        Self {
            m: [
                1.0 - 2.0 * (yy + zz), 2.0 * (xy + wz),       2.0 * (xz - wy),       0.0,
                2.0 * (xy - wz),       1.0 - 2.0 * (xx + zz), 2.0 * (yz + wx),       0.0,
                2.0 * (xz + wy),       2.0 * (yz - wx),       1.0 - 2.0 * (xx + yy), 0.0,
                0.0,                   0.0,                   0.0,                   1.0,
            ],
        }
    }

    pub fn from_trs(pos: Vec3, rot: Quat, scale: Vec3) -> Self {
        let mut r = Self::from_quat(rot);
        r.m[0] *= scale.x;
        r.m[1] *= scale.x;
        r.m[2] *= scale.x;
        r.m[4] *= scale.y;
        r.m[5] *= scale.y;
        r.m[6] *= scale.y;
        r.m[8] *= scale.z;
        r.m[9] *= scale.z;
        r.m[10] *= scale.z;
        r.m[12] = pos.x;
        r.m[13] = pos.y;
        r.m[14] = pos.z;
        r
    }

    /// Right-handed OpenGL perspective projection (NDC z in [-1, 1]).
    pub fn perspective(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y_rad * 0.5).tan();
        let nf = 1.0 / (near - far);
        Self {
            m: [
                f / aspect, 0.0, 0.0,                     0.0,
                0.0,        f,   0.0,                     0.0,
                0.0,        0.0, (far + near) * nf,      -1.0,
                0.0,        0.0, 2.0 * far * near * nf,   0.0,
            ],
        }
    }

    /// Right-handed OpenGL orthographic projection.
    pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let rl = right - left;
        let tb = top - bottom;
        let fn_ = far - near;
        Self {
            m: [
                2.0 / rl,                 0.0,                      0.0,                   0.0,
                0.0,                      2.0 / tb,                 0.0,                   0.0,
                0.0,                      0.0,                     -2.0 / fn_,             0.0,
                -(right + left) / rl,    -(top + bottom) / tb,     -(far + near) / fn_,    1.0,
            ],
        }
    }

    /// Right-handed look-at view matrix.
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let f = (target - eye).norm();
        let s = f.cross(up).norm();
        let u = s.cross(f);
        Self {
            m: [
                s.x,         u.x,        -f.x,        0.0,
                s.y,         u.y,        -f.y,        0.0,
                s.z,         u.z,        -f.z,        0.0,
                -s.dot(eye), -u.dot(eye), f.dot(eye), 1.0,
            ],
        }
    }

    #[inline]
    pub fn mul_vec4(self, v: Vec4) -> Vec4 {
        let m = &self.m;
        Vec4::new(
            m[0] * v.x + m[4] * v.y + m[8] * v.z + m[12] * v.w,
            m[1] * v.x + m[5] * v.y + m[9] * v.z + m[13] * v.w,
            m[2] * v.x + m[6] * v.y + m[10] * v.z + m[14] * v.w,
            m[3] * v.x + m[7] * v.y + m[11] * v.z + m[15] * v.w,
        )
    }

    #[inline]
    pub fn transform_point(self, p: Vec3) -> Vec3 {
        self.mul_vec4(Vec4::from_vec3(p, 1.0)).xyz()
    }
}

impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let a = &self.m;
        let b = &rhs.m;
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = a[0 * 4 + row] * b[col * 4 + 0]
                    + a[1 * 4 + row] * b[col * 4 + 1]
                    + a[2 * 4 + row] * b[col * 4 + 2]
                    + a[3 * 4 + row] * b[col * 4 + 3];
            }
        }
        Self { m: out }
    }
}

// ---------------------------------------------------------------------------
// Transform, Aabb, Ray
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform {
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        pos: Vec3::ZERO,
        rot: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    #[inline]
    pub fn from_pos(pos: Vec3) -> Self {
        Self { pos, ..Self::IDENTITY }
    }

    #[inline]
    pub fn from_pos_scale(pos: Vec3, scale: Vec3) -> Self {
        Self { pos, rot: Quat::IDENTITY, scale }
    }

    #[inline]
    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_trs(self.pos, self.rot, self.scale)
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    #[inline]
    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    #[inline]
    pub fn from_center_half(center: Vec3, half: Vec3) -> Self {
        Self { min: center - half, max: center + half }
    }

    #[inline]
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn half_extents(self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    #[inline]
    pub fn intersects(self, b: Self) -> bool {
        self.min.x <= b.max.x
            && self.max.x >= b.min.x
            && self.min.y <= b.max.y
            && self.max.y >= b.min.y
            && self.min.z <= b.max.z
            && self.max.z >= b.min.z
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    #[inline]
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir: dir.norm() }
    }

    #[inline]
    pub fn at(self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Slab ray-AABB intersection; returns entry distance t >= 0 if hit.
    pub fn hit_aabb(self, b: Aabb) -> Option<f32> {
        let mut tmin = 0.0f32;
        let mut tmax = f32::INFINITY;
        let orig = self.origin.to_array();
        let dir = self.dir.to_array();
        let bmin = b.min.to_array();
        let bmax = b.max.to_array();
        for i in 0..3 {
            if dir[i].abs() < 1e-8 {
                if orig[i] < bmin[i] || orig[i] > bmax[i] {
                    return None;
                }
            } else {
                let inv = 1.0 / dir[i];
                let mut t1 = (bmin[i] - orig[i]) * inv;
                let mut t2 = (bmax[i] - orig[i]) * inv;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tmin = tmin.max(t1);
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return None;
                }
            }
        }
        Some(tmin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec3_cross_and_quat_rotate() {
        let c = Vec3::X.cross(Vec3::Y);
        assert!((c - Vec3::Z).len() < 1e-6);

        let q = Quat::from_axis_angle(Vec3::Y, PI * 0.5);
        let r = q.mul_vec3(Vec3::Z);
        assert!((r - Vec3::X).len() < 1e-5);
    }

    #[test]
    fn mat4_trs_and_ray_aabb() {
        let m = Transform {
            pos: Vec3::new(1.0, 2.0, 3.0),
            rot: Quat::from_axis_angle(Vec3::Y, PI * 0.5),
            scale: Vec3::new(2.0, 2.0, 2.0),
        }
        .to_mat4();
        let p = m.transform_point(Vec3::Z);
        assert!((p - Vec3::new(3.0, 2.0, 3.0)).len() < 1e-5);

        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = ray.hit_aabb(Aabb::from_center_half(Vec3::ZERO, Vec3::ONE));
        assert!((hit.unwrap() - 4.0).abs() < 1e-5);
    }
}
