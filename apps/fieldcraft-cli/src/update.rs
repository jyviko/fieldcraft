//! `fieldcraft-cli update --check | --install` (docs/updates.md): the same check as the app's
//! Help › Check for Updates (`app.update.check`), and installing the latest release over this
//! copy of the CLI's install, verified against the release key first. `FIELDCRAFT_UPDATE_HOST`
//! points both at a local test server.

use fieldcraft_engine::Session;
use fieldcraft_update::install::{self, Finish};
use fieldcraft_update::net::Client;
use fieldcraft_update::stage::{self, Options};
use serde_json::json;

pub const USAGE: &str = "usage: fieldcraft-cli update --check | --install";

pub fn run(args: &[String]) -> Result<(), String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--check"] => check(),
        ["--install"] => install(),
        _ => Err(USAGE.into()),
    }
}

fn check() -> Result<(), String> {
    fieldcraft_engine::enable_update_checks(true);
    let report = Session::default().execute("app.update.check", &json!({})).map_err(|e| e.to_string())?;
    println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    Ok(())
}

fn install() -> Result<(), String> {
    let client = Client::from_env();
    let check = client.check().map_err(|e| e.to_string())?;
    if !check.newer() {
        println!("FieldCraft {} is up to date (the latest release is {}).", check.current, check.latest.version);
        return Ok(());
    }
    let here = install::here();
    let downloads = fieldcraft_io::recovery::data_dir().map(|d| d.join("updates")).unwrap_or_else(|| std::env::temp_dir().join("fieldcraft-updates"));
    eprintln!("Downloading FieldCraft {} for {}…", check.latest.version, here.describe());
    let progress = |done: u64, total: u64| {
        if total > 0 && done == total {
            eprintln!("Downloaded {} MB; verifying…", total / (1024 * 1024));
        }
    };
    let options = Options { key: fieldcraft_update::PUBLIC_KEY, downloads: &downloads, arch: std::env::consts::ARCH, progress: &progress };
    let staged = stage::stage(&client, &check.latest, &here, &options).map_err(|e| e.to_string())?;
    match install::plan(&staged, false).map_err(|e| e.to_string())? {
        Finish::Swap(plan) => {
            stage::apply(&plan).map_err(|e| e.to_string())?;
            println!("Installed FieldCraft {} ({}). A running FieldCraft app uses it from its next start.", staged.version, here.describe());
        }
        Finish::Package { file, command } => {
            println!("Downloaded and verified {}.\nInstall it with the system's package manager:\n  {command}", file.display());
        }
    }
    Ok(())
}
