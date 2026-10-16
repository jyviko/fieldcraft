//! `fieldcraft-cli snapshot`: a script in, a PNG out, with no window and no display. The picture
//! is the viewport's: the engine's scene ([`fieldcraft_engine::view::frame`]) drawn with the
//! window's GPU pipelines ([`fieldcraft_gpu::capture`]) on a hardware GPU or a software one
//! (lavapipe, llvmpipe, WARP), and by the CPU rasterizer as the last tier (`FIELDCRAFT_GPU`,
//! [`fieldcraft_gpu::select`]).

use fieldcraft_engine::view::{self, Palette};
use fieldcraft_gpu::capture::Capturer;
use fieldcraft_gpu::select::Policy;
use fieldcraft_render::{Orientation, StandardView};

/// The picture size when `--size` is not given.
const DEFAULT_SIZE: (u32, u32) = (1280, 800);

#[derive(Debug, PartialEq)]
pub struct Options {
    pub script: String,
    pub out: String,
    pub view: StandardView,
    pub size: (u32, u32),
}

/// A view name: a standard view, or the axis looked along (`x`: from the right, `y`: from the
/// front, `z`: from the top).
fn view_of(s: &str) -> Option<StandardView> {
    match s.to_ascii_lowercase().as_str() {
        "x" => Some(StandardView::Right),
        "y" => Some(StandardView::Front),
        "z" => Some(StandardView::Top),
        other => StandardView::parse(other),
    }
}

/// `WxH`, each side 1..=the renderer's limit.
fn size_of(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?);
    let max = fieldcraft_gpu::capture::MAX_SIDE;
    ((1..=max).contains(&w) && (1..=max).contains(&h) && u64::from(w) * u64::from(h) <= fieldcraft_gpu::capture::MAX_PIXELS).then_some((w, h))
}

/// The options: the script, then each flag once, as `--flag value` or `--flag=value`.
pub fn options(args: &[String]) -> Result<Options, String> {
    let (mut script, mut out, mut view, mut size) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if !arg.starts_with("--") {
            if script.replace(arg.clone()).is_some() {
                return Err(format!("snapshot takes one script, not `{arg}` too"));
            }
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (arg.as_str(), None),
        };
        let value = inline.or_else(|| it.next().cloned()).filter(|v| !v.is_empty()).ok_or_else(|| format!("{flag} needs a value"))?;
        let twice = match flag {
            "--out" => out.replace(value).is_some(),
            "--view" => view
                .replace(
                    view_of(&value).ok_or_else(|| format!("--view: unknown view `{value}` (iso, x, y, z, front, back, left, right, top, bottom)"))?,
                )
                .is_some(),
            "--size" => size
                .replace(size_of(&value).ok_or_else(|| {
                    format!(
                        "--size: `{value}` is not WxH with each side 1..={} and at most {} pixels",
                        fieldcraft_gpu::capture::MAX_SIDE,
                        fieldcraft_gpu::capture::MAX_PIXELS
                    )
                })?)
                .is_some(),
            other => return Err(format!("snapshot: unknown option `{other}`")),
        };
        if twice {
            return Err(format!("{flag} is given twice"));
        }
    }
    Ok(Options {
        script: script.ok_or("snapshot needs a script")?,
        out: out.ok_or("snapshot needs --out FILE.png")?,
        view: view.unwrap_or(StandardView::Iso),
        size: size.unwrap_or(DEFAULT_SIZE),
    })
}

