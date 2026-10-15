//! Downloading, verifying and unpacking a release beside the install ([`stage`]), and swapping it
//! in ([`apply`]).
//!
//! Nothing is unpacked before the checksum list's signature and the asset's checksum are
//! checked; a staged macOS app must also pass `codesign --verify --deep --strict`, carry the
//! FieldCraft bundle id, and pass Gatekeeper (`spctl`) whenever the installed app does. Archives
//! are unpacked with every path kept inside the staging folder and their sizes capped.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::install::{Install, Plan, STAGE_PREFIX, Staged};
use crate::net::{Client, MAX_ASSET};
use crate::verify::{self, MAX_SIGNATURE, MAX_SUMS};
use crate::{Error, Release, Result, SIGNATURE, SUMS};

/// The most files an archive may hold.
const MAX_ENTRIES: usize = 20_000;
/// The most an archive may unpack to (bytes).
const MAX_UNPACKED: u64 = 4 * 1024 * 1024 * 1024;

/// What staging needs besides the release and the install.
pub struct Options<'a> {
    /// The release key's public half (minisign base64): [`crate::PUBLIC_KEY`], or a test key.
    pub key: &'a str,
    /// Where a system package (`.deb`, `.rpm`) is downloaded to.
    pub downloads: &'a Path,
    /// `std::env::consts::ARCH`.
    pub arch: &'a str,
    /// Hears the bytes downloaded so far and the total (0 when not known).
    pub progress: &'a dyn Fn(u64, u64),
}

fn io(what: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Install(format!("{}: {e}", what.display()))
}

/// Download `release`'s asset for `install`, verify it and unpack it into a staging folder.
pub fn stage(client: &Client, release: &Release, install: &Install, o: &Options) -> Result<Staged> {
    if let Some(why) = install.unsupported() {
        return Err(Error::NotSupported(why));
    }
    let v = &release.version;
    let name =
        install.asset_name(v, o.arch).ok_or_else(|| Error::NotSupported(format!("there is no release build for this computer ({})", o.arch)))?;
    let (Some(sums), Some(sig)) = (release.asset(SUMS), release.asset(SIGNATURE)) else {
        return Err(Error::Verify(format!("FieldCraft {v} is not signed, so it cannot be installed from here; download it from {}", release.url)));
    };
    let asset = release.asset(&name).ok_or_else(|| Error::Release(format!("FieldCraft {v} has no {name}")))?;
    // The list and its signature first: nothing else is downloaded unless they check out.
    let sums_bytes = client.small(&sums.url, MAX_SUMS)?;
    let sig = String::from_utf8(client.small(&sig.url, MAX_SIGNATURE)?).map_err(|_| Error::Verify("SHA256SUMS.txt.minisig is not text".into()))?;
    verify::signature(&sums_bytes, &sig, o.key)?;
    let sums = verify::parse_sums(std::str::from_utf8(&sums_bytes).map_err(|_| Error::Verify("SHA256SUMS.txt is not text".into()))?)?;
    if !sums.contains_key(&name) {
        return Err(Error::Verify(format!("SHA256SUMS.txt does not list {name}")));
    }

    let dir = install.stage_dir(v, o.downloads).ok_or_else(|| Error::Install(format!("no folder to stage {name} in")))?;
    let package = matches!(install, Install::Deb | Install::Rpm);
    if !package && dir.exists() {
        fs::remove_dir_all(&dir).map_err(io(&dir))?;
    }
    fs::create_dir_all(&dir).map_err(|e| {
        Error::Install(format!(
            "cannot write to {}: {e}. Install FieldCraft {v} from {} instead, or give your account write access there",
            dir.parent().unwrap_or(&dir).display(),
            release.url
        ))
    })?;
    let staged = (|| {
        let file = dir.join(&name);
        let sha = client.download(&asset.url, &file, MAX_ASSET, o.progress)?;
        if let Err(e) = verify::checksum(&sums, &name, &sha) {
            let _ = fs::remove_file(&file);
            return Err(e);
        }
        let (payload, entries) = unpack(install, &file, &dir)?;
        Ok(Staged { install: install.clone(), version: v.clone(), dir: dir.clone(), payload, entries })
    })();
    if staged.is_err() && !package {
        let _ = fs::remove_dir_all(&dir);
    }
    staged
}

