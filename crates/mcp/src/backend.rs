//! Where tool calls end up: an in-process headless session, or a running app's control channel
//! (docs/control-protocol.md: `engine.execute`, `engine.commands`, `ui.view`, `ui.camera`,
//! `ui.render`, `ui.capture`, `ui.select`, `ui.selection`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fieldcraft_engine::Session;
use fieldcraft_engine::view::{Palette, render_png};
use fieldcraft_io::roots::Roots;
use fieldcraft_render::{Orientation, StandardView};
use serde_json::{Value, json};

/// What a picture shows.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureRequest {
    /// A standard view to turn to first (`iso`, `front`…; `home` and `fit` frame the model).
    pub view: Option<String>,
    /// A camera orientation to turn to first (degrees; `view` and `camera` are not given together).
    pub camera: Option<Orientation>,
    /// The whole app window (`true`) or the model only.
    pub window: bool,
    /// Image size in pixels (a window capture: points); `None` keeps the default.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A PNG and what it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub png: Vec<u8>,
    pub info: Value,
}

/// Something that runs FieldCraft commands and draws pictures.
pub trait Backend {
    /// Run a command (`engine.execute`).
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String>;
    /// Every command with its parameter docs (`engine.commands`).
    fn commands(&mut self) -> Result<Value, String>;
    /// A PNG of the model, or of the app's window when one is attached.
    fn picture(&mut self, request: &PictureRequest) -> Result<Picture, String>;
    /// A request to the app's window (`ui.select`, `ui.selection`): an error without one.
    fn window(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// The folders file access is confined to (checked by the server for the files it writes
    /// itself).
    fn roots(&self) -> &Roots;
    /// For the client's instructions.
    fn describe(&self) -> String;
}

/// Default size of a model picture.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (960, 600);
/// Largest picture side the server asks for, in pixels.
pub(crate) const MAX_SIDE: u32 = 4096;
/// Largest image read back from the app.
const MAX_IMAGE_BYTES: u64 = 64 << 20;
/// Longest reply line accepted from the app.
const MAX_REPLY: u64 = 256 << 20;

fn standard_view(v: &str) -> Result<StandardView, String> {
    match v {
        "home" | "fit" => Ok(StandardView::Iso),
        _ => StandardView::parse(v).ok_or_else(|| format!("unknown view `{}`", v.chars().take(40).collect::<String>())),
    }
}

/// An in-process headless session; pictures come from the CPU rasterizer.
pub struct Headless {
    pub session: Session,
    /// The orientation of pictures; each one frames everything drawn, unless the orientation
    /// has a distance.
    pub view: Orientation,
    roots: Roots,
}

impl Default for Headless {
    fn default() -> Self {
        Headless::new(Roots::default())
    }
}

impl Headless {
    /// An empty session whose commands read and write files only as `roots` allow.
    pub fn new(roots: Roots) -> Self {
        let mut session = Session::default();
        session.file_access = roots.clone();
        Headless { session, view: Orientation::of(StandardView::Iso), roots }
    }
}

impl Backend for Headless {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        self.session.execute(command, &params).map_err(|e| e.to_string())
    }

    fn commands(&mut self) -> Result<Value, String> {
        Ok(fieldcraft_engine::commands().iter().map(fieldcraft_engine::CommandSpec::listing).collect())
    }

    fn picture(&mut self, r: &PictureRequest) -> Result<Picture, String> {
        if let Some(v) = &r.view {
            self.view = Orientation::of(standard_view(v)?);
        }
        if let Some(o) = r.camera {
            self.view = o;
        }
        let w = r.width.unwrap_or(DEFAULT_SIZE.0).clamp(16, MAX_SIDE);
        let h = r.height.unwrap_or(DEFAULT_SIZE.1).clamp(16, MAX_SIDE);
        let (png, shown) = render_png(&self.session, &self.view, w as usize, h as usize, &Palette::LIGHT).map_err(|e| e.to_string())?;
        let contour = shown
            .contour
            .map(|(field, min, max)| json!({"field": field, "min": {"node": min.0, "value": min.1}, "max": {"node": max.0, "value": max.1}}));
        let info = json!({
            "source": "model",
            "view": self.view,
            "width": w,
            "height": h,
            "results": shown.step.map(|step| json!({"step": step, "scale": shown.scale, "contour": contour})),
            "undrawn_bodies": shown.failed,
            "sampled_symbols": shown.sampled,
        });
        Ok(Picture { png, info })
    }

