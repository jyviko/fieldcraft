use super::*;

/// The plate (40 × 30 × 10) with its Ø10 hole round (15, 15): a point on the hole's wall.
const ON_HOLE: [f64; 3] = [20.0, 15.0, 5.0];
const HOLE: ([f64; 2], f64) = ([15.0, 15.0], 5.0);

fn plate() -> Session {
    super::super::datum::tests::plate().0
}

fn hole_radius(p: [f64; 3]) -> f64 {
    ((p[0] - HOLE.0[0]).powi(2) + (p[1] - HOLE.0[1]).powi(2)).sqrt()
}

/// Which analytic face a node of the plate lies on, if any.
fn on_faces(p: [f64; 3]) -> Vec<&'static str> {
    let mut f = Vec::new();
    if (hole_radius(p) - HOLE.1).abs() < 1e-6 {
        f.push("hole");
    }
    if p[2].abs() < 1e-6 || (p[2] - 10.0).abs() < 1e-6 {
        f.push("cap");
    }
    f
}

/// Spikes near the hole: interior nodes pushed about; nodes on the hole turned round it and
/// moved along its axis (still on the cylinder), nodes on the caps moved in their plane.
fn perturb(s: &mut Session, centre: [f64; 3], radius: f64, amount: f64) {
    let nodes: Vec<(u32, [f64; 3])> = s.model.nodes().iter().map(|(n, p)| (*n, p.xyz)).collect();
    for (k, (n, p)) in nodes.into_iter().enumerate() {
        let d2: f64 = (0..3).map(|i| (p[i] - centre[i]).powi(2)).sum();
        if d2 > radius * radius {
            continue;
        }
        let wiggle = |m: usize| amount * ((k * m % 7) as f64 - 3.0) / 3.0;
        let faces = on_faces(p);
        let q = match faces.as_slice() {
            [] => [p[0] + wiggle(3), p[1] + wiggle(5), p[2] + wiggle(11)],
            ["hole"] => {
                let a = wiggle(3) / HOLE.1;
                let (x, y) = (p[0] - HOLE.0[0], p[1] - HOLE.0[1]);
                [HOLE.0[0] + x * a.cos() - y * a.sin(), HOLE.0[1] + x * a.sin() + y * a.cos(), p[2] + wiggle(5)]
            }
            ["cap"] if hole_radius(p) > HOLE.1 + 1e-6 => [p[0] + wiggle(3), p[1] + wiggle(5), p[2]],
            _ => continue,
        };
        s.model.set_node(n, q).unwrap();
    }
}

fn check_on_geometry(before: &Model, after: &Model) {
    for (n, p) in after.nodes() {
        let old = before.node(*n).unwrap().xyz;
        for f in on_faces(old) {
            match f {
                "hole" => assert!((hole_radius(p.xyz) - HOLE.1).abs() < 1e-6, "node {n} left the hole: r = {}", hole_radius(p.xyz)),
                _ => assert!((p.xyz[2] - old[2]).abs() < 1e-6, "node {n} left its cap"),
            }
        }
    }
}

fn invalid(s: &mut Session) -> u64 {
    s.execute("mesh.quality", &json!({})).unwrap()["invalid"]["count"].as_u64().unwrap_or(0)
}

