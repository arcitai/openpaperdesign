//! Browser glue: the JS-facing API. A `<canvas>` is rendered by upstream
//! `render::gpu::CanvasRenderer`; everything else (input, redraw decisions, saving) lives in
//! [`crate::state::WebState`], which is plain Rust and tested on the host.
//!
//! Time comes from the page (`performance.now()` / `event.timeStamp`), never from
//! `std::time`, which panics on wasm32-unknown-unknown. Saving is pulled by the page:
//! [`WebEditor::save_snapshot`] hands out the page file to PUT, and the server's revision
//! check decides whether it lands.

use crate::editor::Button;
use crate::editor::shortcuts::Action;
use crate::geom::V2;
use crate::health::{self, Acquired, Health, Step};
use crate::render::gpu::CanvasRenderer;
use crate::render::scene::{self, SceneCache};
use crate::state::WebState;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: CanvasRenderer,
    scale: f32,
}

#[wasm_bindgen]
pub struct WebEditor {
    state: WebState,
    cache: SceneCache,
    gpu: Gpu,
    backend: String,
    /// Presented frames and the first terminal GPU problem (shared with wgpu's callbacks).
    health: Arc<Mutex<Health>>,
}

fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&format!("openpaperdesign-web panicked: {info}").into());
        }));
    });
}

/// Open `page_json` (an upstream page file, listed in the manifest as `page_id`/`page_name`)
/// on `canvas`.
///
/// `seed` comes from `crypto.getRandomValues`: fastrand has no entropy source on
/// wasm32-unknown-unknown and would otherwise start from a fixed seed, so ids made with
/// `project::new_id` (pages) could repeat across sessions.
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub async fn start(
    canvas: web_sys::HtmlCanvasElement,
    page_json: String,
    page_id: String,
    page_name: String,
    seed_hi: u32,
    seed_lo: u32,
    mac: bool,
    force_webgl: bool,
    css_width: f64,
    css_height: f64,
    dpr: f64,
) -> Result<WebEditor, JsError> {
    install_panic_hook();
    fastrand::seed(((seed_hi as u64) << 32) | seed_lo as u64);
    let state = WebState::open(&page_json, page_id, page_name, mac, css_width, css_height)
        .map_err(js_err)?;

    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = if force_webgl {
        wgpu::Backends::GL
    } else {
        wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL
    };
    let instance = wgpu::util::new_instance_with_webgpu_detection(desc).await;
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(js_err)?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|e| js_err(format!("no GPU adapter: {e}")))?;
    let info = adapter.get_info();
    let backend = format!(
        "{:?} / {} ({:?})",
        info.backend, info.name, info.device_type
    );
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("openpaperdesign-web"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
        .map_err(|e| js_err(format!("failed to create device: {e}")))?;
    // GPU errors outside the startup scope below, and device loss: logged and recorded, so
    // drawing stops and the page shows an error instead of a silently broken canvas.
    let health = Arc::new(Mutex::new(Health::default()));
    let sink = health.clone();
    device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        web_sys::console::error_1(&format!("wgpu: {e}").into());
        sink.lock().unwrap().fail(format!("GPU error: {e}"));
    }));
    let sink = health.clone();
    device.set_device_lost_callback(move |reason, msg| {
        let msg = format!("GPU device lost ({reason:?}): {msg}");
        // `Destroyed` is our own drop (e.g. the WebGPU editor discarded for a fallback).
        if reason != wgpu::DeviceLostReason::Destroyed {
            web_sys::console::error_1(&msg.clone().into());
        }
        sink.lock().unwrap().fail(msg);
    });
    // Shader, pipelines, surface setup and the first frame must validate before the page may
    // say it is ready (browser WebGPU validates WGSL more strictly than native wgpu).
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let caps = surface.get_capabilities(&adapter);
    // Like the native app: blend in gamma space, so prefer a non-sRGB target.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .or_else(|| caps.formats.first().copied())
        .ok_or_else(|| js_err("surface reports no formats"))?;
    let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        caps.alpha_modes[0]
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: 1,
        height: 1,
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
        color_space: Default::default(),
    };
    let renderer = CanvasRenderer::new(&device, format);
    let gpu = Gpu {
        surface,
        device,
        queue,
        config,
        renderer,
        scale: 1.0,
    };

    let mut w = WebEditor {
        state,
        cache: SceneCache::default(),
        gpu,
        backend,
        health,
    };
    w.resize(css_width, css_height, dpr);
    w.frame(0.0);
    if let Some(e) = scope.pop().await {
        let msg = format!("GPU validation failed on {}: {e}", w.backend);
        web_sys::console::error_1(&msg.clone().into());
        return Err(JsError::new(&msg));
    }
    // A problem recorded during startup (device lost, terminal surface error) fails startup
    // too. A first frame that was merely skipped (hidden tab) is retried; `ready()` stays false
    // until a frame has been presented.
    if let Some(e) = w.health.lock().unwrap().error() {
        return Err(JsError::new(&format!("{} on {}", e, w.backend)));
    }
    Ok(w)
}

fn button(b: u8) -> Option<Button> {
    // DOM `MouseEvent.button`: 0 primary, 1 middle (wheel), 2 secondary.
    match b {
        0 => Some(Button::Primary),
        1 => Some(Button::Middle),
        2 => Some(Button::Secondary),
        _ => None,
    }
}

#[wasm_bindgen]
impl WebEditor {
    /// "Backend / adapter name (device type)" of the adapter that was created.
    pub fn backend(&self) -> String {
        self.backend.clone()
    }

