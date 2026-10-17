//! Sphere-on-plate at size 4 (#547): the tet mesher's report and `mesh.quality` measure the same
//! final elements, tet4 and tet10 alike, and no needle is left below the skill's scaled Jacobian
//! limit although every dihedral angle is fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fieldcraft_engine::Session;
use fieldcraft_testkit::corpus;
use serde_json::json;

#[test]
fn sphere_on_plate_report_and_quality_agree() {
    let parts = corpus::parts().unwrap();
    let p = parts.iter().find(|p| p.name == "sphere-on-plate").unwrap();
    let dir = std::env::temp_dir().join(format!("fieldcraft-tet-metric-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.solidcraft");
    std::fs::write(&path, corpus::design_bytes(p).unwrap()).unwrap();
    for order in ["linear", "quadratic"] {
        let mut s = Session::default();
        s.execute("geometry.open", &json!({"path": path})).unwrap();
        let r = s.execute("mesh.tet.generate", &json!({"body": "Plate", "size": 4.0, "order": order})).unwrap();
        let mesh = &r["bodies"][0]["quality"]["mesh"];
        let q = s.execute("mesh.quality", &json!({})).unwrap();
        let m = &q["metrics"];
        let f = |v: &serde_json::Value| v.as_f64().unwrap();
        assert_eq!(f(&mesh["min_dihedral"]), f(&m["min_dihedral"]["min"]), "{order}");
        assert_eq!(f(&mesh["max_dihedral"]), f(&m["max_dihedral"]["max"]), "{order}");
        assert_eq!(f(&mesh["min_scaled_jacobian"]), f(&m["scaled_jacobian"]["min"]), "{order}");
        assert!(f(&mesh["min_dihedral"]) > 30.0 && f(&mesh["max_dihedral"]) < 145.0, "{order}: {mesh}");
        assert!(f(&mesh["min_scaled_jacobian"]) >= 0.3, "{order}: {mesh}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