/// Unpack the verified download `file` in `dir`: the new app, file or folder, and (for a
/// Windows folder) the names it holds.
fn unpack(install: &Install, file: &Path, dir: &Path) -> Result<(PathBuf, Vec<String>)> {
    match install {
        Install::MacApp { bundle } => {
            let app = mount_and_copy_app(file, dir)?;
            let _ = fs::remove_file(file);
            check_mac_app(&app, bundle)?;
            Ok((app, Vec::new()))
        }
        Install::MacCli { .. } => {
            let out = dir.join("unpacked");
            unzip(file, &out)?;
            let _ = fs::remove_file(file);
            let exe = find(&out, &|p| p.file_name().is_some_and(|n| n == "fieldcraft-cli") && p.is_file())
                .ok_or_else(|| Error::Install("the download has no fieldcraft-cli".into()))?;
            executable(&exe)?;
            codesign_verify(&exe)?;
            Ok((exe, Vec::new()))
        }
        Install::Windows { .. } => {
            let out = dir.join("new");
            unzip(file, &out)?;
            let _ = fs::remove_file(file);
            let folder =
                find(&out, &|p| p.join("fieldcraft.exe").is_file()).ok_or_else(|| Error::Install("the download has no fieldcraft.exe".into()))?;
            let mut entries: Vec<String> =
                fs::read_dir(&folder).map_err(io(&folder))?.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect();
            entries.sort();
            Ok((folder, entries))
        }
        Install::TarGz { .. } => {
            let out = dir.join("new");
            untar_gz(file, &out)?;
            let _ = fs::remove_file(file);
            let root =
                find(&out, &|p| p.join("bin/fieldcraft").is_file()).ok_or_else(|| Error::Install("the download has no bin/fieldcraft".into()))?;
            Ok((root, Vec::new()))
        }
        Install::AppImage { .. } => {
            executable(file)?;
            Ok((file.to_path_buf(), Vec::new()))
        }
        Install::Deb | Install::Rpm => Ok((file.to_path_buf(), Vec::new())),
        Install::Unmanaged(why) => Err(Error::NotSupported(why.clone())),
    }
}

/// `dir` itself or the first folder below it (two levels) that `is` accepts.
fn find(dir: &Path, is: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if is(dir) {
        return Some(dir.to_path_buf());
    }
    let mut children: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    children.sort();
    children.iter().find(|c| is(c)).cloned().or_else(|| children.iter().filter(|c| c.is_dir()).find_map(|c| find_shallow(c, is)))
}

fn find_shallow(dir: &Path, is: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut children: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    children.sort();
    children.into_iter().find(|c| is(c))
}

/// Mark `file` executable (Unix).
fn executable(file: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).map_err(io(file))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

/// Unpack zip `file` into `out`, every path kept inside it.
fn unzip(file: &Path, out: &Path) -> Result<()> {
    let bad = |e: zip::result::ZipError| Error::Install(format!("{}: {e}", file.display()));
    let mut archive = zip::ZipArchive::new(fs::File::open(file).map_err(io(file))?).map_err(bad)?;
    if archive.len() > MAX_ENTRIES {
        return Err(Error::Install(format!("{} holds more than {MAX_ENTRIES} files", file.display())));
    }
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(bad)?;
        let rel = entry.enclosed_name().ok_or_else(|| Error::Install(format!("{} holds a path outside its folder", file.display())))?;
        let path = out.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&path).map_err(io(&path))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut f = fs::File::create(&path).map_err(io(&path))?;
        let left = MAX_UNPACKED.saturating_sub(total);
        let n = std::io::copy(&mut (&mut entry).take(left + 1), &mut f).map_err(io(&path))?;
        total += n;
        if total > MAX_UNPACKED {
            return Err(Error::Install(format!("{} unpacks to more than {MAX_UNPACKED} bytes", file.display())));
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777)).map_err(io(&path))?;
        }
    }
    Ok(())
}

