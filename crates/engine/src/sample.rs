//! The sample model the desktop app opens with `--sample`: a script of ordinary commands, so it
//! builds the same way a user's script does.
//!
//! Two parts side by side: a 40 × 20 × 10 block of 5 mm hex8 cells, and a 15 mm cube split into
//! 162 tet4 elements (each 5 mm cell into six tetrahedra around its main diagonal, the Kuhn
//! subdivision, which is conforming between neighbouring cells). Every element is positively
//! oriented.

use serde_json::{Value, json};

/// Hex block: cells along x, y, z and the cell size (mm).
const HEX_CELLS: [u32; 3] = [8, 4, 2];
/// Tet cube: cells per side and its offset from the origin (mm).
const TET_CELLS: u32 = 3;
const TET_ORIGIN: [f64; 3] = [50.0, 2.5, 0.0];
const CELL: f64 = 5.0;

/// The script: `mesh.node.add` and `mesh.element.add` steps.
pub fn script() -> Value {
    let mut steps = Vec::new();
    let mut node = 0u32;
    let mut element = 0u32;
    // Hex block.
    let [nx, ny, nz] = HEX_CELLS;
    let first = node + 1;
    let hid = |i: u32, j: u32, k: u32| first + i + j * (nx + 1) + k * (nx + 1) * (ny + 1);
    for k in 0..=nz {
        for j in 0..=ny {
            for i in 0..=nx {
                node += 1;
                steps.push(json!({"command": "mesh.node.add", "params": {"label": node, "xyz": [f64::from(i) * CELL, f64::from(j) * CELL, f64::from(k) * CELL]}}));
            }
        }
    }
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                element += 1;
                let nodes = [
                    hid(i, j, k),
                    hid(i + 1, j, k),
                    hid(i + 1, j + 1, k),
                    hid(i, j + 1, k),
                    hid(i, j, k + 1),
                    hid(i + 1, j, k + 1),
                    hid(i + 1, j + 1, k + 1),
                    hid(i, j + 1, k + 1),
                ];
                steps.push(json!({"command": "mesh.element.add", "params": {"label": element, "kind": "hex8", "nodes": nodes}}));
            }
        }
    }
    // Tet cube.
    let n = TET_CELLS;
    let first = node + 1;
    let tid = |i: u32, j: u32, k: u32| first + i + j * (n + 1) + k * (n + 1) * (n + 1);
    let at = |i: u32, j: u32, k: u32| [TET_ORIGIN[0] + f64::from(i) * CELL, TET_ORIGIN[1] + f64::from(j) * CELL, TET_ORIGIN[2] + f64::from(k) * CELL];
    for k in 0..=n {
        for j in 0..=n {
            for i in 0..=n {
                node += 1;
                steps.push(json!({"command": "mesh.node.add", "params": {"label": node, "xyz": at(i, j, k)}}));
            }
        }
    }
    // The six axis orders: each walks from the cell's corner (0,0,0) to (1,1,1) one axis at a time.
    const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for k in 0..n {
        for j in 0..n {
            for i in 0..n {
                for order in ORDERS {
                    let mut c = [0u32; 3];
                    let mut corners = vec![c];
                    for axis in order {
                        if let Some(x) = c.get_mut(axis) {
                            *x = 1;
                        }
                        corners.push(c);
                    }
                    let mut nodes: Vec<u32> = corners.iter().map(|c| tid(i + c[0], j + c[1], k + c[2])).collect();
                    let p: Vec<[f64; 3]> = corners.iter().map(|c| at(i + c[0], j + c[1], k + c[2])).collect();
                    if signed_volume(&p) < 0.0 {
                        nodes.swap(1, 2);
                    }
                    element += 1;
                    steps.push(json!({"command": "mesh.element.add", "params": {"label": element, "kind": "tet4", "nodes": nodes}}));
                }
            }
        }
    }
    Value::Array(steps)
}

/// Six times the signed volume of the tetrahedron `p[0..4]` (positive when p3 lies on the side
/// p0 → p1 → p2 turns counter-clockwise around).
fn signed_volume(p: &[[f64; 3]]) -> f64 {
    let (Some(a), Some(b), Some(c), Some(d)) = (p.first(), p.get(1), p.get(2), p.get(3)) else { return 0.0 };
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let w = [d[0] - a[0], d[1] - a[1], d[2] - a[2]];
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn the_sample_builds_positive_elements_that_fill_both_parts() {
        let mut s = Session::default();
        s.run_script(&script()).unwrap();
        let info = s.execute("model.info", &json!({})).unwrap();
        assert_eq!(info["elements"]["hex8"], 64);
        assert_eq!(info["elements"]["tet4"], 162);
        assert_eq!(info["nodes"], 9 * 5 * 3 + 4 * 4 * 4);
        // Tet volumes are positive and add up to the cube's.
        let mut volume = 0.0;
        for e in s.model.elements().values().filter(|e| e.kind == modelcraft_model::ElementKind::Tet4) {
            let p: Vec<[f64; 3]> = e.nodes.iter().map(|n| s.model.node(*n).unwrap().xyz).collect();
            let v = signed_volume(&p) / 6.0;
            assert!(v > 0.0);
            volume += v;
        }
        assert!((volume - 15.0f64.powi(3)).abs() < 1e-9, "{volume}");
    }
}
