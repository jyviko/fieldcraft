//! An orbit camera about a target point, Z up, with perspective or orthographic projection.
//!
//! Adapted from SolidCraft's `crates/render/src/camera.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use serde::{Deserialize, Serialize};

use crate::pick::Projector;
use crate::{Aabb3, Vec3};

/// Column-major 4×4 matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f64; 4]; 4]);

impl Mat4 {
    pub fn mul(&self, o: &Mat4) -> Mat4 {
        let mut r = [[0.0; 4]; 4];
        for (c, col) in r.iter_mut().enumerate() {
            for (rr, cell) in col.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.0[k][rr] * o.0[c][k]).sum();
            }
        }
        Mat4(r)
    }
    /// Transform a point; returns (x, y, z, w).
    pub fn apply(&self, p: Vec3) -> [f64; 4] {
        let v = [p.x, p.y, p.z, 1.0];
        let mut out = [0.0; 4];
        for (r, o) in out.iter_mut().enumerate() {
            *o = (0..4).map(|c| self.0[c][r] * v[c]).sum();
        }
        out
    }
    pub fn to_f32(&self) -> [[f32; 4]; 4] {
        self.0.map(|c| c.map(|x| x as f32))
    }
}

/// Named view orientations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardView {
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
    Iso,
}

impl StandardView {
    pub const ALL: [StandardView; 7] = [
        StandardView::Front,
        StandardView::Back,
        StandardView::Left,
        StandardView::Right,
        StandardView::Top,
        StandardView::Bottom,
        StandardView::Iso,
    ];

    pub fn parse(s: &str) -> Option<StandardView> {
        Some(match s.to_ascii_lowercase().as_str() {
            "front" => StandardView::Front,
            "back" => StandardView::Back,
            "top" => StandardView::Top,
            "bottom" => StandardView::Bottom,
            "left" => StandardView::Left,
            "right" => StandardView::Right,
            "iso" | "isometric" => StandardView::Iso,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            StandardView::Front => "Front",
            StandardView::Back => "Back",
            StandardView::Top => "Top",
            StandardView::Bottom => "Bottom",
            StandardView::Left => "Left",
            StandardView::Right => "Right",
            StandardView::Iso => "Iso",
        }
    }
    /// (yaw, pitch) in radians. Yaw 0 looks along +Y (front view); the iso view looks from the
    /// front-right-top corner. Pitch is the elevation above the XY plane.
    pub fn angles(self) -> (f64, f64) {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        match self {
            StandardView::Front => (0.0, 0.0),
            StandardView::Back => (PI, 0.0),
            StandardView::Right => (-FRAC_PI_2, 0.0),
            StandardView::Left => (FRAC_PI_2, 0.0),
            StandardView::Top => (0.0, FRAC_PI_2),
            StandardView::Bottom => (0.0, -FRAC_PI_2),
            // atan(1/√2): the elevation of the cube diagonal.
            StandardView::Iso => (-FRAC_PI_4, 0.6154797086703874),
        }
    }
}

/// Where a camera looks from, in degrees as people and agents give it: `yaw` turns about Z (0
/// looks along +Y, the front view; −45 is the iso view's), `pitch` is the elevation above the XY
/// plane and `distance`, when given, the distance from the target (a zoom in orthographic views).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Orientation {
    pub yaw: f64,
    pub pitch: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
}

impl Orientation {
    /// Closest and farthest camera distance.
    pub const DISTANCE: (f64, f64) = (1e-3, 1e9);

    /// The orientation of a standard view.
    pub fn of(view: StandardView) -> Self {
        let (yaw, pitch) = view.angles();
        Orientation { yaw: yaw.to_degrees(), pitch: pitch.to_degrees(), distance: None }
    }

    /// A checked orientation: `None` when an angle or the distance is not finite or the distance
    /// is not above 0. The yaw is wrapped into [−180, 180), the pitch clamped to ±90 and the
    /// distance to [`Orientation::DISTANCE`].
    pub fn checked(yaw: f64, pitch: f64, distance: Option<f64>) -> Option<Self> {
        if !(yaw.is_finite() && pitch.is_finite()) || distance.is_some_and(|d| !(d.is_finite() && d > 0.0)) {
            return None;
        }
        let yaw = (yaw + 180.0).rem_euclid(360.0) - 180.0;
        let (near, far) = Self::DISTANCE;
        Some(Orientation { yaw, pitch: pitch.clamp(-90.0, 90.0), distance: distance.map(|d| d.clamp(near, far)) })
    }
}