    /// The first terminal GPU problem (error, device loss, lost surface); drawing stops then.
    pub fn gpu_error(&self) -> Option<String> {
        self.health.lock().unwrap().error()
    }

    /// GPU objects validated, a frame was presented, and no GPU problem since.
    pub fn ready(&self) -> bool {
        self.health.lock().unwrap().ready()
    }

    pub fn resize(&mut self, css_width: f64, css_height: f64, dpr: f64) {
        let g = &mut self.gpu;
        let max = g.device.limits().max_texture_dimension_2d;
        let w = ((css_width * dpr).round() as u32).clamp(1, max);
        let h = ((css_height * dpr).round() as u32).clamp(1, max);
        g.scale = dpr as f32;
        if (g.config.width, g.config.height) != (w, h) {
            g.config.width = w;
            g.config.height = h;
            g.surface.configure(&g.device, &g.config);
        }
        self.state.resize(css_width, css_height);
    }

    /// Advance animations and draw if anything changed. Call once per animation frame.
    pub fn frame(&mut self, now_ms: f64) {
        if !self.state.tick(now_ms) || !self.health.lock().unwrap().can_draw() {
            return;
        }
        let g = &mut self.gpu;
        use wgpu::CurrentSurfaceTexture as C;
        let (tex, acquired) = match g.surface.get_current_texture() {
            C::Success(t) => (Some(t), Acquired::Success),
            C::Suboptimal(t) => (Some(t), Acquired::Suboptimal),
            C::Timeout => (None, Acquired::Timeout),
            C::Occluded => (None, Acquired::Occluded),
            C::Outdated => (None, Acquired::Outdated),
            C::Lost => (None, Acquired::Lost),
            C::Validation => (None, Acquired::Validation),
        };
        // Skipped or failed frames leave the redraw pending (no `drawn()`).
        let reconfigure_after = match (health::step(acquired), tex) {
            (Step::Draw { reconfigure_after }, Some(t)) => {
                self.draw(t);
                reconfigure_after
            }
            (Step::Reconfigure, _) => true,
            (Step::Fail(msg), _) => {
                web_sys::console::error_1(&msg.clone().into());
                self.health.lock().unwrap().fail(msg);
                false
            }
            _ => false,
        };
        if reconfigure_after {
            let g = &mut self.gpu;
            g.surface.configure(&g.device, &g.config);
        }
    }

    fn draw(&mut self, tex: wgpu::SurfaceTexture) {
        let g = &mut self.gpu;
        let view = tex.texture.create_view(&Default::default());
        let frame = scene::canvas(&self.state.ed, &mut self.cache, V2::ZERO, true);
        let bg = scene::CANVAS_BG.to_f32(1.0);
        let mut enc = g.device.create_command_encoder(&Default::default());
        g.renderer.render(
            &g.device,
            &g.queue,
            &mut enc,
            &view,
            [g.config.width, g.config.height],
            g.scale,
            &frame,
            [bg[0] as f64, bg[1] as f64, bg[2] as f64, 1.0],
        );
        g.queue.submit([enc.finish()]);
        g.queue.present(tex);
        self.state.drawn();
        self.health.lock().unwrap().presented();
    }

    pub fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool) {
        self.state.modifiers(shift, ctrl, alt, meta);
    }

    pub fn pointer_down(&mut self, x: f64, y: f64, dom_button: u8, time_ms: f64) {
        if let Some(b) = button(dom_button) {
            self.state.pointer_down(x, y, b, time_ms);
        }
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, time_ms: f64) {
        self.state.pointer_move(x, y, time_ms);
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, dom_button: u8, time_ms: f64) {
        if let Some(b) = button(dom_button) {
            self.state.pointer_up(x, y, b, time_ms);
        }
    }

    /// DOM wheel delta in CSS px (positive y = scroll down).
    pub fn wheel(&mut self, dx: f64, dy: f64, x: f64, y: f64) {
        self.state.wheel(dx, dy, x, y);
    }

    /// Returns whether the editor took the key (the page then prevents the browser default).
    pub fn key_down(&mut self, key: &str, code: &str, repeat: bool) -> bool {
        self.state.key_down(key, code, repeat)
    }

    pub fn key_up(&mut self, key: &str, code: &str) {
        self.state.key_up(key, code);
    }

    /// Toolbar: "select", "hand", "rect", "ellipse".
    pub fn set_tool(&mut self, name: &str) {
        self.state.set_tool(name);
    }

    pub fn tool(&self) -> String {
        self.state.tool()
    }

    pub fn undo(&mut self) {
        self.state.run(Action::Undo);
    }

    pub fn redo(&mut self) {
        self.state.run(Action::Redo);
    }

    pub fn delete(&mut self) {
        self.state.run(Action::Delete);
    }

    pub fn selection_len(&self) -> usize {
        self.state.ed.selection.len()
    }

    pub fn node_count(&self) -> usize {
        self.state.ed.doc.nodes.len()
    }

    pub fn status(&self) -> String {
        self.state.ed.status.clone()
    }

    /// Top-level nodes as JSON (see [`WebState::root_nodes`]).
    pub fn root_nodes(&self) -> String {
        self.state.root_nodes()
    }

    pub fn dirty(&self) -> bool {
        self.state.dirty()
    }

    pub fn save_snapshot(&mut self) -> Option<String> {
        self.state.save_snapshot()
    }

    pub fn save_succeeded(&mut self) {
        self.state.save_succeeded();
    }

    pub fn save_failed(&mut self) {
        self.state.save_failed();
    }
}