/// Unpack tar.gz `file` into `out`, every path kept inside it.
fn untar_gz(file: &Path, out: &Path) -> Result<()> {
    fs::create_dir_all(out).map_err(io(out))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(file).map_err(io(file))?));
    archive.set_preserve_permissions(true);
    let mut total = 0u64;
    for (i, entry) in archive.entries().map_err(io(file))?.enumerate() {
        let mut entry = entry.map_err(io(file))?;
        total = total.saturating_add(entry.header().size().unwrap_or(0));
        if i >= MAX_ENTRIES || total > MAX_UNPACKED {
            return Err(Error::Install(format!("{} holds more than {MAX_ENTRIES} files or {MAX_UNPACKED} bytes", file.display())));
        }
        if !entry.unpack_in(out).map_err(io(file))? {
            return Err(Error::Install(format!("{} holds a path outside its folder", file.display())));
        }
    }
    Ok(())
}

/// Run `program` with `args`; its failure, with what it printed, as an error.
fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<std::process::Output> {
    let out = Command::new(program).args(args).output().map_err(|e| Error::Install(format!("{program}: {e}")))?;
    if out.status.success() { Ok(out) } else { Err(Error::Install(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))) }
}

/// Copy `FieldCraft.app` out of disk image `dmg` into `dir`.
fn mount_and_copy_app(dmg: &Path, dir: &Path) -> Result<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Err(Error::NotSupported("a disk image is opened on macOS only".into()));
    }
    let mnt = dir.join("mnt");
    fs::create_dir_all(&mnt).map_err(io(&mnt))?;
    run(
        "hdiutil",
        &[
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-readonly".as_ref(),
            "-noautoopen".as_ref(),
            "-mountpoint".as_ref(),
            mnt.as_os_str(),
            dmg.as_os_str(),
        ],
    )?;
    let app = dir.join("FieldCraft.app");
    let copied = run("ditto", &[mnt.join("FieldCraft.app").as_os_str(), app.as_os_str()]);
    if run("hdiutil", &["detach".as_ref(), mnt.as_os_str()]).is_err() {
        let _ = run("hdiutil", &["detach".as_ref(), "-force".as_ref(), mnt.as_os_str()]);
    }
    copied?;
    let _ = fs::remove_dir(&mnt);
    Ok(app)
}

/// `codesign --verify --strict` (macOS; nothing elsewhere).
fn codesign_verify(path: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        run("codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), path.as_os_str()])
            .map_err(|e| Error::Verify(format!("the downloaded program's code signature is not valid: {e}")))?;
    }
    Ok(())
}

/// A staged macOS app must be validly signed, be FieldCraft (its bundle id), and pass Gatekeeper
/// whenever the installed app does (a notarized install is never replaced by one that is not).
fn check_mac_app(app: &Path, installed: &Path) -> Result<()> {
    codesign_verify(app)?;
    let info = run("codesign", &["-d".as_ref(), "--verbose=2".as_ref(), app.as_os_str()])?;
    let text = String::from_utf8_lossy(&info.stderr);
    if !text.lines().any(|l| l.trim() == format!("Identifier={}", crate::install::BUNDLE_ID)) {
        return Err(Error::Verify(format!("the downloaded app is not {}", crate::install::BUNDLE_ID)));
    }
    let gatekeeper = |p: &Path| run("spctl", &["--assess".as_ref(), "--type".as_ref(), "execute".as_ref(), p.as_os_str()]);
    if gatekeeper(installed).is_ok() {
        gatekeeper(app).map_err(|e| Error::Verify(format!("Gatekeeper does not accept the downloaded app: {e}")))?;
    }
    Ok(())
}

