//! Windows builds embed the application icon (assets/app-icon/icon.ico) in `fieldcraft.exe`, so
//! Explorer, the taskbar and shortcuts show it. Other targets need nothing here.
#![forbid(unsafe_code)]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let icon = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../assets/app-icon/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let rc = out.join("fieldcraft.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", icon.display().to_string().replace('\\', "/")))?;
    // Missing resource tools are a build error, rather than silently shipping a generic icon.
    embed_resource::compile_for(&rc, ["fieldcraft"], embed_resource::NONE).manifest_required()?;
    Ok(())
}
