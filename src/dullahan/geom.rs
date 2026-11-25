//! Flat, opt-in collision, raycast, overlap, heightfield, and rigid-body physics helpers.
//!
//! Call directly inside flat `update()` loops—no mandatory physics world or hidden solver state.

use crate::dullahan::formats::png::DecodedHeight16;
use crate::dullahan::math::{Aabb, Quat, Ray, Vec2, Vec3, clamp, lerp};

// ---------------------------------------------------------------------------
// 2D Collision & Overlap Helpers
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect2D {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect2D {
    #[inline]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    #[inline]
    pub fn overlaps(&self, b: &Rect2D) -> bool {
        self.x < b.x + b.w && self.x + self.w > b.x && self.y < b.y + b.h && self.y + self.h > b.y
    }
}

#[inline]
pub fn circle_overlaps(a_pos: Vec2, a_r: f32, b_pos: Vec2, b_r: f32) -> bool {
    (a_pos - b_pos).len_sq() < (a_r + b_r) * (a_r + b_r)
}

/// Minimum translation vector to push a 2D circle out of a `Rect2D`.
pub fn circle_rect_pushout(center: Vec2, radius: f32, rect: Rect2D) -> Option<Vec2> {
    let closest = Vec2::new(
        clamp(center.x, rect.x, rect.x + rect.w),
        clamp(center.y, rect.y, rect.y + rect.h),
    );
    let delta = center - closest;
    let d2 = delta.len_sq();
    if d2 >= radius * radius {
        return None;
    }
    if d2 > 1e-8 {
        let d = d2.sqrt();
        Some(delta * ((radius - d) / d))
    } else {
        Some(Vec2::new(0.0, -radius))
    }
}

// ---------------------------------------------------------------------------
// 3D Raycasts, Overlaps & Pushout Solvers
// ---------------------------------------------------------------------------

#[inline]
pub fn sphere_overlaps(a_pos: Vec3, a_r: f32, b_pos: Vec3, b_r: f32) -> bool {
    (a_pos - b_pos).len_sq() < (a_r + b_r) * (a_r + b_r)
}

/// Ray vs sphere intersection; returns entry distance `t >= 0` if hit.
pub fn ray_sphere(ray: Ray, center: Vec3, radius: f32) -> Option<f32> {
    let oc = ray.origin - center;
    let b = oc.dot(ray.dir);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sqrt_d = disc.sqrt();
    let t0 = -b - sqrt_d;
    if t0 >= 0.0 {
        Some(t0)
    } else {
        let t1 = -b + sqrt_d;
        if t1 >= 0.0 { Some(t1) } else { None }
    }
}

/// Ray vs infinite plane; returns `t >= 0` if hit.
pub fn ray_plane(ray: Ray, plane_point: Vec3, plane_normal: Vec3) -> Option<f32> {
    let denom = ray.dir.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_point - ray.origin).dot(plane_normal) / denom;
    if t >= 0.0 { Some(t) } else { None }
}

/// Möller–Trumbore ray-triangle intersection; returns `t >= 0` if hit.
pub fn ray_triangle(ray: Ray, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let h = ray.dir.cross(e2);
    let a = e1.dot(h);
    if a.abs() < 1e-7 {
        return None;
    }
    let f = 1.0 / a;
    let s = ray.origin - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = f * ray.dir.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * e2.dot(q);
    if t >= 0.0 { Some(t) } else { None }
}

/// Minimum translation vector to push `mover` out of `solid` along the shallowest axis.
pub fn aabb_pushout(mover: Aabb, solid: Aabb) -> Option<Vec3> {
    if !mover.intersects(solid) {
        return None;
    }
    let ox_pos = solid.max.x - mover.min.x;
    let ox_neg = solid.min.x - mover.max.x;
    let ox = if ox_pos.abs() < ox_neg.abs() { ox_pos } else { ox_neg };

    let oy_pos = solid.max.y - mover.min.y;
    let oy_neg = solid.min.y - mover.max.y;
    let oy = if oy_pos.abs() < oy_neg.abs() { oy_pos } else { oy_neg };

    let oz_pos = solid.max.z - mover.min.z;
    let oz_neg = solid.min.z - mover.max.z;
    let oz = if oz_pos.abs() < oz_neg.abs() { oz_pos } else { oz_neg };

    let (ax, ay, az) = (ox.abs(), oy.abs(), oz.abs());
    if ax <= ay && ax <= az {
        Some(Vec3::new(ox, 0.0, 0.0))
    } else if ay <= az {
        Some(Vec3::new(0.0, oy, 0.0))
    } else {
        Some(Vec3::new(0.0, 0.0, oz))
    }
}