/// Swap the staged release in: each item moves to its backup and the new one into its place; a
/// failure puts back what was moved. Then the new version starts (when the plan says so) and
/// the staging folder is removed (on Windows the old, still running program stays in it until
/// [`clean_stale`] at the next start).
pub fn apply(plan: &Plan) -> Result<()> {
    let mut done: Vec<(&crate::install::Replace, bool)> = Vec::new();
    let undo = |done: &[(&crate::install::Replace, bool)]| {
        for (r, had) in done.iter().rev() {
            let _ = fs::rename(&r.target, &r.new);
            if *had {
                let _ = fs::rename(&r.backup, &r.target);
            }
        }
    };
    for r in &plan.replace {
        let had = fs::symlink_metadata(&r.target).is_ok();
        if had {
            if let Some(parent) = r.backup.parent() {
                fs::create_dir_all(parent).map_err(io(parent))?;
            }
            if let Err(e) = fs::rename(&r.target, &r.backup) {
                undo(&done);
                return Err(Error::Install(format!("{}: {e}", r.target.display())));
            }
        }
        if let Err(e) = fs::rename(&r.new, &r.target) {
            if had {
                let _ = fs::rename(&r.backup, &r.target);
            }
            undo(&done);
            return Err(Error::Install(format!("{}: {e}", r.target.display())));
        }
        done.push((r, had));
    }
    if let Some((program, args)) = &plan.launch
        && let Err(e) = Command::new(program).args(args).spawn()
    {
        log::warn!("update: installed, but {} did not start: {e}", program.display());
    }
    let _ = fs::remove_dir_all(&plan.cleanup);
    Ok(())
}

