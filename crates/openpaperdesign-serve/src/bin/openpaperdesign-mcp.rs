//! Stdio entrypoint; the launcher is shared with its lifecycle integration tests.

#[tokio::main]
async fn main() {
    openpaperdesign_serve::launcher::main().await;
}
