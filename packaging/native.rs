//! Distribution entrypoint. Reuse the native editor and CLI without a second UI.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod upstream {
    include!("../src/main.rs");

    pub fn launch() -> std::process::ExitCode {
        main()
    }
}

fn main() -> std::process::ExitCode {
    // A new app must not silently adopt the upstream app's existing library. Explicit
    // PAPER_RS_HOME continues to work for operators and disposable verification.
    if std::env::var_os("PAPER_RS_HOME").is_none() {
        let Some(documents) =
            dirs::document_dir().or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        else {
            eprintln!(
                "Cannot find your home folder. Set PAPER_RS_HOME to a chosen project library."
            );
            return std::process::ExitCode::FAILURE;
        };
        let root = documents.join("OpenPaperDesign");
        // The process is still single-threaded, before the editor/logger/runtime starts.
        unsafe { std::env::set_var("PAPER_RS_HOME", root) };
    }
    upstream::launch()
}
