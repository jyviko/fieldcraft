//! Embeds the toolbar icons: `assets/icons/*.svg` become `ICONS`, a sorted `(id, svg)` table
//! (`src/icons.rs`).

use std::fmt::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let dir = manifest.join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut ids: Vec<String> =
        std::fs::read_dir(&dir)?.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".svg")).map(str::to_string)).collect();
    ids.sort();
    let mut out = String::from("/// Every toolbar icon by catalogue id, sorted by id.\npub static ICONS: &[(&str, &str)] = &[\n");
    for id in &ids {
        let path = dir.join(format!("{id}.svg"));
        println!("cargo:rerun-if-changed={}", path.display());
        let _ = writeln!(out, "    ({id:?}, include_str!({:?})),", path.display().to_string());
    }
    out.push_str("];\n");
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR")?).join("icons.rs"), out)?;
    Ok(())
}
