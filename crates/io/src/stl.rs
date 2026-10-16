//! STL: element faces written as triangles, for mesh viewers, slicers and tools that read only
//! triangles.
//!
//! The layout follows the StereoLithography Interface Specification (3D Systems, 1988/1989).
//! Binary: an 80-byte header that does not start with `solid` (readers take that as ASCII), the
//! triangle count as a little-endian `u32`, then 50 bytes per triangle: the normal and the three
//! vertices as little-endian `f32` triples, and a `u16` attribute of 0. ASCII: `solid name`,
//! one `facet normal … outer loop … endloop endfacet` per triangle, `endsolid name`.
//!
//! Faces are triangulated through their corner nodes (a quadrilateral as 0-1-2 and 0-2-3), so the
//! vertices run counter-clockwise seen from outside when the faces do ([`ElementKind::faces`]).
//! Each facet's normal comes from its own vertices. Coordinates are rounded to single precision.
//!
//! [`ElementKind::faces`]: fieldcraft_model::ElementKind::faces

use fieldcraft_model::{ElementFace, Model};

use crate::{Error, Result};

/// The most triangles one file holds: two per face of a model at its element cap is far below.
pub const MAX_TRIANGLES: usize = 200_000_000;

const HEADER: &[u8] = b"FieldCraft STL";

type Triangle = [[f64; 3]; 3];

/// The triangles of `faces`, in their order (each quadrilateral as two).
pub fn triangles(model: &Model, faces: &[ElementFace]) -> Result<Vec<Triangle>> {
    let mut out = Vec::new();
    for &face in faces {
        let nodes = model.face_nodes(face).ok_or_else(|| Error::Invalid(format!("element {} has no face {}", face.element, face.face)))?;
        let corners = if nodes.len() >= 6 { nodes.len() / 2 } else { nodes.len() };
        let mut p = Vec::with_capacity(corners);
        for label in nodes.iter().take(corners) {
            let n = model.node(*label).ok_or_else(|| Error::Invalid(format!("element {} uses missing node {label}", face.element)))?;
            p.push(n.xyz);
        }
        match p.as_slice() {
            [a, b, c] => out.push([*a, *b, *c]),
            [a, b, c, d] => out.extend([[*a, *b, *c], [*a, *c, *d]]),
            _ => return Err(Error::Invalid(format!("face {} of element {} is not a triangle or a quadrilateral", face.face, face.element))),
        }
        if out.len() > MAX_TRIANGLES {
            return Err(Error::Limit(format!("an STL file here holds at most {MAX_TRIANGLES} triangles")));
        }
    }
    Ok(out)
}

/// The unit normal of a triangle from its vertices (right-hand rule); zero when degenerate.
fn normal(t: &Triangle) -> [f64; 3] {
    let [a, b, c] = t;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 && len.is_finite() { [n[0] / len, n[1] / len, n[2] / len] } else { [0.0; 3] }
}

