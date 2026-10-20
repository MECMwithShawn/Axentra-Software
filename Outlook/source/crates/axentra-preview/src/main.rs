//! Browser preview (Dioxus liveview): the same UI served over a websocket,
//! backed by an in-memory store with demo data. For development and
//! screenshots only. `AXENTRA_PREVIEW_PORT` overrides the port (default 8090).
//!
//! `AXENTRA_FAKE_WINDOW_CONTROLS=1` draws the desktop title bar (window
//! controls, drag regions, resize handles, rounded frame) with no-op
//! controls, to check its layout here; set it to a GNOME `button-layout`
//! string instead (e.g. `close,minimize,maximize:`) to try another layout.
//! Maximize / Restore and double-clicks on the header toggle a fake
//! maximised state; the other actions are only logged.
//! `AXENTRA_FAKE_WINDOW_TILED` (`top bottom left`, `right`, `all`, ...)
//! starts the fake window tiled on those edges.

use std::sync::Arc;

use axentra_core::Store;
use axentra_ui::shell::window::{ButtonLayout, ResizeEdge, TiledEdges, TitlebarPrefs, WindowControls};
use dioxus::prelude::*;
use dioxus_liveview::LiveviewRouter;

/// The fake window controls' layout (`AXENTRA_FAKE_WINDOW_CONTROLS`).
fn fake_layout() -> Option<ButtonLayout> {
    let v = std::env::var("AXENTRA_FAKE_WINDOW_CONTROLS").ok()?;
    match v.trim() {
        "" | "0" => None,
        "1" => Some(ButtonLayout::default()),
        layout => Some(ButtonLayout::parse(layout)),
    }
}

/// Tiled edges for the fake window (`AXENTRA_FAKE_WINDOW_TILED`, e.g.
/// `top bottom left` for GNOME's left half, or `all`).
fn fake_tiled() -> TiledEdges {
    let v = std::env::var("AXENTRA_FAKE_WINDOW_TILED").unwrap_or_default();
    if v.trim() == "all" {
        return TiledEdges::ALL;
    }
    let has = |edge: &str| v.split([' ', ',']).any(|w| w.trim() == edge);
    TiledEdges { top: has("top"), right: has("right"), bottom: has("bottom"), left: has("left") }
}

/// The app with no-op window controls (dev only).
#[allow(non_snake_case)]
fn FakeWindowRoot() -> Element {
    let prefs = try_use_context::<TitlebarPrefs>().unwrap_or_default();
    let mut maximized = use_signal(|| false);
    let tiled = use_signal(fake_tiled);
    use_context_provider(move || {
        let log = |what: &'static str| Callback::new(move |()| println!("[fake window] {what}"));
        WindowControls {
            minimize: log("minimize"),
            toggle_maximize: Callback::new(move |()| {
                let now = !maximized();
                println!("[fake window] maximized = {now}");
                maximized.set(now);
            }),
            close: log("close"),
            start_drag: log("start drag"),
            start_resize: Callback::new(|edge: ResizeEdge| println!("[fake window] start resize {edge:?}")),
            is_maximized: maximized,
            tiled,
            layout: prefs.layout,
            double_click: prefs.double_click,
            middle_click: prefs.middle_click,
        }
    });
    rsx! {
        axentra_ui::App {}
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
    axentra_core::seed::seed_demo(&store, chrono::Local::now()).expect("seed demo data");
    axentra_ui::init(store, None);
    // Backups: the password stays in memory, and AXENTRA_BACKUP_DIR (else a
    // temp folder) stands in for the OneDrive folder; the real one is never used.
    axentra_ui::settings::backup::preview_mode();
    let port: u16 = std::env::var("AXENTRA_PREVIEW_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8090);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    // The liveview catch-all route ("/{*route}") does not match "/" itself.
    let router = axum::Router::new().route("/", axum::routing::get(|| async { axum::response::Redirect::temporary("/app") }));
    let router = match fake_layout() {
        Some(layout) => {
            let prefs = TitlebarPrefs { layout, ..TitlebarPrefs::default() };
            router.with_virtual_dom("/", move || VirtualDom::new(FakeWindowRoot).with_root_context(prefs))
        }
        None => router.with_app("/", axentra_ui::App),
    };
    println!("Axentra preview on http://{addr}/app");
    router.start(addr).await;
}
