//! Distribution entrypoint. Reuse the native editor and CLI without a second UI.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod diagnostics;

#[allow(dead_code)] // The shared CLI's main is retained in its original source.
mod upstream {
    include!("../src/main.rs");

    pub fn launch(args: Vec<String>) -> Result<(), String> {
        run(args)
    }
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let gui = args.is_empty()
        || args.as_slice() == ["--demo"]
        || (args.len() == 1 && !args[0].starts_with('-'));
    attach_parent_console(!gui);
    // A new app must not silently adopt the upstream app's existing library. Explicit
    // PAPER_RS_HOME continues to work for operators and disposable verification.
    if std::env::var_os("PAPER_RS_HOME").is_none() {
        let Some(data) = dirs::data_local_dir() else {
            eprintln!(
                "Cannot find your local application-data folder. Set PAPER_RS_HOME to a chosen project library."
            );
            return std::process::ExitCode::FAILURE;
        };
        // Protected Documents access can block macOS before the native event loop is
        // running. The app's own local store avoids that first-window dependency.
        // Existing Documents projects are left in place; an explicit root is honoured.
        let root = data.join("OpenPaperDesign/projects");
        // The process is still single-threaded, before the editor/logger/runtime starts.
        unsafe { std::env::set_var("PAPER_RS_HOME", root) };
    }
    let diagnostic = diagnostics::start(gui);
    match upstream::launch(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            log::error!("Application stopped: {error}");
            eprintln!("error: {error}");
            if gui {
                let detail = diagnostic.map_or_else(
                    || "A local diagnostic log could not be created.".to_string(),
                    |path| format!("Details were saved locally in:\n{}", path.display()),
                );
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("OpenPaperDesign could not continue")
                    .set_description(format!("{error}\n\n{detail}"))
                    .show();
            }
            std::process::ExitCode::FAILURE
        }
    }
}

// Windows GUI builds have no automatic parent console. Reattach only for CLI calls;
// redirected stdout/stderr are already usable and must stay connected to their pipes.
#[cfg(target_os = "windows")]
fn attach_parent_console(cli: bool) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
    }
    if cli {
        unsafe {
            if GetStdHandle(-11i32 as u32).is_null() {
                AttachConsole(u32::MAX);
            }
        }
    }
}
#[cfg(not(target_os = "windows"))]
fn attach_parent_console(_: bool) {}