/// Minimum pushout vector to resolve a sphere penetrating an AABB.
pub fn sphere_aabb_pushout(center: Vec3, radius: f32, box_: Aabb) -> Option<Vec3> {
    let closest = Vec3::new(
        clamp(center.x, box_.min.x, box_.max.x),
        clamp(center.y, box_.min.y, box_.max.y),
        clamp(center.z, box_.min.z, box_.max.z),
    );
    let delta = center - closest;
    let dist_sq = delta.len_sq();
    if dist_sq >= radius * radius {
        return None;
    }
    if dist_sq > 1e-10 {
        let dist = dist_sq.sqrt();
        Some(delta * ((radius - dist) / dist))
    } else {
        let mut best_axis = Vec3::Y;
        let mut best_pen = f32::INFINITY;
        let faces = [
            (box_.max.x - center.x, Vec3::X),
            (center.x - box_.min.x, -Vec3::X),
            (box_.max.y - center.y, Vec3::Y),
            (center.y - box_.min.y, -Vec3::Y),
            (box_.max.z - center.z, Vec3::Z),
            (center.z - box_.min.z, -Vec3::Z),
        ];
        for (dist, dir) in faces {
            if dist < best_pen {
                best_pen = dist;
                best_axis = dir;
            }
        }
        Some(best_axis * (best_pen + radius))
    }
}

