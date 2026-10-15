//! A small 3D vector and bounding box for view math.

use std::ops::{Add, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }
    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn dist(self, o: Vec3) -> f64 {
        (self - o).len()
    }
    /// The unit vector, or `None` for a zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec3> {
        let l = self.len();
        (l > 1e-300 && l.is_finite()).then(|| self * (1.0 / l))
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    pub fn to_f32(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }
}

impl From<[f64; 3]> for Vec3 {
    fn from(a: [f64; 3]) -> Vec3 {
        Vec3::new(a[0], a[1], a[2])
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f64) -> Vec3 {
        Vec3::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// An axis-aligned bounding box; empty until a point is added.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb3 {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb3 {
    fn default() -> Self {
        Aabb3::EMPTY
    }
}

impl Aabb3 {
    pub const EMPTY: Aabb3 = Aabb3 {
        min: Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        max: Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    pub fn is_empty(&self) -> bool {
        !(self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z)
    }
    pub fn add(&mut self, p: Vec3) {
        if !p.is_finite() {
            return;
        }
        self.min = Vec3::new(self.min.x.min(p.x), self.min.y.min(p.y), self.min.z.min(p.z));
        self.max = Vec3::new(self.max.x.max(p.x), self.max.y.max(p.y), self.max.z.max(p.z));
    }
    /// Grow to hold `o` too (an empty box adds nothing).
    pub fn union(&mut self, o: Aabb3) {
        if !o.is_empty() {
            self.add(o.min);
            self.add(o.max);
        }
    }
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn diagonal(&self) -> f64 {
        if self.is_empty() { 0.0 } else { self.max.dist(self.min) }
    }
    /// The eight corners (bit 0 picks x, bit 1 y, bit 2 z: min or max).
    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|k| {
            Vec3::new(
                if k & 1 == 0 { self.min.x } else { self.max.x },
                if k & 2 == 0 { self.min.y } else { self.max.y },
                if k & 4 == 0 { self.min.z } else { self.max.z },
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_algebra_and_boxes() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::new(3.0, 4.0, 0.0).len(), 5.0);
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        let mut b = Aabb3::EMPTY;
        assert!(b.is_empty() && b.diagonal() == 0.0);
        b.add(Vec3::new(f64::NAN, 1.0, 1.0));
        assert!(b.is_empty());
        b.add(Vec3::ZERO);
        b.add(Vec3::new(2.0, 4.0, 4.0));
        assert_eq!(b.center(), Vec3::new(1.0, 2.0, 2.0));
        assert_eq!(b.diagonal(), 6.0);
    }
}
