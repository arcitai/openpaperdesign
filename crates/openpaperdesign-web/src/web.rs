//! Browser glue: the JS-facing API. A `<canvas>` shows upstream's whole workspace, painted by
//! upstream `render::frame::Painter` (the egui UI and the canvas renderer on one wgpu device,
//! as in the native window); everything else (input, layout, requests, redraw decisions,
//! saving) lives in [`crate::state::WebState`], which is plain Rust and tested on the host.
//!
//! Time comes from the page (`performance.now()` / `event.timeStamp`), never from
//! `std::time`, which panics on wasm32-unknown-unknown. Saving is pulled by the page:
//! [`WebEditor::save_snapshot`] hands out the canvas save to send (page list, active page and
//! each changed page), and the server's canvas revision check decides whether it lands.

use crate::editor::Button;
use crate::health::{self, Acquired, Health, Step};
use crate::render::frame::Painter;
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
    painter: Painter,
}

#[wasm_bindgen]
pub struct WebEditor {
    state: WebState,
    gpu: Gpu,
    backend: String,
    /// Presented frames and the first terminal GPU problem (shared with wgpu's callbacks).
    health: Arc<Mutex<Health>>,
}

/// An export for the page to offer as a download.
#[wasm_bindgen]
pub struct WebDownload {
    name: String,
    mime: String,
    bytes: Vec<u8>,
}

#[wasm_bindgen]
impl WebDownload {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn mime(&self) -> String {
        self.mime.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&format!("openpaperdesign-web panicked: {info}").into());
        }));
    });
}

/// Open `canvas_json` (the whole canvas as the server sends it: every page, the page list and
/// the active page, see `WebState::open`) on `canvas`, in the page's appearance (`dark`, see
/// `WebState::set_dark`) from the first frame on.
///
/// `seed` comes from `crypto.getRandomValues`: fastrand has no entropy source on
/// wasm32-unknown-unknown and would otherwise start from a fixed seed, so ids made with
/// `project::new_id` (new and duplicated pages) could repeat across sessions.
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub async fn start(
    canvas: web_sys::HtmlCanvasElement,
    canvas_json: String,
    seed_hi: u32,
    seed_lo: u32,
    mac: bool,
    force_webgl: bool,
    css_width: f64,
    css_height: f64,
    dpr: f64,
    dark: bool,
) -> Result<WebEditor, JsError> {
    install_panic_hook();
    fastrand::seed(((seed_hi as u64) << 32) | seed_lo as u64);
    let mut state = WebState::open(&canvas_json, mac, css_width, css_height).map_err(js_err)?;
    state.set_dark(dark);

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
    // Shaders (the canvas's and egui's), pipelines, surface setup and the first frame must
    // validate before the page may say it is ready (browser WebGPU validates WGSL more strictly
    // than native wgpu).
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
    let painter = Painter::new(&device, format);
    let gpu = Gpu {
        surface,
        device,
        queue,
        config,
        painter,
    };

    let mut w = WebEditor {
        state,
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
        if (g.config.width, g.config.height) != (w, h) {
            g.config.width = w;
            g.config.height = h;
            g.surface.configure(&g.device, &g.config);
        }
        self.state.resize(css_width, css_height, dpr);
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
        // Skipped or failed frames leave the redraw (and egui's input) pending: no layout and
        // no `drawn()`.
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
        let layout = self.state.layout();
        let g = &mut self.gpu;
        let view = tex.texture.create_view(&Default::default());
        g.painter.paint(
            &g.device,
            &g.queue,
            &view,
            [g.config.width, g.config.height],
            &self.state.ctx,
            layout,
            &self.state.ed,
            !self.state.ui.show_grid_off,
        );
        g.queue.present(tex);
        self.state.drawn();
        self.health.lock().unwrap().presented();
    }

    /// The page's appearance changed (see `WebState::set_dark`); drawn on the next frame.
    pub fn set_dark(&mut self, dark: bool) {
        self.state.set_dark(dark);
    }

    pub fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool) {
        self.state.modifiers(shift, ctrl, alt, meta);
    }

    /// The canvas element gained or lost keyboard focus.
    pub fn focus(&mut self, focused: bool) {
        self.state.set_focused(focused);
    }

    /// Positions are CSS px in the canvas element.
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

    pub fn pointer_leave(&mut self) {
        self.state.pointer_leave();
    }

    /// DOM wheel delta in CSS px (positive y = scroll down).
    pub fn wheel(&mut self, dx: f64, dy: f64, x: f64, y: f64) {
        self.state.wheel(dx, dy, x, y);
    }

    /// Returns whether the workspace took the key (the page then prevents the browser default).
    pub fn key_down(&mut self, key: &str, code: &str, repeat: bool) -> bool {
        self.state.key_down(key, code, repeat)
    }

    pub fn key_up(&mut self, key: &str, code: &str) {
        self.state.key_up(key, code);
    }

    /// A `paste` event's plain text.
    pub fn paste(&mut self, text: &str) {
        self.state.paste(text);
    }

    /// A `copy` or `cut` event: the text for its clipboard data, if the editor copied any.
    pub fn copy(&mut self, cut: bool) -> Option<String> {
        self.state.copy(cut)
    }

    /// Text to write to the clipboard outside a copy event (best effort, by the page).
    pub fn take_clipboard_text(&mut self) -> Option<String> {
        self.state.take_clipboard_text()
    }

    /// The CSS cursor to show.
    pub fn cursor(&self) -> String {
        self.state.cursor().to_string()
    }

    /// The next host request ("save" or "home"), if any.
    pub fn take_request(&mut self) -> Option<String> {
        self.state.take_request().map(|r| r.name().to_string())
    }

    /// The next export to offer as a download, if any.
    pub fn take_download(&mut self) -> Option<WebDownload> {
        self.state.take_download().map(|d| WebDownload {
            name: d.name,
            mime: d.mime.to_string(),
            bytes: d.bytes,
        })
    }

    pub fn tool(&self) -> String {
        self.state.tool()
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

    /// The canvas rectangle inside the workspace, `[x, y, w, h]` in CSS px (for checks).
    pub fn canvas_rect(&self) -> Vec<f64> {
        let c = self.state.canvas_rect();
        vec![
            c.min.x as f64,
            c.min.y as f64,
            c.width() as f64,
            c.height() as f64,
        ]
    }

    pub fn dirty(&self) -> bool {
        self.state.dirty()
    }

    /// Unsaved content or an unfinished UI edit (see [`WebState::unsaved_work`]).
    pub fn unsaved_work(&self) -> bool {
        self.state.unsaved_work()
    }

    pub fn save_snapshot(&mut self) -> Option<String> {
        self.state.save_snapshot()
    }

    pub fn save_succeeded(&mut self) {
        self.state.save_succeeded();
    }

    /// See [`WebState::can_reload`].
    pub fn can_reload(&self) -> bool {
        self.state.can_reload()
    }

    /// Load the canvas as the server has it now, in place (see [`WebState::reload`]).
    pub fn reload(&mut self, canvas_json: &str) -> Result<(), JsError> {
        self.state.reload(canvas_json).map_err(js_err)
    }

    pub fn save_failed(&mut self) {
        self.state.save_failed();
    }
}
