//! The GPU viewport in the window: an `egui_wgpu` paint callback drawing through
//! [`fieldcraft_gpu::view`]'s pipelines. Geometry is uploaded when the scene changes; each frame
//! only the camera uniform changes. The setup's symbols are a second scene, uploaded on their
//! own, so zooming (which resizes them) never uploads the mesh again; the selection highlight is
//! a third, small scene (the overlay), so a hover change uploads only the overlay. Device errors
//! (out of memory, validation) go to [`GpuErrors`] and the status bar instead of wgpu's default
//! handler, which panics.
//!
//! Adapted from SolidCraft's `crates/ui-egui/src/gpu.rs` (MIT OR Apache-2.0), see
//! ATTRIBUTION.md.

use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;
pub use fieldcraft_gpu::view::{Frame, GpuErrors, GpuScene, GpuTarget};
use fieldcraft_gpu::view::{Layer, Pipelines};

/// Longest device error message kept for the status bar.
const MAX_ERROR_LEN: usize = 500;

/// Where the UI hands a new scene to the renderer (taken on the next frame's `prepare`).
pub type SceneSlot = Arc<Mutex<Option<GpuScene>>>;

/// Per-frame parameters.
pub struct ViewportCallback {
    /// Changes whenever the scene does; the slot holds the scene for a new key.
    pub key: u64,
    pub slot: SceneSlot,
    /// The same for the setup's symbols.
    pub symbols_key: u64,
    pub symbols_slot: SceneSlot,
    /// The same for the overlay (the selection highlight).
    pub overlay_key: u64,
    pub overlay_slot: SceneSlot,
    pub frame: Frame,
}

/// An uploaded layer and the key it was uploaded for.
#[derive(Default)]
struct Keyed {
    key: Option<u64>,
    layer: Layer,
}

impl Keyed {
    fn take(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: u64, slot: &SceneSlot) {
        if self.key == Some(key) {
            return;
        }
        if let Some(sc) = slot.lock().ok().and_then(|mut s| s.take()) {
            self.layer.upload(device, queue, &sc);
            self.key = Some(key);
        }
    }
}

struct Resources {
    pipelines: Pipelines,
    scene: Keyed,
    symbols: Keyed,
    overlay: Keyed,
}

/// Create the pipelines. Call once with the app's render state and its depth and MSAA settings.
/// Device errors are logged and kept in `errors` for the status bar (`ctx` is repainted so it
/// shows), replacing wgpu's default handler, which panics.
pub fn install(rs: &egui_wgpu::RenderState, depth_bits: u8, samples: u32, errors: GpuErrors, ctx: egui::Context) -> GpuTarget {
    rs.device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        let text: String = e.to_string().chars().take(MAX_ERROR_LEN).collect();
        log::error!("graphics device error: {text}");
        if let Ok(mut slot) = errors.lock() {
            *slot = Some(text);
        }
        ctx.request_repaint();
    }));
    let t = GpuTarget { format: rs.target_format, depth: egui_wgpu::depth_format_from_bits(depth_bits, 0), samples };
    let res = Resources { pipelines: Pipelines::new(&rs.device, t), scene: Keyed::default(), symbols: Keyed::default(), overlay: Keyed::default() };
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
        res.scene.take(device, queue, self.key, &self.slot);
        res.symbols.take(device, queue, self.symbols_key, &self.symbols_slot);
        res.overlay.take(device, queue, self.overlay_key, &self.overlay_slot);
        res.pipelines.write_frame(queue, &self.frame);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(res) = resources.get::<Resources>() else { return };
        let vp = info.viewport_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 {
            return;
        }
        pass.set_viewport(vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32, 0.0, 1.0);
        res.pipelines.paint(pass, &res.scene.layer, &res.symbols.layer, &res.overlay.layer);
    }
}
