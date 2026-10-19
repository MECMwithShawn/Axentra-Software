//! Browser preview (Dioxus liveview): the same UI served over a websocket,
//! backed by an in-memory store with demo data. For development and
//! screenshots only. `AXENTRA_PREVIEW_PORT` overrides the port (default 8090).

use std::sync::Arc;

use axentra_core::Store;
use dioxus_liveview::LiveviewRouter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    axentra_core::seed::seed_demo(&store, chrono::Local::now()).expect("seed demo data");
    axentra_ui::init(store, None);
    let port: u16 = std::env::var("AXENTRA_PREVIEW_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8090);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    // The liveview catch-all route ("/{*route}") does not match "/" itself.
    let router = axum::Router::new()
        .route("/", axum::routing::get(|| async { axum::response::Redirect::temporary("/app") }))
        .with_app("/", axentra_ui::App);
    println!("Axentra preview on http://{addr}/app");
    router.start(addr).await;
}