/// The issue's hex test on a design: spikes near the hole relax, every surface node stays on its
/// face (the hole's cylinder exactly), quality rises, nothing inverts, nothing outside the sphere
/// moves; one undo step; the field is kept and saved with the model.
#[test]
fn hex_spikes_near_a_hole_relax_on_the_geometry() {
    let mut s = plate();
    s.execute("mesh.hex.generate", &json!({"size": 2.0})).unwrap();
    perturb(&mut s, ON_HOLE, 3.0, 0.35);
    let perturbed = s.model.clone();
    let r = s.execute("mesh.node.relax", &json!({"center": ON_HOLE, "radius": 4.0, "iterations": 10})).unwrap();
    let (before, after) = (r["before"]["min_scaled_jacobian"].as_f64().unwrap(), r["after"]["min_scaled_jacobian"].as_f64().unwrap());
    assert!(after > before + 0.05, "{r}");
    assert_eq!(r["after"]["invalid"], 0, "{r}");
    assert!(r["moved"].as_u64().unwrap() > 0 && r["field"] == "Relax1", "{r}");
    check_on_geometry(&perturbed, &s.model);
    for (n, p) in s.model.nodes() {
        let old = perturbed.node(*n).unwrap().xyz;
        if old != p.xyz {
            let d2: f64 = (0..3).map(|i| (old[i] - ON_HOLE[i]).powi(2)).sum();
            assert!(d2 < 16.0, "node {n} outside the sphere moved");
        }
    }
    assert_eq!(invalid(&mut s), 0);
    // The field is saved with the model.
    let text = serde_json::to_string(&s.model).unwrap();
    let back: Model = serde_json::from_str(&text).unwrap();
    assert_eq!(back.relaxation("Relax1"), s.model.relaxation("Relax1"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.model, perturbed);
}

/// The same on tet10 with curved mid-side nodes on the hole: they stay on the cylinder.
#[test]
fn tet10_relaxes_with_curved_mid_side_nodes_on_the_hole() {
    let mut s = plate();
    s.execute("mesh.tet.generate", &json!({"size": 2.5, "order": "quadratic"})).unwrap();
    perturb(&mut s, ON_HOLE, 3.0, 0.2);
    // Put the mid-side nodes back on their edges' middles, on the hole where the edge is.
    let mids: Vec<(u32, u32, u32)> = s
        .model
        .elements()
        .values()
        .flat_map(|e| e.kind.edges().iter().map(|ed| (e.nodes[ed[0]], e.nodes[ed[1]], e.nodes[ed[2]])).collect::<Vec<_>>())
        .collect();
    let original = s.model.clone();
    for (a, b, m) in mids {
        let (pa, pb, pm) = (original.node(a).unwrap().xyz, original.node(b).unwrap().xyz, original.node(m).unwrap().xyz);
        let mut q = [(pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0, (pa[2] + pb[2]) / 2.0];
        if on_faces(pm).contains(&"hole") {
            let r = hole_radius(q);
            q = [HOLE.0[0] + (q[0] - HOLE.0[0]) * HOLE.1 / r, HOLE.0[1] + (q[1] - HOLE.0[1]) * HOLE.1 / r, q[2]];
        }
        s.model.set_node(m, q).unwrap();
    }
    let perturbed = s.model.clone();
    let r = s.execute("mesh.node.relax", &json!({"center": ON_HOLE, "radius": 4.0, "iterations": 8})).unwrap();
    assert!(r["after"]["min_scaled_jacobian"].as_f64().unwrap() >= r["before"]["min_scaled_jacobian"].as_f64().unwrap(), "{r}");
    assert_eq!(r["after"]["invalid"], 0, "{r}");
    check_on_geometry(&perturbed, &s.model);
}

/// A partition cut is not geometry: nodes on it move freely, shared by the cells on both sides.
#[test]
fn nodes_on_a_partition_cut_move_freely() {
    let mut s = plate();
    s.execute("geometry.datum", &json!({"name": "Cut", "by": {"type": "offset", "plane": "yz", "offset": 27}})).unwrap();
    s.execute("geometry.partition", &json!({"name": "P", "datum": "Cut"})).unwrap();
    s.execute("mesh.cells", &json!({"size": 2.0, "default": "hex"})).unwrap();
    let centre = [27.0, 15.0, 5.0];
    perturb(&mut s, centre, 2.5, 0.3);
    let before = s.model.clone();
    s.execute("mesh.node.relax", &json!({"center": centre, "radius": 3.5, "iterations": 10})).unwrap();
    let moved_off = s
        .model
        .nodes()
        .iter()
        .filter(|(n, p)| {
            let old = before.node(**n).unwrap().xyz;
            (old[0] - 27.0).abs() < 1e-6 && on_faces(old).is_empty() && (p.xyz[0] - 27.0).abs() > 1e-6
        })
        .count();
    assert!(moved_off > 0, "nodes on the cut are free");
    assert_eq!(invalid(&mut s), 0);
}

/// A field marked `reapply` is applied again after a remesh; `mesh.node.relax.apply` applies it
/// on demand; around the worst element works.
#[test]
fn a_kept_field_is_applied_again_after_a_remesh() {
    let mut s = plate();
    s.execute("mesh.hex.generate", &json!({"size": 3.0})).unwrap();
    perturb(&mut s, ON_HOLE, 4.0, 0.3);
    let r = s.execute("mesh.node.relax", &json!({"center": ON_HOLE, "radius": 5.0, "field": "Spike", "reapply": true})).unwrap();
    assert_eq!(r["field"], "Spike");
    let r = s.execute("mesh.hex.generate", &json!({"size": 1.5})).unwrap();
    assert_eq!(r["relaxations_applied"][0]["field"], "Spike", "{r}");
    assert_eq!(invalid(&mut s), 0);
    let r = s.execute("mesh.node.relax.apply", &json!({"field": "Spike"})).unwrap();
    assert!(r["summary"].as_str().unwrap().contains("Spike"));
    let worst = s.execute("mesh.quality", &json!({})).unwrap()["metrics"]["scaled_jacobian"]["worst"].as_u64().unwrap();
    let r = s.execute("mesh.node.relax", &json!({"around_element": worst})).unwrap();
    assert!(r["after"]["min_scaled_jacobian"].as_f64() >= r["before"]["min_scaled_jacobian"].as_f64(), "{r}");
}

#[test]
fn hostile_parameters_never_panic() {
    let mut s = plate();
    s.execute("mesh.hex.generate", &json!({"size": 4.0})).unwrap();
    for p in [
        json!({}),
        json!({"center": [0, 0, 0]}),
        json!({"center": [1e300, 0, 0], "radius": 1}),
        json!({"center": [0, 0, 0], "radius": -1}),
        json!({"center": [0, 0, 0], "radius": 1, "nodes": [1]}),
        json!({"nodes": [1], "strength": 2}),
        json!({"nodes": [1], "iterations": 0}),
        json!({"nodes": [1], "iterations": 1000}),
        json!({"nodes": [1], "falloff": "cubic"}),
        json!({"around_element": 99999999}),
        json!({"set": "nope"}),
        json!({"nodes": [1], "bodies": ["nope"]}),
    ] {
        assert!(s.execute("mesh.node.relax", &p).is_err(), "{p}");
    }
    assert!(s.execute("mesh.node.relax.apply", &json!({"field": "nope"})).is_err());
    assert!(s.execute("mesh.node.relax.apply", &json!({})).is_err());
}
