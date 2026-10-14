//! `fieldcraft-cli --version`, which the release packaging runs to check each build.

use std::process::Command;

#[test]
fn version_prints_the_workspace_version() {
    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_fieldcraft-cli")).arg(flag).output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("fieldcraft-cli {}", env!("CARGO_PKG_VERSION")));
    }
}
