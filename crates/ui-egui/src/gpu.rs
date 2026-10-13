//! GPU viewport: lit triangles and screen-space wide lines with a depth buffer, drawn in an
//! `egui_wgpu` paint callback. Geometry is uploaded when the scene changes; each frame only the
//! camera uniform changes. Nodes are drawn as zero-length lines, which the line shader turns
//! into squares.
//!
//! Adapted from SolidCraft's `crates/ui-egui/src/gpu.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;
use egui_wgpu::wgpu::util::DeviceExt;
use modelcraft_render::{Rgb, Scene};

/// Triangle vertex: position (3 × f32), normal (3 × f32), sRGB colour (4 × u8).
pub const TRI_SIZE: usize = 28;
/// Line instance: a, b (3 × f32 each), colour (4 × u8), width in physical pixels (f32).
pub const LINE_SIZE: usize = 32;
const UNIFORM_SIZE: u64 = 112;

/// Scene geometry in the GPU's vertex formats, waiting to be uploaded.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct GpuScene {
    pub tris: Vec<u8>,
    pub lines: Vec<u8>,
}

impl GpuScene {
    /// The scene's triangles, lines and points; widths and sizes scaled by `pixels_per_point`
    /// (the scene gives them in points).
    pub fn from_scene(scene: &Scene, pixels_per_point: f32) -> GpuScene {
        let mut g = GpuScene { tris: Vec::with_capacity(scene.tris.len() * 3 * TRI_SIZE), lines: Vec::new() };
        for t in &scene.tris {
            for p in t.p {
                g.vertex(p.to_f32(), t.normal.to_f32(), t.color);
            }
        }
        for l in &scene.lines {
            g.line(l.a.to_f32(), l.b.to_f32(), l.color, l.width as f32 * pixels_per_point);
        }
        for p in &scene.points {
            g.line(p.p.to_f32(), p.p.to_f32(), p.color, p.size as f32 * pixels_per_point);
        }
        g
    }
    fn vertex(&mut self, p: [f32; 3], n: [f32; 3], c: Rgb) {
        for v in p.iter().chain(&n) {
            self.tris.extend_from_slice(&v.to_le_bytes());
        }
        self.tris.extend_from_slice(&[c.0, c.1, c.2, 255]);
    }
    fn line(&mut self, a: [f32; 3], b: [f32; 3], c: Rgb, width: f32) {
        for v in a.iter().chain(&b) {
            self.lines.extend_from_slice(&v.to_le_bytes());
        }
        self.lines.extend_from_slice(&[c.0, c.1, c.2, 255]);
        self.lines.extend_from_slice(&width.to_le_bytes());
    }
}

/// Where the UI hands a new scene to the renderer (taken on the next frame's `prepare`).
pub type SceneSlot = Arc<Mutex<Option<GpuScene>>>;

/// Per-frame parameters.
pub struct ViewportCallback {
    /// Changes whenever the scene does; the slot holds the scene for a new key.
    pub key: u64,
    pub slot: SceneSlot,
    /// Column-major view-projection.
    pub view_proj: [[f32; 4]; 4],
    /// Direction toward the eye (for lighting).
    pub back: [f32; 3],
    /// Eye position and 1 for a perspective camera (0: orthographic).
    pub eye: [f32; 4],
    /// Viewport size in physical pixels.
    pub size_px: [f32; 2],
}

struct Batch {
    buffer: wgpu::Buffer,
    count: u32,
}

struct Resources {
    tri: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    linear_out: bool,
    key: Option<u64>,
    tris: Option<Batch>,
    lines: Option<Batch>,
}

const SHADER: &str = r#"
struct U {
    vp: mat4x4<f32>,
    back: vec4<f32>,   // xyz = toward the eye, w = 1 for linear output
    screen: vec4<f32>, // viewport width, height (px), depth bias
    eye: vec4<f32>,    // eye position, w = 1 for perspective
};
@group(0) @binding(0) var<uniform> u: U;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(c, pow((c + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4)), c > vec3<f32>(0.04045));
}

fn out_color(c: vec4<f32>) -> vec4<f32> {
    if (u.back.w > 0.5) {
        return vec4<f32>(to_linear(c.rgb), c.a);
    }
    return c;
}

struct TOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) c: vec4<f32>,
};

@vertex
fn vs_tri(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) c: vec4<f32>) -> TOut {
    var o: TOut;
    o.pos = u.vp * vec4<f32>(p, 1.0);
    o.n = n;
    o.c = c;
    return o;
}

