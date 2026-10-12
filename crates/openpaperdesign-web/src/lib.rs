//! openpaperdesign-web: the upstream paper-rs editor and canvas renderer in a browser.
//!
//! Upstream modules are compiled straight from `../../src` via `#[path]`, with module paths
//! mirroring the upstream crate so their `use crate::...` lines resolve unchanged. Left out:
//! the egui/winit app shell (`app`, `ui`, `render::frame`), `session` (native clocks and
//! storage) and `mcp`. The OS clipboard is replaced by `openpaperdesign-clipboard-stub`
//! (imported as `arboard`). The browser glue lives in `state` (host-testable) and `web`
//! (wasm-bindgen, wasm32 only).

#[path = "../../../src/assets.rs"]
pub mod assets;
#[path = "../../../src/connector.rs"]
pub mod connector;
#[path = "../../../src/demo.rs"]
pub mod demo;
#[path = "../../../src/editor/mod.rs"]
pub mod editor;
#[path = "../../../src/geom.rs"]
pub mod geom;
#[path = "../../../src/history.rs"]
pub mod history;
#[path = "../../../src/ink.rs"]
pub mod ink;
#[path = "../../../src/io/mod.rs"]
pub mod io;
#[path = "../../../src/layout.rs"]
pub mod layout;
#[path = "../../../src/library.rs"]
pub mod library;
#[path = "../../../src/model/mod.rs"]
pub mod model;
#[path = "../../../src/project/mod.rs"]
pub mod project;
#[path = "../../../src/text.rs"]
pub mod text;
#[path = "../../../src/theme.rs"]
pub mod theme;
#[path = "../../../src/vector.rs"]
pub mod vector;

#[path = "../../../src/render"]
pub mod render {
    pub mod draw;
    pub mod gpu;
    pub mod mesh;
    pub mod rough;
    pub mod scene;
    pub mod tess;
}

pub mod health;
pub mod keys;
pub mod state;
#[cfg(target_arch = "wasm32")]
pub mod web;
