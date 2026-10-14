//! ModelCraft desktop app.
//!
//! Usage: `modelcraft [--sample [NAME]] [--control PORT] [FILE…]` (files open as if dropped on the
//! window; NAME is a sample of `modelcraft_engine::sample`, `parts` by default)
//!
//! `--sample` opens a sample model (`modelcraft_engine::sample`). `--control PORT` (or
//! `MODELCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"engine.execute","params":{"command":"model.info","params":{}}}`. See
//! `modelcraft_ui_egui::control` and docs/control-protocol.md.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod control_server;

use modelcraft_engine::Session;
use modelcraft_engine::autosave::{self, Autosaver};
use modelcraft_ui_egui::{Filter, ModelApp, Services};

const USAGE: &str = "usage: modelcraft [--sample [parts|cantilever|impact]] [--control PORT] [FILE…]  (FILE: .modelcraft, .solidcraft or .3mf)";
const DEPTH_BITS: u8 = 24;
/// The window draws its own caption buttons where the OS ones would sit apart from the app bar.
const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");
const MSAA: u16 = 4;

/// The window and Dock icon: the 256 px export of the application icon (assets/app-icon).
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/icon.iconset/icon_128x128@2x.png");

/// The application icon for the window; eframe's default icon if the PNG cannot be decoded.
fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_else(|e| {
        log::warn!("app icon: {e}");
        egui::IconData::default()
    })
}

/// How often unsaved changes are autosaved for recovery (seconds).
const AUTOSAVE_SECONDS: f64 = 30.0;

struct App {
    app: ModelApp,
    /// Autosave for crash recovery (`None` when the recovery folder is unavailable).
    autosaver: Option<Autosaver>,
    /// App time of the last autosave check.
    autosaved_at: f64,
}

impl App {
    fn autosave(&mut self) {
        if let Some(a) = &mut self.autosaver
            && let Err(e) = a.save(&self.app.session)
        {
            log::warn!("autosave: {e}");
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        if self.app.now - self.autosaved_at >= AUTOSAVE_SECONDS {
            self.autosaved_at = self.app.now;
            self.autosave();
        }
        if self.autosaver.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(AUTOSAVE_SECONDS));
        }
    }
    fn on_exit(&mut self) {
        if let Some(a) = self.autosaver.take()
            && let Err(e) = a.close(&self.app.session)
        {
            log::warn!("autosave: {e}");
        }
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Native file pickers and the recent-files file, for the UI.
fn services() -> Services {
    let dialog = |filters: &[Filter]| filters.iter().fold(rfd::FileDialog::new(), |d, f| d.add_filter(f.name, f.extensions));
    Services {
        pick_open: Some(Box::new(move |filters: &[Filter]| dialog(filters).pick_file().map(|p| p.to_string_lossy().to_string()))),
        pick_save: Some(Box::new(move |name: &str, filters: &[Filter]| {
            dialog(filters).set_file_name(name).save_file().map(|p| p.to_string_lossy().to_string())
        })),
        // Beside the recovery folder in the per-user data folder.
        recent_path: modelcraft_io::recovery::default_dir().and_then(|d| d.parent().map(|p| p.join("recent.json"))),
    }
}

/// Command-line options.
#[derive(Debug, Default, PartialEq)]
struct Args {
    /// The sample to open (`--sample [NAME]`).
    sample: Option<String>,
    control: Option<u16>,
    /// Files to open (models, designs, tagged meshes).
    files: Vec<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>, env_port: Option<String>) -> Result<Option<Args>, String> {
    let mut out = Args { control: env_port.and_then(|p| p.trim().parse().ok()), ..Default::default() };
    let mut args = args.into_iter().peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => {
                // An optional sample name follows.
                let name = args.peek().filter(|a| modelcraft_engine::sample::find(a).is_some()).cloned();
                if name.is_some() {
                    args.next();
                }
                out.sample = Some(name.unwrap_or_else(|| "parts".into()));
            }
            "--control" => {
                let port = args.next().ok_or("--control needs a port")?;
                out.control = Some(port.parse().map_err(|_| format!("--control: `{port}` is not a port number"))?);
            }
            "--version" | "-V" => {
                println!("modelcraft {}", modelcraft_ui_egui::VERSION);
                return Ok(None);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other if other.starts_with('-') => return Err(format!("unknown argument `{other}`\n{USAGE}")),
            file => out.files.push(file.to_string()),
        }
    }
    Ok(Some(out))
}