/// Two-sided lighting: a key light above the eye, a fill from the eye and a small highlight
/// (modelcraft_render::shade computes the same on the CPU).
@fragment
fn fs_tri(i: TOut) -> @location(0) vec4<f32> {
    let v = normalize(u.back.xyz);
    var n = normalize(i.n);
    if (dot(n, v) < 0.0) { n = -n; }
    let key = normalize(v * 0.8 + vec3<f32>(-0.3, 0.2, 0.6));
    let diff = max(dot(n, key), 0.0);
    let fill = max(dot(n, v), 0.0);
    let h = normalize(key + v);
    let spec = pow(max(dot(n, h), 0.0), 40.0) * 0.25;
    let k = 0.42 + 0.38 * diff + 0.25 * fill;
    return out_color(vec4<f32>(min(i.c.rgb * k + vec3<f32>(spec), vec3<f32>(1.0)), i.c.a));
}

/// Clip-space z of `p` (clipped to `c`) pulled `s` depth biases toward the eye, so edges and
/// nodes on a face win the depth test against it.
fn biased_z(p: vec3<f32>, c: vec4<f32>, s: f32) -> f32 {
    if (u.eye.w > 0.5) {
        let q = u.vp * vec4<f32>(mix(p, u.eye.xyz, u.screen.z * 8.0 * s), 1.0);
        return q.z / max(q.w, 1e-6) * c.w;
    }
    return c.z - u.screen.z * s * c.w;
}

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) c: vec4<f32>,
};

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>, @location(2) c: vec4<f32>, @location(3) w: f32) -> LOut {
    var pa = a;
    var pb = b;
    var ca = u.vp * vec4<f32>(a, 1.0);
    var cb = u.vp * vec4<f32>(b, 1.0);
    // An end behind the near plane has no screen point: cut the segment at the plane.
    if (ca.z < 0.0 && cb.z > 0.0) {
        let k = ca.z / (ca.z - cb.z);
        ca = mix(ca, cb, k);
        pa = mix(a, b, k);
    } else if (cb.z < 0.0 && ca.z > 0.0) {
        let k = cb.z / (cb.z - ca.z);
        cb = mix(cb, ca, k);
        pb = mix(b, a, k);
    }
    let half = u.screen.xy * 0.5;
    let sa = ca.xy / max(ca.w, 1e-6) * half;
    let sb = cb.xy / max(cb.w, 1e-6) * half;
    var dir = sb - sa;
    let len = length(dir);
    // A zero-length line (a node) becomes a square `w` pixels across.
    if (len < 1e-6) { dir = vec2<f32>(1.0, 0.0); } else { dir = dir / len; }
    let nrm = vec2<f32>(-dir.y, dir.x);
    // Two triangles: corners (t, side).
    var ts = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var ss = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let t = ts[vi];
    let s = ss[vi];
    let base = select(ca, cb, t > 0.5);
    let off = nrm * s * w * 0.5 + dir * (t * 2.0 - 1.0) * w * 0.5;
    var o: LOut;
    o.pos = vec4<f32>(base.xy + off / half * base.w, biased_z(select(pa, pb, t > 0.5), base, 1.0), base.w);
    o.c = c;
    return o;
}

@fragment
fn fs_line(i: LOut) -> @location(0) vec4<f32> {
    return out_color(i.c);
}
"#;

fn tri_layout() -> wgpu::VertexBufferLayout<'static> {
    const A: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4];
    wgpu::VertexBufferLayout { array_stride: TRI_SIZE as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &A }
}

fn line_layout() -> wgpu::VertexBufferLayout<'static> {
    const A: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32];
    wgpu::VertexBufferLayout { array_stride: LINE_SIZE as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &A }
}

/// The target configuration: colour format, depth format and MSAA sample count of the app.
#[derive(Clone, Copy, Debug)]
pub struct GpuTarget {
    pub format: wgpu::TextureFormat,
    pub depth: Option<wgpu::TextureFormat>,
    pub samples: u32,
}

impl Resources {
    fn new(device: &wgpu::Device, t: GpuTarget) -> Self {
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mc_viewport"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mc_viewport"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mc_viewport"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mc_viewport_u"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mc_viewport"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let depth = |compare: wgpu::CompareFunction, write: bool| -> Option<wgpu::DepthStencilState> {
            t.depth.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        buf: wgpu::VertexBufferLayout<'static>,
                        ds: Option<wgpu::DepthStencilState>,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), buffers: &[Some(buf)], compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
                depth_stencil: ds,
                multisample: wgpu::MultisampleState { count: t.samples.max(1), mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: t.format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Resources {
            tri: pipeline("mc_tris", "vs_tri", "fs_tri", tri_layout(), depth(wgpu::CompareFunction::Less, true), None),
            line: pipeline(
                "mc_lines",
                "vs_line",
                "fs_line",
                line_layout(),
                depth(wgpu::CompareFunction::LessEqual, false),
                Some(wgpu::BlendState::ALPHA_BLENDING),
            ),
            uniform,
            bind,
            linear_out: t.format.is_srgb(),
            key: None,
            tris: None,
            lines: None,
        }
    }