/// Vertical character capsule (`seg_a` bottom sphere center to `seg_b` top sphere center, `radius`)
/// vs AABB pushout vector.
pub fn capsule_aabb_pushout(seg_a: Vec3, seg_b: Vec3, radius: f32, box_: Aabb) -> Option<Vec3> {
    // Find closest point on vertical/arbitrary segment to box center, then test sphere at that point.
    let box_c = box_.center();
    let ab = seg_b - seg_a;
    let ab_len_sq = ab.len_sq();
    let t = if ab_len_sq > 1e-8 {
        clamp((box_c - seg_a).dot(ab) / ab_len_sq, 0.0, 1.0)
    } else {
        0.0
    };
    let sample = seg_a + ab * t;
    // Also check endpoints in case the box edge is closer to an end cap.
    let mut best: Option<Vec3> = None;
    let mut best_len_sq = 0.0f32;
    for pt in [sample, seg_a, seg_b] {
        if let Some(push) = sphere_aabb_pushout(pt, radius, box_) {
            let l2 = push.len_sq();
            if l2 > best_len_sq {
                best_len_sq = l2;
                best = Some(push);
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------
// HeightField (directly consumes 16-bit PNG heightmaps)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct HeightField {
    pub width: u32,
    pub depth: u32,
    pub world_size: Vec3,
    pub origin: Vec3,
    pub heights: Vec<f32>,
}

impl HeightField {
    pub fn from_height16(h16: &DecodedHeight16, world_size: Vec3, origin: Vec3) -> Self {
        let heights = h16
            .samples
            .iter()
            .map(|&s| (s as f32 / 65535.0) * world_size.y)
            .collect();
        Self {
            width: h16.width,
            depth: h16.height,
            world_size,
            origin,
            heights,
        }
    }

    pub fn sample_height(&self, wx: f32, wz: f32) -> f32 {
        if self.width < 2 || self.depth < 2 {
            return self.origin.y;
        }
        let u = clamp((wx - self.origin.x) / self.world_size.x + 0.5, 0.0, 1.0);
        let v = clamp((wz - self.origin.z) / self.world_size.z + 0.5, 0.0, 1.0);
        let fx = u * (self.width - 1) as f32;
        let fz = v * (self.depth - 1) as f32;
        let x0 = (fx as u32).min(self.width - 2);
        let z0 = (fz as u32).min(self.depth - 2);
        let tx = fx - x0 as f32;
        let tz = fz - z0 as f32;

        let idx = |x: u32, z: u32| self.heights[(z * self.width + x) as usize];
        let h00 = idx(x0, z0);
        let h10 = idx(x0 + 1, z0);
        let h01 = idx(x0, z0 + 1);
        let h11 = idx(x0 + 1, z0 + 1);
        let h0 = lerp(h00, h10, tx);
        let h1 = lerp(h01, h11, tx);
        self.origin.y + lerp(h0, h1, tz)
    }

    pub fn sample_normal(&self, wx: f32, wz: f32) -> Vec3 {
        let eps_x = (self.world_size.x / (self.width.max(2) - 1) as f32) * 0.25;
        let eps_z = (self.world_size.z / (self.depth.max(2) - 1) as f32) * 0.25;
        let hl = self.sample_height(wx - eps_x, wz);
        let hr = self.sample_height(wx + eps_x, wz);
        let hd = self.sample_height(wx, wz - eps_z);
        let hu = self.sample_height(wx, wz + eps_z);
        let dh_dx = (hr - hl) / (2.0 * eps_x).max(1e-5);
        let dh_dz = (hu - hd) / (2.0 * eps_z).max(1e-5);
        Vec3::new(-dh_dx, 1.0, -dh_dz).norm()
    }

    /// Raymarch + binary refinement against the heightfield; returns hit distance `t` if intersected.
    pub fn raycast(&self, ray: Ray, max_dist: f32, step: f32) -> Option<f32> {
        let step = step.max(0.1);
        let mut prev_t = 0.0f32;
        let prev_p = ray.origin;
        let mut prev_above = prev_p.y >= self.sample_height(prev_p.x, prev_p.z);
        let mut t = step;
        while t <= max_dist {
            let p = ray.at(t);
            let above = p.y >= self.sample_height(p.x, p.z);
            if prev_above && !above {
                let mut lo = prev_t;
                let mut hi = t;
                for _ in 0..8 {
                    let mid = (lo + hi) * 0.5;
                    let mp = ray.at(mid);
                    if mp.y >= self.sample_height(mp.x, mp.z) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return Some((lo + hi) * 0.5);
            }
            prev_t = t;
            prev_above = above;
            t += step;
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Opt-in Flat RigidBody Solver (Impulse-based Bounce, Friction & Grounding)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct RigidBody {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub ang_vel: Vec3,
    pub radius: f32,
    pub inv_mass: f32,
    pub restitution: f32,
    pub friction: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub grounded: bool,
    pub ground_normal: Vec3,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            ang_vel: Vec3::ZERO,
            radius: 0.5,
            inv_mass: 1.0,
            restitution: 0.35,
            friction: 0.65,
            linear_damping: 0.15,
            angular_damping: 0.4,
            grounded: false,
            ground_normal: Vec3::Y,
        }
    }
}

impl RigidBody {
    pub fn apply_impulse(&mut self, impulse: Vec3) {
        self.vel += impulse * self.inv_mass;
    }

    pub fn step(
        &mut self,
        gravity: Vec3,
        static_boxes: &[Aabb],
        heightfield: Option<&HeightField>,
        ground_y: Option<f32>,
        dt: f32,
    ) {
        if self.inv_mass <= 0.0 {
            return;
        }
        self.grounded = false;
        self.ground_normal = Vec3::Y;

        self.vel += gravity * dt;
        let lin_damp = (1.0 - self.linear_damping * dt).max(0.0);
        let ang_damp = (1.0 - self.angular_damping * dt).max(0.0);
        self.vel *= lin_damp;
        self.ang_vel *= ang_damp;

        self.pos += self.vel * dt;
        let ang_speed = self.ang_vel.len();
        if ang_speed > 1e-5 {
            let dq = Quat::from_axis_angle(self.ang_vel / ang_speed, ang_speed * dt);
            self.rot = (dq * self.rot).norm();
        }

        // Resolve against optional HeightField
        if let Some(hf) = heightfield {
            let h = hf.sample_height(self.pos.x, self.pos.z);
            let pen = (h + self.radius) - self.pos.y;
            if pen > 0.0 {
                let n = hf.sample_normal(self.pos.x, self.pos.z);
                self.resolve_static_contact(n, pen, dt);
            }
        } else if let Some(gy) = ground_y {
            let pen = (gy + self.radius) - self.pos.y;
            if pen > 0.0 {
                self.resolve_static_contact(Vec3::Y, pen, dt);
            }
        }

        // Resolve against static AABBs
        for &b in static_boxes {
            if let Some(push) = sphere_aabb_pushout(self.pos, self.radius, b) {
                let pen = push.len();
                if pen > 1e-6 {
                    self.resolve_static_contact(push / pen, pen, dt);
                }
            }
        }
    }

    fn resolve_static_contact(&mut self, normal: Vec3, penetration: f32, dt: f32) {
        self.pos += normal * penetration;
        if normal.y > 0.55 {
            self.grounded = true;
            self.ground_normal = normal;
        }
        let vn = self.vel.dot(normal);
        if vn < 0.0 {
            let bounce = if vn.abs() > 0.6 { self.restitution } else { 0.0 };
            self.vel -= normal * ((1.0 + bounce) * vn);
            // Tangential Coulomb friction + rolling angular coupling
            let vt = self.vel - normal * self.vel.dot(normal);
            let drag = (1.0 - self.friction * (10.0 * dt).min(1.0)).max(0.0);
            self.vel = normal * self.vel.dot(normal) + vt * drag;
            if self.radius > 1e-4 {
                self.ang_vel = normal.cross(vt) / self.radius;
            }
        }
    }
}

/// Elastic impulse & positional separation between two dynamic `RigidBody` spheres.
pub fn resolve_body_pair(a: &mut RigidBody, b: &mut RigidBody) -> bool {
    let delta = b.pos - a.pos;
    let min_dist = a.radius + b.radius;
    let dist_sq = delta.len_sq();
    if dist_sq >= min_dist * min_dist {
        return false;
    }
    let inv_sum = a.inv_mass + b.inv_mass;
    if inv_sum <= 0.0 {
        return false;
    }
    let dist = dist_sq.sqrt();
    let normal = if dist > 1e-6 { delta / dist } else { Vec3::X };
    let pen = min_dist - dist;
    a.pos -= normal * (pen * (a.inv_mass / inv_sum));
    b.pos += normal * (pen * (b.inv_mass / inv_sum));

    let rel_v = b.vel - a.vel;
    let vn = rel_v.dot(normal);
    if vn < 0.0 {
        let e = a.restitution.min(b.restitution);
        let j = -(1.0 + e) * vn / inv_sum;
        let impulse = normal * j;
        a.vel -= impulse * a.inv_mass;
        b.vel += impulse * b.inv_mass;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_overlaps_heightfield_and_rigid_body() {
        // 2D overlap & pushout
        assert!(Rect2D::new(0.0, 0.0, 10.0, 10.0).overlaps(&Rect2D::new(5.0, 5.0, 10.0, 10.0)));
        assert!(circle_overlaps(Vec2::ZERO, 1.0, Vec2::new(1.5, 0.0), 1.0));
        let p2 = circle_rect_pushout(Vec2::new(10.5, 5.0), 1.0, Rect2D::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        assert!((p2.x - 0.5).abs() < 1e-5);

        // 3D raycasts & pushouts
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!((ray_sphere(ray, Vec3::ZERO, 1.0).unwrap() - 4.0).abs() < 1e-5);
        assert!(
            ray_triangle(
                ray,
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0)
            )
            .is_some()
        );
        let b = Aabb::from_center_half(Vec3::ZERO, Vec3::splat(0.5));
        let cap = capsule_aabb_pushout(Vec3::new(0.0, 0.8, 0.0), Vec3::new(0.0, 1.8, 0.0), 0.5, b).unwrap();
        assert!((cap.y - 0.2).abs() < 1e-5);

        // HeightField sampling & raycast
        let h16 = DecodedHeight16 { width: 2, height: 2, samples: vec![0, 65535, 0, 65535] };
        let hf = HeightField::from_height16(&h16, Vec3::new(10.0, 4.0, 10.0), Vec3::ZERO);
        assert!((hf.sample_height(0.0, 0.0) - 2.0).abs() < 1e-4);
        assert!(hf.sample_normal(0.0, 0.0).x < -0.2);
        let down_ray = Ray::new(Vec3::new(0.0, 10.0, 0.0), -Vec3::Y);
        assert!((hf.raycast(down_ray, 20.0, 0.5).unwrap() - 8.0).abs() < 0.05);

        // RigidBody drop & ground bounce + pair bounce
        let mut rb = RigidBody { pos: Vec3::new(0.0, 3.0, 0.0), radius: 0.5, restitution: 0.5, ..Default::default() };
        for _ in 0..120 {
            rb.step(Vec3::new(0.0, -20.0, 0.0), &[], None, Some(0.0), 1.0 / 60.0);
        }
        assert!(rb.grounded);
        assert!((rb.pos.y - 0.5).abs() < 0.05);

        let mut ba = RigidBody { pos: Vec3::new(-0.4, 1.0, 0.0), vel: Vec3::new(4.0, 0.0, 0.0), radius: 0.5, ..Default::default() };
        let mut bb = RigidBody { pos: Vec3::new(0.4, 1.0, 0.0), vel: Vec3::new(-4.0, 0.0, 0.0), radius: 0.5, ..Default::default() };
        assert!(resolve_body_pair(&mut ba, &mut bb));
        assert!(ba.vel.x < 0.0 && bb.vel.x > 0.0);
    }
}