/// Remove staging folders an earlier update left beside `install` (on Windows, the old program
/// could not be deleted while it ran).
pub fn clean_stale(install: &Install) {
    let Some(parent) = install.target().and_then(Path::parent) else { return };
    let Ok(entries) = fs::read_dir(parent) else { return };
    for e in entries.flatten() {
        if e.file_name().to_str().is_some_and(|n| n.starts_with(STAGE_PREFIX)) && e.path().is_dir() {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;

    use super::*;
    use crate::install::{Finish, plan};
    use crate::mock::Server;
    use crate::verify::tests::{keypair, sign};

    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Temp {
            let d = std::env::temp_dir().join(format!("fieldcraft-update-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            Temp(d)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A tar.gz of the Linux layout, `fieldcraft-9.0.0-linux-x86_64/bin/fieldcraft` saying `text`.
    fn linux_tar(text: &str) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut t = tar::Builder::new(&mut gz);
            for (path, body) in [
                ("fieldcraft-9.0.0-linux-x86_64/bin/fieldcraft", text),
                ("fieldcraft-9.0.0-linux-x86_64/share/doc/fieldcraft/THIRD-PARTY-LICENSES.txt", "licences"),
            ] {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o755);
                h.set_cksum();
                t.append_data(&mut h, path, body.as_bytes()).unwrap();
            }
            t.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    /// A zip of the Windows layout.
    fn windows_zip(text: &str) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["fieldcraft.exe", "fieldcraft-cli.exe", "THIRD-PARTY-LICENSES.txt"] {
            z.start_file(format!("fieldcraft-9.0.0-windows-x64/{name}"), zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// Publish release 9.0.0 on `server` with `assets`, its checksum list (with `tamper` changing
    /// it after hashing) signed by `sk`.
    fn publish(server: &Server, sk: Option<&minisign::SecretKey>, assets: &[(&str, Vec<u8>)], tamper: bool) -> Release {
        let mut sums: String = assets.iter().map(|(n, b)| format!("{}  {n}\n", verify::sha256_hex(b))).collect();
        if tamper {
            sums = assets.iter().map(|(n, _)| format!("{}  {n}\n", verify::sha256_hex(b"something else"))).collect();
        }
        let mut list = Vec::new();
        for (name, body) in assets {
            server.route(&format!("/dl/{name}"), 200, body.clone());
            list.push(json!({"name": name, "browser_download_url": format!("{}/dl/{name}", server.url()), "size": body.len()}));
        }
        server.route("/dl/SHA256SUMS.txt", 200, sums.clone().into_bytes());
        list.push(json!({"name": "SHA256SUMS.txt", "browser_download_url": format!("{}/dl/SHA256SUMS.txt", server.url())}));
        if let Some(sk) = sk {
            server.route("/dl/SHA256SUMS.txt.minisig", 200, sign(sk, sums.as_bytes()).into_bytes());
            list.push(json!({"name": "SHA256SUMS.txt.minisig", "browser_download_url": format!("{}/dl/SHA256SUMS.txt.minisig", server.url())}));
        }
        let r = json!({"tag_name": "v9.0.0", "html_url": "https://example.invalid/v9.0.0", "assets": list});
        server.release("latest", &r);
        Release::from_github(&r).unwrap()
    }

    fn options<'a>(key: &'a str, downloads: &'a Path) -> Options<'a> {
        Options { key, downloads, arch: "x86_64", progress: &|_, _| {} }
    }

    /// A tar.gz install is staged beside itself, verified, and swapped in by renames; the
    /// staging folder goes.
    #[test]
    fn a_verified_folder_release_is_staged_and_swapped_in() {
        let t = Temp::new("tar");
        let root = t.0.join("fieldcraft");
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/fieldcraft"), "old").unwrap();
        let install = Install::TarGz { root: root.clone() };
        let server = Server::start().unwrap();
        let (sk, public) = keypair();
        let release = publish(&server, Some(&sk), &[("fieldcraft-9.0.0-linux-x86_64.tar.gz", linux_tar("new"))], false);

        let staged = stage(&Client::new(&server.url()), &release, &install, &options(&public, &t.0)).unwrap();
        assert!(staged.dir.starts_with(&t.0) && staged.dir.file_name().unwrap() == ".fieldcraft-update-9.0.0");
        assert_eq!(fs::read_to_string(staged.payload.join("bin/fieldcraft")).unwrap(), "new");
        assert_eq!(fs::read_to_string(root.join("bin/fieldcraft")).unwrap(), "old", "nothing changes before the swap");
        let Finish::Swap(p) = plan(&staged, false).unwrap() else { panic!() };
        apply(&p).unwrap();
        assert_eq!(fs::read_to_string(root.join("bin/fieldcraft")).unwrap(), "new");
        assert!(!staged.dir.exists());
    }

    /// The Windows folder is swapped file by file (a running .exe can be renamed); the old files
    /// wait in the staging folder, which the next start removes.
    #[test]
    fn a_windows_folder_is_swapped_file_by_file() {
        let t = Temp::new("win");
        let dir = t.0.join("FieldCraft");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("fieldcraft.exe"), "old").unwrap();
        fs::write(dir.join("settings-the-user-added.txt"), "mine").unwrap();
        let install = Install::Windows { dir: dir.clone() };
        let server = Server::start().unwrap();
        let (sk, public) = keypair();
        let release = publish(&server, Some(&sk), &[("fieldcraft-9.0.0-windows-x64.zip", windows_zip("new"))], false);
        let staged = stage(&Client::new(&server.url()), &release, &install, &options(&public, &t.0)).unwrap();
        assert_eq!(staged.entries, ["THIRD-PARTY-LICENSES.txt", "fieldcraft-cli.exe", "fieldcraft.exe"]);
        let Finish::Swap(p) = plan(&staged, false).unwrap() else { panic!() };
        apply(&p).unwrap();
        assert_eq!(fs::read_to_string(dir.join("fieldcraft.exe")).unwrap(), "new");
        assert_eq!(fs::read_to_string(dir.join("settings-the-user-added.txt")).unwrap(), "mine");
        fs::create_dir_all(t.0.join(".fieldcraft-update-8.0.0/previous")).unwrap();
        clean_stale(&install);
        assert!(!t.0.join(".fieldcraft-update-8.0.0").exists());
    }

    /// A failed swap puts back what it moved.
    #[test]
    fn a_failed_swap_is_undone() {
        let t = Temp::new("undo");
        let (a, b) = (t.0.join("a"), t.0.join("b"));
        fs::write(&a, "old a").unwrap();
        fs::write(&b, "old b").unwrap();
        fs::write(t.0.join("new-a"), "new a").unwrap();
        let p = Plan {
            replace: vec![
                crate::install::Replace { target: a.clone(), new: t.0.join("new-a"), backup: t.0.join("stage/a") },
                crate::install::Replace { target: b.clone(), new: t.0.join("missing"), backup: t.0.join("stage/b") },
            ],
            launch: None,
            cleanup: t.0.join("stage"),
        };
        assert!(apply(&p).is_err());
        assert_eq!((fs::read_to_string(&a).unwrap(), fs::read_to_string(&b).unwrap()), ("old a".into(), "old b".into()));
    }

    /// A checksum list that does not match the download, one signed by another key, or none at
    /// all: nothing is installed, and with a bad signature the asset is not even downloaded.
    #[test]
    fn unverified_releases_are_refused() {
        let t = Temp::new("refuse");
        let root = t.0.join("fieldcraft");
        fs::create_dir_all(root.join("bin")).unwrap();
        let install = Install::TarGz { root: root.clone() };
        let asset = [("fieldcraft-9.0.0-linux-x86_64.tar.gz", linux_tar("new"))];
        let (sk, public) = keypair();

        let server = Server::start().unwrap();
        let release = publish(&server, Some(&sk), &asset, true);
        let e = stage(&Client::new(&server.url()), &release, &install, &options(&public, &t.0)).unwrap_err();
        assert!(matches!(&e, Error::Verify(m) if m.contains("does not match its checksum")), "{e}");
        assert!(!t.0.join(".fieldcraft-update-9.0.0").exists(), "the staging folder goes");

        let (other, _) = keypair();
        let server = Server::start().unwrap();
        let release = publish(&server, Some(&other), &asset, false);
        let e = stage(&Client::new(&server.url()), &release, &install, &options(&public, &t.0)).unwrap_err();
        assert!(matches!(&e, Error::Verify(m) if m.contains("not signed by the FieldCraft release key")), "{e}");
        assert_eq!(server.requests(), 2, "the list and its signature, not the asset");

        let server = Server::start().unwrap();
        let release = publish(&server, None, &asset, false);
        let e = stage(&Client::new(&server.url()), &release, &install, &options(&public, &t.0)).unwrap_err();
        assert!(matches!(&e, Error::Verify(m) if m.contains("not signed")), "{e}");
        assert_eq!(server.requests(), 0);

        // A development build checks but does not install.
        let e = stage(&Client::new(&server.url()), &release, &Install::Unmanaged("a development build".into()), &options(&public, &t.0)).unwrap_err();
        assert!(matches!(e, Error::NotSupported(_)));
    }

    /// An AppImage is replaced as one file; a .deb is downloaded for the system's installer.
    #[test]
    fn app_images_and_packages() {
        let t = Temp::new("files");
        let file = t.0.join("FieldCraft.AppImage");
        fs::write(&file, "old").unwrap();
        let server = Server::start().unwrap();
        let (sk, public) = keypair();
        let release = publish(
            &server,
            Some(&sk),
            &[("fieldcraft-9.0.0-linux-x86_64.AppImage", b"new".to_vec()), ("fieldcraft_9.0.0_amd64.deb", b"deb".to_vec())],
            false,
        );
        let c = Client::new(&server.url());
        let staged = stage(&c, &release, &Install::AppImage { file: file.clone() }, &options(&public, &t.0)).unwrap();
        let Finish::Swap(p) = plan(&staged, false).unwrap() else { panic!() };
        apply(&p).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");

        let downloads = t.0.join("downloads");
        let staged = stage(&c, &release, &Install::Deb, &options(&public, &downloads)).unwrap();
        let Finish::Package { file, command } = plan(&staged, true).unwrap() else { panic!() };
        assert_eq!(fs::read(&file).unwrap(), b"deb");
        assert!(command.starts_with("sudo apt install ") && command.ends_with("fieldcraft_9.0.0_amd64.deb"), "{command}");
    }

    /// Paths that leave the staging folder are refused.
    #[test]
    fn archives_cannot_write_outside_their_folder() {
        let t = Temp::new("escape");
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        z.start_file("../escaped.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x").unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let file = t.0.join("bad.zip");
        fs::write(&file, bytes).unwrap();
        assert!(unzip(&file, &t.0.join("out")).is_err());
        assert!(!t.0.join("escaped.txt").exists());
    }
}