/// What drew a picture.
#[derive(Debug, PartialEq)]
pub enum Drawn {
    Gpu { adapter: String, tier: &'static str, samples: u32 },
    Cpu { why: Option<String> },
}

/// The PNG of the session `o` describes, drawn as `policy` says. Under [`Policy::Auto`] a GPU
/// that cannot be opened or fails to draw falls back to the CPU rasterizer (the reason is
/// returned); a strict policy returns the error.
pub fn picture(o: &Options, policy: Policy) -> Result<(Vec<u8>, Drawn), String> {
    let session = crate::session_of(&o.script)?;
    let (w, h) = o.size;
    let f = view::frame(&session, &Orientation::of(o.view), w as usize, h as usize, &Palette::LIGHT);
    let gpu = Capturer::open(policy).and_then(|c| match c {
        Some(c) => c.rgba(&f.scene, &[&f.symbols], &f.cam, w, h).map(|px| Some((px, c))),
        None => Ok(None),
    });
    let why = match gpu {
        Ok(Some((px, c))) => {
            let png = fieldcraft_render::png(w as usize, h as usize, &px).ok_or("the picture could not be encoded")?;
            return Ok((png, Drawn::Gpu { adapter: c.adapter.clone(), tier: c.tier.label(), samples: c.samples() }));
        }
        Ok(None) => (policy == Policy::Auto).then(|| "no GPU adapter".to_string()),
        Err(e) if policy == Policy::Auto => Some(e.to_string()),
        Err(e) => return Err(e.to_string()),
    };
    let png = fieldcraft_render::render_layers_png(&f.scene, &[&f.symbols], &f.cam, w as usize, h as usize).map_err(|e| format!("render: {e}"))?;
    Ok((png, Drawn::Cpu { why }))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let o = options(args)?;
    let policy = Policy::from_env().map_err(|e| e.to_string())?;
    let (png, drawn) = picture(&o, policy)?;
    std::fs::write(&o.out, &png).map_err(|e| format!("{}: {e}", o.out))?;
    let renderer = match &drawn {
        Drawn::Gpu { adapter, tier, samples } => serde_json::json!({"kind": tier, "adapter": adapter, "msaa": samples}),
        Drawn::Cpu { why } => serde_json::json!({"kind": "CPU rasterizer", "why": why}),
    };
    let report = serde_json::json!({"out": o.out, "width": o.size.0, "height": o.size.1, "view": o.view.label(), "renderer": renderer});
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(args: &[&str]) -> Result<Options, String> {
        options(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn options_are_checked() {
        let o = opts(&["s.json", "--out", "a.png"]).unwrap();
        assert_eq!(o, Options { script: "s.json".into(), out: "a.png".into(), view: StandardView::Iso, size: DEFAULT_SIZE });
        let o = opts(&["--view=z", "s.json", "--size", "640x480", "--out=b.png"]).unwrap();
        assert_eq!((o.view, o.size), (StandardView::Top, (640, 480)));
        assert_eq!(opts(&["s", "--out", "o", "--view", "X"]).unwrap().view, StandardView::Right);
        assert_eq!(opts(&["s", "--out", "o", "--view", "front"]).unwrap().view, StandardView::Front);
        for bad in [
            vec![],
            vec!["s.json"],
            vec!["--out", "a.png"],
            vec!["s", "t", "--out", "o"],
            vec!["s", "--out"],
            vec!["s", "--out", "o", "--out", "p"],
            vec!["s", "--out", "o", "--view", "diagonal"],
            vec!["s", "--out", "o", "--size", "0x10"],
            vec!["s", "--out", "o", "--size", "10"],
            vec!["s", "--out", "o", "--size", "99999x10"],
            vec!["s", "--out", "o", "--size", "8192x8192"],
            vec!["s", "--out", "o", "--size", "-5x10"],
            vec!["s", "--out", "o", "--nope", "1"],
        ] {
            assert!(opts(&bad).is_err(), "{bad:?}");
        }
    }

    /// PNG width, height and pixels (RGBA).
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut r = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    /// The cantilever drawn headless: a PNG of the size asked for, with the model in it (pixels
    /// of the hex colour) and its element edges (dark pixels). `auto` draws on the GPU when the
    /// machine has one and on the CPU otherwise (a CI box without a software GPU); `cpu` always
    /// on the CPU, and both pictures show the same model.
    #[test]
    fn snapshots_the_model_and_its_mesh_headless() {
        let dir = std::env::temp_dir().join(format!("fieldcraft-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("cantilever.json");
        std::fs::write(&script, fieldcraft_engine::sample::cantilever().to_string()).unwrap();
        let o = Options { script: script.to_string_lossy().into(), out: String::new(), view: StandardView::Iso, size: (320, 200) };
        for policy in [Policy::Auto, Policy::Cpu] {
            let (png, drawn) = picture(&o, policy).unwrap();
            let (w, h, px) = decode(&png);
            assert_eq!((w, h), (320, 200), "{drawn:?}");
            let hex = Palette::LIGHT.hex;
            let near = |p: &[u8], c: fieldcraft_render::Rgb, d: u8| p[0].abs_diff(c.0) <= d && p[1].abs_diff(c.1) <= d && p[2].abs_diff(c.2) <= d;
            // Shaded faces vary in brightness around the hex colour; the edges are near black.
            let faces = px.chunks(4).filter(|p| near(p, hex, 60) && p[2] > p[0]).count();
            let edges = px.chunks(4).filter(|p| p[0] < 90 && p[1] < 90 && p[2] < 90).count();
            assert!(faces > 500, "{drawn:?}: {faces} face pixels");
            assert!(edges > 100, "{drawn:?}: {edges} edge pixels");
            if policy == Policy::Cpu {
                assert_eq!(drawn, Drawn::Cpu { why: None });
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
