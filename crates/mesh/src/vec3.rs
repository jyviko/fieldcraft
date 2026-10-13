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
