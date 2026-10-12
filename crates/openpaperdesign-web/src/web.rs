//! Browser glue: a `<canvas>` rendered by upstream `render::gpu::CanvasRenderer`, driven by
//! upstream `editor::Editor` through DOM events forwarded from the page's JavaScript.
//!
//! Time comes from the page (`performance.now()` / `event.timeStamp`), never from
//! `std::time`, which panics on wasm32-unknown-unknown. Saving is pulled by the page:
//! [`WebEditor::save_snapshot`] hands out the page file to PUT, and the server's revision
//! check decides whether it lands.

use crate::editor::shortcuts::Action;
use crate::editor::{Button, Editor, InputEvent, Tool};
use crate::geom::{V2, v2};
use crate::keys;
use crate::project::format::{page_from_json, page_to_json};
use crate::render::gpu::CanvasRenderer;
use crate::render::scene::{self, SceneCache};
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
    ed: Editor,
    cache: SceneCache,
    gpu: Gpu,
    mac: bool,
    backend: String,
    /// `doc_stamp` of the content last confirmed written by the server.
    saved_stamp: u64,
    /// `doc_stamp` of the snapshot being written, if any.
    inflight: Option<u64>,
    drawn_revision: Option<u64>,
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

    let mut page = page_from_json(&page_json).map_err(js_err)?;
    // The manifest owns page ids and names (a migrated legacy file gets a fresh random id).
    page.id = page_id;
    page.name = page_name.clone();
    page.doc.name = page_name;

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
    device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| {
        web_sys::console::error_1(&format!("wgpu: {e}").into());
    }));
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

    let mut ed = Editor::default();
    ed.viewport = v2(css_width.max(1.0), css_height.max(1.0));
    ed.open_pages(vec![page], None);
    let mut w = WebEditor {
        saved_stamp: ed.doc_stamp,
        ed,
        cache: SceneCache::default(),
        gpu,
        mac,
        backend,
        inflight: None,
        drawn_revision: None,
    };
    w.resize(css_width, css_height, dpr);
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
        self.ed.viewport = v2(css_width.max(1.0), css_height.max(1.0));
        if self.ed.pending_fit && css_width > 1.0 {
            self.ed.pending_fit = false;
            self.ed.zoom_to_fit();
        }
        self.drawn_revision = None;
    }

    /// Advance animations and draw if anything changed. Call once per animation frame.
    pub fn frame(&mut self, now_ms: f64) {
        self.ed.tick(now_ms);
        // Requests for app chrome (export, open, palette, rename...) have no handler in this
        // slice; drop them instead of letting them pile up.
        self.ed.requests.clear();
        self.ed.ui_events.clear();
        let animating = self.ed.animating(now_ms);
        if !animating && self.drawn_revision == Some(self.ed.revision) {
            return;
        }
        let g = &mut self.gpu;
        let tex = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                g.surface.configure(&g.device, &g.config);
                return;
            }
        };
        let view = tex.texture.create_view(&Default::default());
        let frame = scene::canvas(&self.ed, &mut self.cache, V2::ZERO, true);
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
        self.drawn_revision = Some(self.ed.revision);
    }

    pub fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool) {
        let m = keys::modifiers(self.mac, shift, ctrl, alt, meta);
        if m != self.ed.mods {
            self.ed.handle(InputEvent::Modifiers(m));
        }
    }

    pub fn pointer_down(&mut self, x: f64, y: f64, dom_button: u8, time_ms: f64) {
        if let Some(b) = button(dom_button) {
            self.ed.handle(InputEvent::PointerDown {
                pos: v2(x, y),
                button: b,
                pressure: None,
                time_ms,
            });
        }
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, time_ms: f64) {
        self.ed.handle(InputEvent::PointerMove {
            pos: v2(x, y),
            pressure: None,
            time_ms,
        });
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, dom_button: u8, time_ms: f64) {
        if let Some(b) = button(dom_button) {
            self.ed.handle(InputEvent::PointerUp {
                pos: v2(x, y),
                button: b,
                time_ms,
            });
        }
    }

    /// DOM wheel delta in CSS px (positive y = scroll down). With Ctrl/⌘ held (which is how
    /// browsers report a trackpad pinch) upstream zooms instead of panning.
    pub fn wheel(&mut self, dx: f64, dy: f64, x: f64, y: f64) {
        let d = if self.ed.mods.shift && dx == 0.0 {
            v2(dy, 0.0)
        } else {
            v2(dx, dy)
        };
        self.ed.handle(InputEvent::Scroll {
            delta: d,
            pos: v2(x, y),
        });
    }

    /// Returns whether the key was used (the page then prevents the browser default).
    pub fn key_down(&mut self, key: &str, code: &str, repeat: bool) -> bool {
        let editing = self.ed.text_edit.is_some();
        let Some(k) = keys::key(self.ed.mods, editing, key, code) else {
            return false;
        };
        let before = (self.ed.revision, self.ed.doc_stamp, self.ed.tool);
        self.ed.handle(InputEvent::KeyDown { key: k, repeat });
        if editing && !self.ed.mods.command() && key.chars().count() == 1 {
            self.ed.handle(InputEvent::Text(key.to_string()));
        }
        editing || before != (self.ed.revision, self.ed.doc_stamp, self.ed.tool)
    }

    pub fn key_up(&mut self, key: &str, code: &str) {
        if let Some(k) = keys::key(self.ed.mods, self.ed.text_edit.is_some(), key, code) {
            self.ed.handle(InputEvent::KeyUp { key: k });
        }
    }

    /// Toolbar: "select", "hand", "rect", "ellipse", "text".
    pub fn set_tool(&mut self, name: &str) {
        let tool = match name {
            "select" => Tool::Select,
            "hand" => Tool::Hand,
            "rect" => Tool::Rect,
            "ellipse" => Tool::Ellipse,
            "text" => Tool::Text,
            _ => return,
        };
        self.ed.run(Action::Tool(tool));
    }

    pub fn tool(&self) -> String {
        format!("{:?}", self.ed.tool).to_lowercase()
    }

    pub fn undo(&mut self) {
        self.ed.run(Action::Undo);
    }

    pub fn redo(&mut self) {
        self.ed.run(Action::Redo);
    }

    pub fn delete(&mut self) {
        self.ed.run(Action::Delete);
    }

    pub fn selection_len(&self) -> usize {
        self.ed.selection.len()
    }

    pub fn node_count(&self) -> usize {
        self.ed.doc.nodes.len()
    }

    /// Top-level nodes as JSON `[{id, name, kind, children, world: [x,y,w,h], screen: [x,y,w,h]}]`
    /// (screen = CSS px in the canvas), for status displays and automated checks.
    pub fn root_nodes(&self) -> String {
        let doc = &self.ed.doc;
        let list: Vec<_> = doc
            .root
            .iter()
            .filter_map(|&id| {
                let n = doc.get(id)?;
                let w = doc.world_rect(id)?;
                let s = self.ed.camera.world_rect_to_screen(&w);
                let kind = serde_json::to_value(&n.kind)
                    .ok()
                    .and_then(|v| v.get("type").cloned())
                    .unwrap_or_default();
                Some(serde_json::json!({
                    "id": id.0,
                    "name": n.name,
                    "kind": kind,
                    "children": n.children.len(),
                    "world": [w.x, w.y, w.w, w.h],
                    "screen": [s.x, s.y, s.w, s.h],
                }))
            })
            .collect();
        serde_json::Value::from(list).to_string()
    }

    pub fn status(&self) -> String {
        self.ed.status.clone()
    }

    /// Content changed since the last confirmed save (or a save is in flight).
    pub fn dirty(&self) -> bool {
        self.ed.doc_stamp != self.saved_stamp
    }

    /// The page file to write now, or `None` when there is nothing to save, a save is
    /// already in flight, or a gesture/text edit still owns the open transaction.
    pub fn save_snapshot(&mut self) -> Option<String> {
        if self.inflight.is_some() || self.ed.is_busy() || self.ed.text_edit.is_some() {
            return None;
        }
        self.ed.commit_pending();
        if self.ed.doc_stamp == self.saved_stamp {
            return None;
        }
        self.inflight = Some(self.ed.doc_stamp);
        Some(page_to_json(&self.ed.page_data(self.ed.active_page)))
    }

    /// The server stored the last snapshot.
    pub fn save_succeeded(&mut self) {
        if let Some(s) = self.inflight.take() {
            self.saved_stamp = s;
        }
    }

    /// The last snapshot was not stored (conflict or network error); it stays dirty.
    pub fn save_failed(&mut self) {
        self.inflight = None;
    }
}