/// Orbit camera around `target`, Z up.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub target: Vec3,
    /// Rotation about Z; 0 = looking along +Y (from the front).
    pub yaw: f64,
    /// Elevation above the XY plane, in [−π/2, π/2].
    pub pitch: f64,
    /// Distance from target to eye.
    pub distance: f64,
    /// Vertical field of view (radians); 0 = orthographic.
    pub fov: f64,
}

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = StandardView::Iso.angles();
        Camera { target: Vec3::ZERO, yaw, pitch, distance: 200.0, fov: 0.0 }
    }
}

impl Camera {
    /// Direction from the target toward the eye.
    pub fn back(&self) -> Vec3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        Vec3::new(-self.yaw.sin() * cp, -self.yaw.cos() * cp, sp).normalized().unwrap_or(Vec3::Z)
    }
    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }
    /// Camera basis: right, up, back (orthonormal).
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let b = self.back();
        let right = Vec3::new(self.yaw.cos(), -self.yaw.sin(), 0.0);
        let up = b.cross(right).normalized().unwrap_or(Vec3::Z);
        (right, up, b)
    }
    pub fn view(&self) -> Mat4 {
        let (r, u, b) = self.basis();
        let e = self.eye();
        Mat4([[r.x, u.x, b.x, 0.0], [r.y, u.y, b.y, 0.0], [r.z, u.z, b.z, 0.0], [-r.dot(e), -u.dot(e), -b.dot(e), 1.0]])
    }
    /// Half the visible height at the target distance.
    pub fn half_height(&self) -> f64 {
        if self.fov > 0.0 { self.distance * (self.fov * 0.5).tan() } else { self.distance * 0.4 }
    }
    /// Near/far planes that keep a scene of radius `radius` around the target visible.
    pub fn clip(&self, radius: f64) -> (f64, f64) {
        let r = radius.max(1e-3);
        if self.fov > 0.0 {
            ((self.distance - r * 2.0).max(self.distance * 1e-3).max(1e-3), self.distance + r * 2.0)
        } else {
            (self.distance - r * 4.0 - 1.0, self.distance + r * 4.0 + 1.0)
        }
    }
    /// Projection to clip space (wgpu style, depth 0..1).
    pub fn proj(&self, aspect: f64, near: f64, far: f64) -> Mat4 {
        let aspect = if aspect.is_finite() && aspect > 1e-6 { aspect } else { 1.0 };
        let (n, f) = (near, far.max(near + 1e-6));
        if self.fov > 0.0 {
            let t = 1.0 / (self.fov * 0.5).tan();
            Mat4([[t / aspect, 0.0, 0.0, 0.0], [0.0, t, 0.0, 0.0], [0.0, 0.0, f / (n - f), -1.0], [0.0, 0.0, n * f / (n - f), 0.0]])
        } else {
            let h = self.half_height();
            let w = h * aspect;
            Mat4([[1.0 / w, 0.0, 0.0, 0.0], [0.0, 1.0 / h, 0.0, 0.0], [0.0, 0.0, 1.0 / (n - f), 0.0], [0.0, 0.0, n / (n - f), 1.0]])
        }
    }
    pub fn view_proj(&self, aspect: f64, radius: f64) -> Mat4 {
        let (n, f) = self.clip(radius);
        self.proj(aspect, n, f).mul(&self.view())
    }
    /// World point → pixel coordinates (x right, y down) and depth (0 near … 1 far).
    /// For many points, a [`Projector`] computes the projection once.
    pub fn to_screen(&self, p: Vec3, w: f64, h: f64, radius: f64) -> Option<(f64, f64, f64)> {
        Projector::new(self, w, h, radius).screen(p).map(|[x, y, z]| (x, y, z))
    }
    /// Picking ray through pixel (x, y): origin and unit direction.
    pub fn ray(&self, x: f64, y: f64, w: f64, h: f64) -> (Vec3, Vec3) {
        let (r, u, b) = self.basis();
        let hh = self.half_height();
        let aspect = w / h.max(1.0);
        let nx = (x / w.max(1.0)) * 2.0 - 1.0;
        let ny = 1.0 - (y / h.max(1.0)) * 2.0;
        if self.fov > 0.0 {
            let d = (r * (nx * hh * aspect) + u * (ny * hh) - b * self.distance).normalized().unwrap_or(-b);
            (self.eye(), d)
        } else {
            let o = self.target + r * (nx * hh * aspect) + u * (ny * hh) + b * (self.distance * 4.0);
            (o, -b)
        }
    }
    /// The point under pixel (x, y) on the plane through the target facing the eye: the anchor
    /// that stays put when zooming at the cursor.
    pub fn target_plane_point(&self, x: f64, y: f64, w: f64, h: f64) -> Option<Vec3> {
        let (o, d) = self.ray(x, y, w, h);
        let b = self.back();
        let dn = d.dot(b);
        if dn.abs() < 1e-12 {
            return None;
        }
        let p = o + d * ((self.target - o).dot(b) / dn);
        p.is_finite().then_some(p)
    }
    /// Zoom by `factor` (< 1 zooms in) toward a world point that stays put on screen.
    pub fn zoom_at(&mut self, factor: f64, anchor: Option<Vec3>) {
        let f = if factor.is_finite() { factor.clamp(0.05, 20.0) } else { 1.0 };
        let nd = (self.distance * f).clamp(1e-3, 1e9);
        if let Some(a) = anchor.filter(|a| a.is_finite()) {
            self.target = a + (self.target - a) * (nd / self.distance);
        }
        self.distance = nd;
    }
    /// Pan by a screen delta in pixels for a viewport of height `h`.
    pub fn pan(&mut self, dx: f64, dy: f64, h: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let (r, u, _) = self.basis();
        let k = 2.0 * self.half_height() / h.max(1.0);
        self.target = self.target - r * (dx * k) + u * (dy * k);
    }
    /// Orbit by pixel deltas. The model follows the cursor: dragging right turns the side
    /// facing the viewer to the right.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let lim = std::f64::consts::FRAC_PI_2 - 1e-4;
        self.yaw = (self.yaw + dx * 0.008) % std::f64::consts::TAU;
        self.pitch = (self.pitch + dy * 0.008).clamp(-lim, lim);
    }
    pub fn set_view(&mut self, v: StandardView) {
        let lim = std::f64::consts::FRAC_PI_2 - 1e-6;
        let (y, p) = v.angles();
        self.yaw = y;
        self.pitch = p.clamp(-lim, lim);
    }
    /// Turn to `o` (and move to its distance, when it has one); the pitch stays just short of the
    /// poles, as an orbit's does.
    pub fn orient(&mut self, o: &Orientation) {
        let lim = std::f64::consts::FRAC_PI_2 - 1e-6;
        if o.yaw.is_finite() && o.pitch.is_finite() {
            self.yaw = o.yaw.to_radians();
            self.pitch = o.pitch.to_radians().clamp(-lim, lim);
        }
        if let Some(d) = o.distance.filter(|d| d.is_finite() && *d > 0.0) {
            self.distance = d.clamp(Orientation::DISTANCE.0, Orientation::DISTANCE.1);
        }
    }
    /// Frame a bounding box.
    pub fn fit(&mut self, b: &Aabb3) {
        if b.is_empty() {
            return;
        }
        self.target = b.center();
        let r = (b.diagonal() * 0.5).max(1e-3);
        self.distance = if self.fov > 0.0 { r / (self.fov * 0.5).sin() * 1.1 } else { r / 0.4 * 1.15 };
    }
    pub fn is_valid(&self) -> bool {
        self.target.is_finite() && self.yaw.is_finite() && self.pitch.is_finite() && self.distance.is_finite() && self.distance > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_view_projects_target_to_centre() {
        let mut c = Camera { target: Vec3::new(1.0, 2.0, 3.0), distance: 50.0, ..Default::default() };
        c.set_view(StandardView::Front);
        let (x, y, z) = c.to_screen(c.target, 800.0, 600.0, 10.0).unwrap();
        assert!((x - 400.0).abs() < 1e-6 && (y - 300.0).abs() < 1e-6 && (0.0..=1.0).contains(&z));
        // Front view looks along +Y: +X is to the right, +Z up.
        let (xr, _, _) = c.to_screen(c.target + Vec3::X, 800.0, 600.0, 10.0).unwrap();
        let (_, yu, _) = c.to_screen(c.target + Vec3::Z, 800.0, 600.0, 10.0).unwrap();
        assert!(xr > 400.0 && yu < 300.0);
        let (o, d) = c.ray(400.0, 300.0, 800.0, 600.0);
        assert!(d.dist(Vec3::Y) < 1e-9, "{d:?}");
        assert!((o - c.target).cross(d).len() < 1e-6);
    }

    #[test]
    fn horizontal_orbit_follows_the_cursor() {
        let mut c = Camera { distance: 50.0, ..Default::default() };
        c.set_view(StandardView::Front);
        let near = Vec3::new(0.0, -1.0, 0.0);
        let x0 = c.to_screen(near, 800.0, 600.0, 10.0).unwrap().0;
        c.orbit(20.0, 0.0);
        let x1 = c.to_screen(near, 800.0, 600.0, 10.0).unwrap().0;
        assert!(x1 > x0, "dragging right must move the near side right: {x0} -> {x1}");
    }

    #[test]
    fn top_view_and_perspective() {
        let mut c = Camera { distance: 100.0, fov: 0.8, ..Default::default() };
        c.set_view(StandardView::Top);
        let (xr, _, _) = c.to_screen(Vec3::X, 800.0, 600.0, 10.0).unwrap();
        let (_, yu, _) = c.to_screen(Vec3::Y, 800.0, 600.0, 10.0).unwrap();
        assert!(xr > 400.0 && yu < 300.0, "{xr} {yu}");
        let near = c.to_screen(Vec3::new(0.0, 0.0, 10.0), 800.0, 600.0, 20.0).unwrap().2;
        let far = c.to_screen(Vec3::new(0.0, 0.0, -10.0), 800.0, 600.0, 20.0).unwrap().2;
        assert!(near < far);
    }

    #[test]
    fn zoom_at_the_cursor_keeps_the_point_under_it() {
        let mut c = Camera { distance: 100.0, ..Default::default() };
        c.set_view(StandardView::Front);
        let a = c.target_plane_point(600.0, 200.0, 800.0, 600.0).unwrap();
        c.zoom_at(0.5, Some(a));
        let (x, y, _) = c.to_screen(a, 800.0, 600.0, 10.0).unwrap();
        assert!((x - 600.0).abs() < 1e-6 && (y - 200.0).abs() < 1e-6, "{x} {y}");
    }

    #[test]
    fn fit_and_hostile() {
        let mut c = Camera::default();
        let b = Aabb3 { min: Vec3::ZERO, max: Vec3::new(40.0, 30.0, 20.0) };
        c.fit(&b);
        assert!(c.target.dist(Vec3::new(20.0, 15.0, 10.0)) < 1e-9);
        c.zoom_at(f64::NAN, Some(Vec3::new(f64::NAN, 0.0, 0.0)));
        c.orbit(1e9, -1e9);
        c.orbit(f64::NAN, 0.0);
        c.pan(f64::INFINITY, 0.0, 100.0);
        assert!(c.is_valid());
        c.fit(&Aabb3::EMPTY);
        assert!(c.is_valid());
        for v in StandardView::ALL {
            assert_eq!(StandardView::parse(v.label()), Some(v));
        }
    }

    #[test]
    fn orientation_in_degrees() {
        // The iso view's angles, given in degrees, turn the camera as the iso view does.
        let (mut a, mut b) = (Camera::default(), Camera::default());
        a.set_view(StandardView::Front);
        a.orient(&Orientation::of(StandardView::Iso));
        b.set_view(StandardView::Iso);
        assert!((a.yaw - b.yaw).abs() < 1e-12 && (a.pitch - b.pitch).abs() < 1e-12);
        let o = Orientation::checked(-45.0, 35.264, Some(300.0)).unwrap();
        a.orient(&o);
        assert_eq!(a.distance, 300.0);
        // Wrapped, clamped, refused.
        let o = Orientation::checked(540.0, 1e6, Some(1e300)).unwrap();
        assert_eq!((o.yaw, o.pitch, o.distance), (-180.0, 90.0, Some(Orientation::DISTANCE.1)));
        a.orient(&o);
        assert!(a.is_valid() && a.pitch < std::f64::consts::FRAC_PI_2, "{a:?}");
        for (y, p, d) in
            [(f64::NAN, 0.0, None), (0.0, f64::INFINITY, None), (0.0, 0.0, Some(0.0)), (0.0, 0.0, Some(-1.0)), (0.0, 0.0, Some(f64::NAN))]
        {
            assert_eq!(Orientation::checked(y, p, d), None);
        }
        // A hostile orientation built by hand leaves the camera as it was.
        let before = a;
        a.orient(&Orientation { yaw: f64::NAN, pitch: 0.0, distance: Some(f64::INFINITY) });
        assert_eq!(a, before);
    }
}
