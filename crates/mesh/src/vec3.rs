//! Small 3-vector helpers on `[f64; 3]` (floating point: measures and display, never meshing
//! decisions, which use [`crate::predicates`]).

pub(crate) type P = [f64; 3];

pub(crate) fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(crate) fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn det(a: P, b: P, c: P) -> f64 {
    dot(a, cross(b, c))
}

/// The angle between two vectors in degrees (0 when either is zero).
pub(crate) fn angle_deg(a: P, b: P) -> f64 {
    norm(cross(a, b)).atan2(dot(a, b)).to_degrees()
}
pub(crate) fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub(crate) fn dist(a: P, b: P) -> f64 {
    norm(sub(a, b))
}
pub(crate) fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
pub(crate) fn mid(a: P, b: P) -> P {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1]), 0.5 * (a[2] + b[2])]
}
