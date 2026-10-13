//! ModelCraft desktop app.
//!
//! Usage: `modelcraft [--sample] [--control PORT]`
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
use modelcraft_ui_egui::ModelApp;

const USAGE: &str = "usage: modelcraft [--sample] [--control PORT]";
const DEPTH_BITS: u8 = 24;
const MSAA: u16 = 4;

struct App {
    app: ModelApp,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Command-line options.
#[derive(Debug, Default, PartialEq)]
struct Args {
    sample: bool,
    control: Option<u16>,
}

fn parse_args(args: impl IntoIterator<Item = String>, env_port: Option<String>) -> Result<Option<Args>, String> {
    let mut out = Args { control: env_port.and_then(|p| p.trim().parse().ok()), ..Default::default() };
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sample" => out.sample = true,
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
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
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
        viewport: egui::ViewportBuilder::default().with_title("ModelCraft").with_inner_size([1400.0, 900.0]).with_min_inner_size([800.0, 500.0]),
        depth_buffer: DEPTH_BITS,
        multisampling: MSAA,
        ..Default::default()
    };
    eframe::run_native(
        "ModelCraft",
        options,
        Box::new(move |cc| {
            let mut app = ModelApp::new(Session::default());
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                eprintln!("modelcraft: graphics {:?} {}, MSAA {}x (requested {MSAA}x)", info.backend, info.name, rs.msaa_samples);
                app.set_wgpu(rs, DEPTH_BITS, rs.msaa_samples, &cc.egui_ctx);
            }
            if let Some(port) = args.control {
                app = app.with_control(control_server::start(port, cc.egui_ctx.clone()));
            }
            if args.sample {
                // The status bar reports a failure; the app still opens.
                let _ = app.run_script(&modelcraft_engine::sample::script());
            }
            Ok(Box::new(App { app }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str], env: Option<&str>) -> Result<Option<Args>, String> {
        parse_args(a.iter().map(|s| s.to_string()), env.map(str::to_string))
    }

    #[test]
    fn arguments() {
        assert_eq!(parse(&[], None), Ok(Some(Args::default())));
        assert_eq!(parse(&["--sample", "--control", "7070"], None), Ok(Some(Args { sample: true, control: Some(7070) })));
        assert_eq!(parse(&[], Some("9000")), Ok(Some(Args { sample: false, control: Some(9000) })));
        assert_eq!(parse(&[], Some("nope")), Ok(Some(Args::default())));
        assert!(parse(&["--control"], None).is_err());
        assert!(parse(&["--control", "99999"], None).is_err());
        assert!(parse(&["--frobnicate"], None).is_err());
        assert_eq!(parse(&["--help"], None), Ok(None));
    }
}