    fn window(&mut self, method: &str, _params: Value) -> Result<Value, String> {
        Err(format!("{method} needs the app's window: start the server with --connect to a running FieldCraft"))
    }

    fn roots(&self) -> &Roots {
        &self.roots
    }

    fn describe(&self) -> String {
        "a headless in-process FieldCraft session (no window; screenshots are CPU renders)".into()
    }
}

/// A running FieldCraft app reached through its loopback control channel
/// (`fieldcraft --control PORT`).
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    roots: Roots,
}

static SHOT: AtomicU64 = AtomicU64::new(0);

impl Remote {
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1, roots: Roots::default() };
        r.reconnect()?;
        Ok(r)
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // Meshing and solving big models take a while; the app answers a request it
                    // has not finished within 60 s with a timeout of its own.
                    s.set_read_timeout(Some(Duration::from_secs(600))).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        if self.conn.is_none() {
            self.reconnect()?;
        }
        let Some((reader, writer)) = self.conn.as_mut() else { return Err(std::io::Error::other("not connected")) };
        let sent = writer.write_all(line.as_bytes()).and_then(|()| writer.write_all(b"\n")).and_then(|()| writer.flush());
        if let Err(e) = sent {
            self.conn = None;
            return Err(e);
        }
        let mut reply = String::new();
        let n = match reader.by_ref().take(MAX_REPLY).read_line(&mut reply) {
            Ok(n) => n,
            Err(e) => {
                // A timed-out read leaves a late reply on the stream: start afresh next time.
                self.conn = None;
                return Err(e);
            }
        };
        if n == 0 || !reply.ends_with('\n') {
            self.conn = None;
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "connection closed (or reply too large)"));
        }
        Ok(reply)
    }

    /// One control-channel request.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let reply = match self.roundtrip(&line) {
            Ok(r) => r,
            // One retry on a fresh connection (the app may have restarted).
            Err(_) => self.roundtrip(&line).map_err(|e| format!("FieldCraft at {}: {e}", self.addr))?,
        };
        let v: Value = serde_json::from_str(&reply).map_err(|e| format!("bad reply from the app: {e}"))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("error").to_string())
        }
    }
}

/// A temporary PNG path the app writes and the server reads back (both on this computer).
fn temp_png() -> PathBuf {
    let n = SHOT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("fieldcraft-mcp-{}-{n}.png", std::process::id()))
}

impl Backend for Remote {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        self.call("engine.execute", json!({"command": command, "params": params}))
    }

    fn commands(&mut self) -> Result<Value, String> {
        self.call("engine.commands", json!({}))
    }

    fn picture(&mut self, r: &PictureRequest) -> Result<Picture, String> {
        if let Some(v) = &r.view {
            standard_view(v)?;
            self.call("ui.view", json!({"view": v}))?;
        }
        if let Some(o) = r.camera {
            self.call("ui.camera", json!(o))?;
        }
        let path = temp_png();
        let shown = path.to_string_lossy().into_owned();
        let mut params = json!({"path": shown});
        let method = if r.window {
            for (k, v) in [("width", r.width), ("height", r.height)] {
                if let Some(v) = v {
                    params[k] = json!(v.clamp(16, MAX_SIDE));
                }
            }
            "ui.capture"
        } else {
            params["width"] = json!(r.width.unwrap_or(DEFAULT_SIZE.0).clamp(16, MAX_SIDE));
            params["height"] = json!(r.height.unwrap_or(DEFAULT_SIZE.1).clamp(16, MAX_SIDE));
            "ui.render"
        };
        let result = self.call(method, params);
        let read = result.and_then(|info| {
            let len = std::fs::metadata(&path).map_err(|e| format!("{shown}: {e} (is the app on another computer?)"))?.len();
            if len > MAX_IMAGE_BYTES {
                return Err(format!("{shown}: image too large ({len} bytes)"));
            }
            let png = std::fs::read(&path).map_err(|e| format!("{shown}: {e}"))?;
            Ok(Picture { png, info: json!({"source": if r.window { "window" } else { "model" }, "app": info}) })
        });
        let _ = std::fs::remove_file(&path);
        read
    }

    fn window(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call(method, params)
    }

    fn roots(&self) -> &Roots {
        &self.roots
    }

    fn describe(&self) -> String {
        format!("connected to the FieldCraft app at {} (the user watches its window)", self.addr)
    }
}