/// `faces` as binary STL.
pub fn write_binary(model: &Model, faces: &[ElementFace]) -> Result<Vec<u8>> {
    let tris = triangles(model, faces)?;
    let count = u32::try_from(tris.len()).map_err(|_| Error::Limit(format!("binary STL holds at most {} triangles", u32::MAX)))?;
    let mut out = Vec::with_capacity(84 + 50 * tris.len());
    out.extend_from_slice(HEADER);
    out.resize(80, 0);
    out.extend_from_slice(&count.to_le_bytes());
    for t in &tris {
        for v in std::iter::once(normal(t)).chain(t.iter().copied()) {
            for x in v {
                out.extend_from_slice(&(x as f32).to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    Ok(out)
}

/// `faces` as ASCII STL, the solid called `name` (spaces and line breaks become `_`).
pub fn write_ascii(model: &Model, faces: &[ElementFace], name: &str) -> Result<String> {
    use std::fmt::Write;
    let tris = triangles(model, faces)?;
    let name: String = name.chars().take(80).map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).collect();
    let mut out = String::new();
    let _ = writeln!(out, "solid {name}");
    let xyz = |v: [f64; 3]| format!("{} {} {}", v[0] as f32, v[1] as f32, v[2] as f32);
    for t in &tris {
        let _ = writeln!(out, "  facet normal {}", xyz(normal(t)));
        out.push_str("    outer loop\n");
        for v in t {
            let _ = writeln!(out, "      vertex {}", xyz(*v));
        }
        out.push_str("    endloop\n  endfacet\n");
    }
    let _ = writeln!(out, "endsolid {name}");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fieldcraft_model::ElementKind;

    /// An `n`×`n`×`n` block of hex8 of side `size`, node labels from 1.
    fn block(n: u32, size: f64) -> Model {
        let mut m = Model::default();
        let id = |i: u32, j: u32, k: u32| 1 + i + (n + 1) * (j + (n + 1) * k);
        let h = size / f64::from(n);
        for k in 0..=n {
            for j in 0..=n {
                for i in 0..=n {
                    m.set_node(id(i, j, k), [f64::from(i) * h, f64::from(j) * h, f64::from(k) * h]).unwrap();
                }
            }
        }
        let mut label = 1;
        for k in 0..n {
            for j in 0..n {
                for i in 0..n {
                    let b = [id(i, j, k), id(i + 1, j, k), id(i + 1, j + 1, k), id(i, j + 1, k)];
                    let t = [id(i, j, k + 1), id(i + 1, j, k + 1), id(i + 1, j + 1, k + 1), id(i, j + 1, k + 1)];
                    m.set_element(label, ElementKind::Hex8, b.into_iter().chain(t).collect()).unwrap();
                    label += 1;
                }
            }
        }
        m
    }

    /// The triangles read back from binary STL: (normal, vertices).
    fn read(bytes: &[u8]) -> Vec<([f32; 3], [[f32; 3]; 3])> {
        let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), 84 + 50 * count);
        let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let v = |o: usize| [f(o), f(o + 4), f(o + 8)];
        (0..count)
            .map(|t| {
                let o = 84 + 50 * t;
                assert_eq!(&bytes[o + 48..o + 50], &[0, 0]);
                (v(o), [v(o + 12), v(o + 24), v(o + 36)])
            })
            .collect()
    }

    /// The volume a closed triangulated surface encloses (divergence theorem); positive when
    /// every normal points out.
    fn volume(tris: &[([f32; 3], [[f32; 3]; 3])]) -> f64 {
        tris.iter()
            .map(|(_, [a, b, c])| {
                let [a, b, c] = [a, b, c].map(|p| p.map(f64::from));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
            })
            .sum()
    }

    fn check_closed(m: &Model, expected_tris: usize, expected_volume: f64, centre: [f64; 3]) {
        let bytes = write_binary(m, &m.skin_faces()).unwrap();
        assert!(!bytes.starts_with(b"solid"));
        let tris = read(&bytes);
        assert_eq!(tris.len(), expected_tris);
        assert!((volume(&tris) - expected_volume).abs() < 1e-5, "{}", volume(&tris));
        for (n, t) in &tris {
            let fc: Vec<f64> = (0..3).map(|i| t.iter().map(|p| f64::from(p[i])).sum::<f64>() / 3.0 - centre[i]).collect();
            let dot: f64 = (0..3).map(|i| f64::from(n[i]) * fc[i]).sum();
            assert!(dot > 0.0, "normal {n:?} of {t:?} points in");
        }
        assert_eq!(bytes, write_binary(m, &m.skin_faces()).unwrap(), "deterministic");
    }

    #[test]
    fn a_hex_block_gives_two_triangles_per_boundary_quad_and_its_volume() {
        // 2×2×2 hex8 of side 2: 24 boundary quads.
        check_closed(&block(2, 2.0), 48, 8.0, [1.0; 3]);
    }

    #[test]
    fn tet10_and_wedge6_close_with_outward_normals() {
        let mut t = Model::default();
        let p = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [0.5, 0., 0.], [0.5, 0.5, 0.], [0., 0.5, 0.], [0., 0., 0.5], [0.5, 0., 0.5], [0., 0.5, 0.5]];
        for (i, xyz) in p.iter().enumerate() {
            t.set_node(i as u32 + 1, *xyz).unwrap();
        }
        t.set_element(1, ElementKind::Tet10, (1..=10).collect()).unwrap();
        check_closed(&t, 4, 1.0 / 6.0, [0.25; 3]);

        let mut w = Model::default();
        for (i, xyz) in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [0., 1., 1.]].iter().enumerate() {
            w.set_node(i as u32 + 1, *xyz).unwrap();
        }
        w.set_element(1, ElementKind::Wedge6, (1..=6).collect()).unwrap();
        // Two triangles and three quads: 8 facets, volume 1/2.
        check_closed(&w, 8, 0.5, [1.0 / 3.0, 1.0 / 3.0, 0.5]);
    }

    #[test]
    fn a_shell_plate_writes_one_pair_per_quad_along_its_normal() {
        let mut m = Model::default();
        for (i, xy) in [[0., 0.], [1., 0.], [2., 0.], [0., 1.], [1., 1.], [2., 1.]].iter().enumerate() {
            m.set_node(i as u32 + 1, [xy[0], xy[1], 0.0]).unwrap();
        }
        m.set_element(1, ElementKind::Quad4, vec![1, 2, 5, 4]).unwrap();
        m.set_element(2, ElementKind::Quad4, vec![2, 3, 6, 5]).unwrap();
        let tris = read(&write_binary(&m, &m.skin_faces()).unwrap());
        assert_eq!(tris.len(), 4);
        assert!(tris.iter().all(|(n, _)| *n == [0.0, 0.0, 1.0]));
    }

    #[test]
    fn ascii_holds_the_same_facets() {
        let m = block(1, 1.0);
        let text = write_ascii(&m, &m.skin_faces(), "my part").unwrap();
        assert!(text.starts_with("solid my_part\n") && text.ends_with("endsolid my_part\n"), "{text}");
        assert_eq!(text.matches("facet normal").count(), 12);
        assert_eq!(text.matches("vertex").count(), 36);
        assert!(text.contains("facet normal 0 0 -1"), "{text}");
    }

    #[test]
    fn faces_that_are_not_there_are_refused() {
        let m = block(1, 1.0);
        assert!(write_binary(&m, &[ElementFace { element: 9, face: 1 }]).is_err());
        assert!(write_binary(&m, &[ElementFace { element: 1, face: 7 }]).is_err());
        assert_eq!(write_binary(&m, &[]).unwrap().len(), 84);
    }
}
