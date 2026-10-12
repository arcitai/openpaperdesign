//! `openpaperdesign-serve --project <dir> [--port N] [--web-dir <dir>]`
//!
//! Serves one existing paper-rs project folder to a browser on 127.0.0.1 only and prints the
//! URL to open. The URL fragment carries the per-launch API token; fragments are not sent to
//! the server or in `Referer`, and the page moves the token into an `Authorization` header.

use openpaperdesign_serve::{Config, Server, new_token, router};
use std::path::PathBuf;

const DEFAULT_PORT: u16 = 29981;
const USAGE: &str =
    "usage: openpaperdesign-serve --project <project dir> [--port N] [--web-dir <dir>]";

fn default_web_dir() -> PathBuf {
    // Where scripts/build-web.sh puts the bundle in this checkout.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/openpaperdesign-web")
}

fn parse() -> Result<(PathBuf, u16, PathBuf), String> {
    let mut project = None;
    let mut port = DEFAULT_PORT;
    let mut web_dir = default_web_dir();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--project" => project = Some(PathBuf::from(value()?)),
            "--port" => port = value()?.parse().map_err(|e| format!("--port: {e}"))?,
            "--web-dir" => web_dir = PathBuf::from(value()?),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    let project = project.ok_or(USAGE)?;
    Ok((project, port, web_dir))
}

#[tokio::main]
async fn main() {
    let (project, port, web_dir) = parse().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap_or_else(|e| {
            eprintln!("cannot listen on 127.0.0.1:{port}: {e}");
            std::process::exit(1);
        });
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let token = new_token().unwrap_or_else(|e| {
        eprintln!("no secure random source for the API token: {e}");
        std::process::exit(1);
    });
    let server = Server::open(Config {
        project,
        web_dir,
        port,
        token: token.clone(),
    })
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    println!("openpaperdesign-serve: editing {}", server.dir().display());
    if server.bundle_files() == 0 {
        eprintln!("warning: no web bundle found; build it with scripts/build-web.sh");
    }
    println!("open http://127.0.0.1:{port}/#token={token}");
    if let Err(e) = axum::serve(listener, router(server)).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