    fn take(&mut self, device: &wgpu::Device, key: u64, slot: &SceneSlot) {
        if self.key == Some(key) {
            return;
        }
        if let Some(sc) = slot.lock().ok().and_then(|mut s| s.take()) {
            self.tris = upload(device, "mc_tris", &sc.tris, TRI_SIZE);
            self.lines = upload(device, "mc_lines", &sc.lines, LINE_SIZE);
            self.key = Some(key);
        }
    }
}

fn upload(device: &wgpu::Device, label: &str, bytes: &[u8], stride: usize) -> Option<Batch> {
    if bytes.len() < stride {
        return None;
    }
    Some(Batch {
        buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::VERTEX,
        }),
        count: u32::try_from(bytes.len() / stride).unwrap_or(0),
    })
}

/// Create the pipelines. Call once with the app's render state and its depth and MSAA settings.
pub fn install(rs: &egui_wgpu::RenderState, depth_bits: u8, samples: u32) -> GpuTarget {
    let t = GpuTarget { format: rs.target_format, depth: egui_wgpu::depth_format_from_bits(depth_bits, 0), samples };
    let res = Resources::new(&rs.device, t);
    rs.renderer.write().callback_resources.insert(res);
    t
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else { return Vec::new() };
        res.take(device, self.key, &self.slot);
        queue.write_buffer(&res.uniform, 0, &uniform_bytes(self, res.linear_out));
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(res) = resources.get::<Resources>() else { return };
        let vp = info.viewport_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 {
            return;
        }
        pass.set_viewport(vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32, 0.0, 1.0);
        pass.set_bind_group(0, &res.bind, &[]);
        if let Some(b) = &res.tris {
            pass.set_pipeline(&res.tri);
            pass.set_vertex_buffer(0, b.buffer.slice(..));
            pass.draw(0..b.count, 0..1);
        }
        if let Some(b) = &res.lines {
            pass.set_pipeline(&res.line);
            pass.set_vertex_buffer(0, b.buffer.slice(..));
            pass.draw(0..6, 0..b.count);
        }
    }
}

/// Uniform bytes for a frame.
fn uniform_bytes(cb: &ViewportCallback, linear: bool) -> Vec<u8> {
    let mut v: Vec<f32> = cb.view_proj.iter().flatten().copied().collect();
    v.extend([cb.back[0], cb.back[1], cb.back[2], if linear { 1.0 } else { 0.0 }]);
    v.extend([cb.size_px[0].max(1.0), cb.size_px[1].max(1.0), 2e-4, 0.0]);
    v.extend(cb.eye);
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use egui_wgpu::wgpu::naga;
    use modelcraft_render::{SceneLine, ScenePoint, SceneTri, Vec3};

    use super::*;

    /// The viewport shader parses and validates (a bad shader would only fail at run time).
    #[test]
    fn shader_is_valid() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).unwrap();
    }

    #[test]
    fn scene_bytes_match_the_vertex_layouts() {
        let scene = Scene {
            tris: vec![SceneTri { p: [Vec3::ZERO, Vec3::X, Vec3::Y], normal: Vec3::Z, color: Rgb(1, 2, 3) }],
            lines: vec![SceneLine { a: Vec3::ZERO, b: Vec3::Z, color: Rgb(4, 5, 6), width: 1.5 }],
            points: vec![ScenePoint { p: Vec3::X, color: Rgb(7, 8, 9), size: 4.0 }],
            ..Default::default()
        };
        let g = GpuScene::from_scene(&scene, 2.0);
        assert_eq!(g.tris.len(), 3 * TRI_SIZE);
        assert_eq!(g.lines.len(), 2 * LINE_SIZE);
        assert_eq!(&g.tris[24..28], &[1, 2, 3, 255]);
        // Widths are in physical pixels.
        assert_eq!(f32::from_le_bytes([g.lines[28], g.lines[29], g.lines[30], g.lines[31]]), 3.0);
        assert_eq!(f32::from_le_bytes([g.lines[60], g.lines[61], g.lines[62], g.lines[63]]), 8.0);
        let cb =
            ViewportCallback { key: 0, slot: SceneSlot::default(), view_proj: [[0.0; 4]; 4], back: [0.0; 3], eye: [0.0; 4], size_px: [1.0, 1.0] };
        assert_eq!(uniform_bytes(&cb, true).len() as u64, UNIFORM_SIZE);
    }
}
