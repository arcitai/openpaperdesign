//! openpaperdesign-core: the portable part of paper-rs, compiled straight from the upstream
//! sources in `../../src` so a wasm32 build can be checked without editing them.
//!
//! Module paths mirror the upstream crate (`crate::geom`, `crate::render::draw`, ...) so the
//! upstream `use crate::...` lines resolve unchanged. Only modules with no window, editor,
//! UI, MCP or GPU dependency are included; see docs/openpaperdesign/ for what is left out.

#[path = "../../../src/assets.rs"]
pub mod assets;
#[path = "../../../src/connector.rs"]
pub mod connector;
#[path = "../../../src/geom.rs"]
pub mod geom;
#[path = "../../../src/history.rs"]
pub mod history;
#[path = "../../../src/ink.rs"]
pub mod ink;
#[path = "../../../src/layout.rs"]
pub mod layout;
#[path = "../../../src/model/mod.rs"]
pub mod model;
#[path = "../../../src/text.rs"]
pub mod text;
#[path = "../../../src/theme.rs"]
pub mod theme;
#[path = "../../../src/vector.rs"]
pub mod vector;

#[path = "../../../src/io"]
pub mod io {
    pub mod atomic;
}

#[path = "../../../src/render"]
pub mod render {
    pub mod draw;
    pub mod mesh;
    pub mod rough;
    pub mod tess;
}
