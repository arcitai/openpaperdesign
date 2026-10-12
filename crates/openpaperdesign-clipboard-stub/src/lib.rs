//! The part of the [arboard](https://crates.io/crates/arboard) API that paper-rs uses, for
//! targets where arboard has no backend (wasm32). There is no system clipboard: `Clipboard::new`
//! always fails, so upstream `io::clipboard::System::new()` returns `None` and the editor
//! falls back to its internal clipboard (copy/paste within the page still works).

use std::borrow::Cow;

#[derive(Debug)]
pub struct Error;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no system clipboard on this target")
    }
}

impl std::error::Error for Error {}

pub struct ImageData<'a> {
    pub width: usize,
    pub height: usize,
    pub bytes: Cow<'a, [u8]>,
}

pub struct Clipboard(());

impl Clipboard {
    pub fn new() -> Result<Clipboard, Error> {
        Err(Error)
    }
    pub fn get_text(&mut self) -> Result<String, Error> {
        Err(Error)
    }
    pub fn set_text(&mut self, _text: String) -> Result<(), Error> {
        Err(Error)
    }
    pub fn get_image(&mut self) -> Result<ImageData<'static>, Error> {
        Err(Error)
    }
    pub fn set_image(&mut self, _image: ImageData) -> Result<(), Error> {
        Err(Error)
    }
}
