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
//! `AXENTRA_FAKE_ACCOUNT_PHOTO=1` gives the demo account a drawn, clearly
//! synthetic profile photo, stored as if the sync engine had fetched it from
//! Microsoft, to check the account avatars (header, Account Manager,
//! Settings > Accounts).

use std::sync::Arc;

use axentra_core::{AccountPhoto, Store};
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

/// The fake account photo: an abstract drawing, never a real person.
const DEMO_PHOTO_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 96 96'>\
<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='#7f5af0'/><stop offset='1' stop-color='#2cb1bc'/></linearGradient></defs>\
<rect width='96' height='96' fill='url(#g)'/>\
<circle cx='74' cy='22' r='11' fill='#ffd166'/>\
<path d='M0 70 Q24 56 48 66 T96 62 V96 H0 Z' fill='#1b4965' opacity='0.5'/>\
<circle cx='48' cy='41' r='15' fill='#fde2c8'/>\
<path d='M19 96 C21 73 33 62 48 62 C63 62 75 73 77 96 Z' fill='#f4f1de'/>\
</svg>";

/// `data:image/svg+xml,…` with everything but plain characters percent-encoded.
fn svg_data_url(svg: &str) -> String {
    let mut url = String::from("data:image/svg+xml,");
    for b in svg.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/:=,;'()!*".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// `AXENTRA_FAKE_ACCOUNT_PHOTO=1`: store [`DEMO_PHOTO_SVG`] for the demo account.
fn fake_account_photo(store: &Store) {
    if !std::env::var("AXENTRA_FAKE_ACCOUNT_PHOTO").is_ok_and(|v| !matches!(v.trim(), "" | "0")) {
        return;
    }
    let Some(account) = store.primary_account().ok().flatten() else { return };
    let photo = AccountPhoto { etag: "preview".into(), data_url: Some(svg_data_url(DEMO_PHOTO_SVG)), checked_at: Some(chrono::Utc::now()), ..Default::default() };
    store.set_account_photo(&account, &photo).expect("store the demo account photo");
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
    fake_account_photo(&store);
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