fn main() -> eframe::Result {
    let args = match parse_args(std::env::args().skip(1), std::env::var("MODELCRAFT_CONTROL_PORT").ok()) {
        Ok(Some(a)) => a,
        Ok(None) => return Ok(()),
        Err(e) => {
            eprintln!("modelcraft: {e}");
            std::process::exit(2);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ModelCraft")
            .with_icon(app_icon())
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            // The app bar is the title bar (as in the other Craft apps): on macOS under the
            // traffic lights, elsewhere with its own caption buttons and resize zones.
            .with_decorations(!CUSTOM_TITLEBAR)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        depth_buffer: DEPTH_BITS,
        multisampling: MSAA,
        ..Default::default()
    };
    eframe::run_native(
        "ModelCraft",
        options,
        Box::new(move |cc| {
            let mut app = ModelApp::new(Session::default()).with_services(services());
            app.custom_titlebar = CUSTOM_TITLEBAR;
            app.integrated_titlebar = cfg!(target_os = "macos");
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                eprintln!("modelcraft: graphics {:?} {}, MSAA {}x (requested {MSAA}x)", info.backend, info.name, rs.msaa_samples);
                app.set_wgpu(rs, DEPTH_BITS, rs.msaa_samples, &cc.egui_ctx);
            }
            if let Some(port) = args.control {
                app = app.with_control(control_server::start(port, cc.egui_ctx.clone()));
            }
            if let Some(name) = &args.sample {
                app.open_sample(name);
            }
            for f in &args.files {
                app.open_path(f);
            }
            let autosaver = autosave::default_dir().and_then(|dir| {
                let listed = app.session.execute("file.recovery.list", &serde_json::json!({"dir": dir}));
                let recoverable = listed.ok().and_then(|v| v.as_array().map(Vec::len)).unwrap_or(0);
                if recoverable > 0 {
                    app.set_status(format!("{recoverable} autosaved model(s) can be recovered: File › Recover…"), false);
                }
                Autosaver::new(&dir).map_err(|e| log::warn!("autosave unavailable: {e}")).ok()
            });
            Ok(Box::new(App { app, autosaver, autosaved_at: 0.0 }))
        }),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_icon_decodes() {
        let icon = super::app_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    use super::*;

    fn parse(a: &[&str], env: Option<&str>) -> Result<Option<Args>, String> {
        parse_args(a.iter().map(|s| s.to_string()), env.map(str::to_string))
    }

    #[test]
    fn arguments() {
        assert_eq!(parse(&[], None), Ok(Some(Args::default())));
        assert_eq!(
            parse(&["--sample", "--control", "7070"], None),
            Ok(Some(Args { sample: Some("parts".into()), control: Some(7070), files: vec![] }))
        );
        assert_eq!(parse(&[], Some("9000")), Ok(Some(Args { control: Some(9000), ..Default::default() })));
        assert_eq!(parse(&[], Some("nope")), Ok(Some(Args::default())));
        assert!(parse(&["--control"], None).is_err());
        assert!(parse(&["--control", "99999"], None).is_err());
        assert!(parse(&["--frobnicate"], None).is_err());
        assert_eq!(parse(&["--sample", "impact"], None).map(|a| a.and_then(|a| a.sample)), Ok(Some("impact".into())));
        let a = parse(&["--sample", "x.modelcraft"], None).unwrap().unwrap();
        assert_eq!((a.sample.as_deref(), a.files), (Some("parts"), vec!["x.modelcraft".to_string()]));
        assert_eq!(parse(&["a.modelcraft", "--sample"], None).map(|a| a.map(|a| a.files)), Ok(Some(vec!["a.modelcraft".to_string()])));
        assert_eq!(parse(&["--help"], None), Ok(None));
    }
}
