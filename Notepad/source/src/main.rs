//! AxentraNotepad: text and code editing for the Axentra suite.
//!
//! The page (ui/) owns the open documents and is served over the private `axn` scheme. It
//! asks `/api` for files, folders, search and the session; file dialogs and the clipboard
//! run on the GTK main thread.
//!
//!   AxentraNotepad [--new-window] [PATH...]   open files or a folder (one window by default)
//!   AxentraNotepad --serve PORT [-- PATH...]  developer mode: page + API on http://127.0.0.1:PORT
#![cfg_attr(windows, windows_subsystem = "windows")]

mod api;
mod files;
mod fsops;
#[cfg(unix)]
mod instance;
mod logo;
mod search;
mod window_chrome;

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value as J};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, Icon, WindowBuilder};
use wry::http::{header::CONTENT_TYPE, Request, Response, StatusCode};
use wry::{DragDropEvent, RequestAsyncResponder, WebViewBuilder};

const INDEX: &str = include_str!(concat!(env!("OUT_DIR"), "/index.html"));
const APP_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/app.js"));
const CORE_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/core.js"));
const EDITOR_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/editor.js"));
const LANG_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/lang.js"));
const CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/app.css"));
#[cfg(windows)]
const ORIGIN: &str = "http://axn.localhost";
#[cfg(not(windows))]
const ORIGIN: &str = "axn://localhost";
const TITLE: &str = "Axentra Notepad";

enum UserEvent {
    Window(window_chrome::Action),
    OpenDialog(String, RequestAsyncResponder),
    SaveDialog(String, RequestAsyncResponder),
    FolderDialog(String, RequestAsyncResponder),
    ClipGet(RequestAsyncResponder),
    ClipSet(String),
    Title(String),
    /// Files to open: from another launch (raise the window) or dropped on this one.
    Paths(Vec<String>, bool),
    /// The answer to the unsaved-changes question when closing: "save", "discard" or "cancel".
    CloseChoice(&'static str),
    /// Wakes the event loop at a closing deadline (tao's WaitUntil doesn't wake by itself on GTK).
    Wake,
}

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();
static CTX: OnceLock<api::Ctx> = OnceLock::new();
/// Set by the page while the session isn't written yet, so closing lets it finish first.
static GUARD: AtomicBool = AtomicBool::new(false);
/// Set by the page when closing should ask about unsaved changes; DIRTY names the tabs.
static ASK: AtomicBool = AtomicBool::new(false);
static DIRTY: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// How long the page gets to finish closing before the window exits anyway. Unsaved tabs
/// are backed up as you type, so a page that stops answering can't trap the window.
const CLOSE_GRACE: Duration = Duration::from_secs(3);
/// Stands in for the system clipboard in --serve mode.
static DEV_CLIP: Mutex<String> = Mutex::new(String::new());

fn send(ev: UserEvent) {
    if let Some(p) = PROXY.get() {
        let _ = p.lock().unwrap().send_event(ev);
    }
}

/// Delivers UserEvent::Wake at `at`, so a closing deadline is checked even if nothing else happens.
fn wake_at(at: Instant) -> Instant {
    std::thread::spawn(move || {
        std::thread::sleep(at.saturating_duration_since(Instant::now()));
        send(UserEvent::Wake);
    });
    at
}

fn ctx() -> &'static api::Ctx {
    CTX.get().expect("context is set before the page loads")
}

fn respond(status: StatusCode, mime: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder().status(status).header(CONTENT_TYPE, mime).header("Cache-Control", "no-store").body(body).unwrap()
}

fn json_response(v: &J) -> Response<Cow<'static, [u8]>> {
    respond(StatusCode::OK, "application/json", Cow::Owned(serde_json::to_vec(v).unwrap_or_default()))
}

fn spawn_window(path: Option<&str>) -> bool {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("AxentraNotepad")));
    cmd.arg("--new-window");
    if let Some(p) = path {
        cmd.arg("--").arg(p);
    }
    fsops::spawn(cmd)
}

fn handle(req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let text = |s: &'static str, mime: &str| respond(StatusCode::OK, mime, Cow::Borrowed(s.as_bytes()));
    // Debug builds can serve the page from disk (AXN_UI_DIR=ui), so page changes need no rebuild.
    if cfg!(debug_assertions) {
        if let Some(dir) = std::env::var_os("AXN_UI_DIR") {
            let path = req.uri().path();
            let name = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
            if ["index.html", "app.css", "app.js", "core.js", "editor.js", "lang.js"].contains(&name) {
                if let Ok(body) = std::fs::read(std::path::Path::new(&dir).join(name)) {
                    let mime = match name.rsplit('.').next() {
                        Some("html") => "text/html; charset=utf-8",
                        Some("css") => "text/css; charset=utf-8",
                        _ => "text/javascript; charset=utf-8",
                    };
                    return respond(StatusCode::OK, mime, Cow::Owned(body));
                }
            }
        }
    }
    match req.uri().path() {
        "/" | "/index.html" => text(INDEX, "text/html; charset=utf-8"),
        "/app.js" => text(APP_JS, "text/javascript; charset=utf-8"),
        "/core.js" => text(CORE_JS, "text/javascript; charset=utf-8"),
        "/editor.js" => text(EDITOR_JS, "text/javascript; charset=utf-8"),
        "/lang.js" => text(LANG_JS, "text/javascript; charset=utf-8"),
        "/app.css" => text(CSS, "text/css; charset=utf-8"),
        "/logo.png" => {
            static LOGO: OnceLock<Vec<u8>> = OnceLock::new();
            respond(StatusCode::OK, "image/png", Cow::Borrowed(LOGO.get_or_init(|| logo::png(64)).as_slice()))
        }
        "/api" => {
            let body: J = serde_json::from_slice(req.body()).unwrap_or(J::Null);
            json_response(&api::handle(ctx(), &body))
        }
        "/state" => {
            let body: J = serde_json::from_slice(req.body()).unwrap_or(J::Null);
            GUARD.store(body["guard"].as_bool().unwrap_or(false), Ordering::SeqCst);
            ASK.store(body["ask"].as_bool().unwrap_or(false), Ordering::SeqCst);
            if let Some(names) = body["dirty"].as_array() {
                *DIRTY.lock().unwrap_or_else(|e| e.into_inner()) = names.iter().filter_map(|n| n.as_str().map(String::from)).take(50).collect();
            }
            if let Some(t) = body["title"].as_str() {
                send(UserEvent::Title(t.to_string()));
            }
            json_response(&json!({ "ok": true }))
        }
        "/spawn" => {
            let body: J = serde_json::from_slice(req.body()).unwrap_or(J::Null);
            json_response(&json!({ "ok": spawn_window(body["path"].as_str()) }))
        }
        "/clip/set" => {
            let text = String::from_utf8_lossy(req.body()).into_owned();
            if PROXY.get().is_some() {
                send(UserEvent::ClipSet(text));
            } else {
                *DEV_CLIP.lock().unwrap() = text;
            }
            json_response(&json!({ "ok": true }))
        }
        "/clip/get" => json_response(&json!({ "text": DEV_CLIP.lock().unwrap().clone() })),
        "/dialog/open" => json_response(&json!({ "paths": [] })),
        "/dialog/save" | "/dialog/folder" => json_response(&json!({ "path": null })),
        _ => respond(StatusCode::NOT_FOUND, "text/plain", Cow::Borrowed(b"Not found")),
    }
}

/// Requests run off the main thread; a folder search can take a while. A panic still answers
/// the page: an unanswered request would hang it (and anything waiting on it, like closing).
fn spawn_handler(req: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    let _ = std::thread::Builder::new().stack_size(16 << 20).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(req)))
            .unwrap_or_else(|_| json_response(&json!({ "ok": false, "error": "Axentra Notepad hit an internal error. Try that again." })));
        responder.respond(reply)
    });
}

fn window_icon() -> Icon {
    Icon::from_rgba(logo::logo_rgba(64), 64, 64).expect("window icon")
}

struct Args {
    serve: Option<u16>,
    new_window: bool,
    paths: Vec<String>,
}

fn parse_args(args: &[String]) -> Args {
    let mut out = Args { serve: None, new_window: false, paths: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--" => {
                out.paths.extend(it.by_ref().cloned());
                break;
            }
            "--serve" => out.serve = it.next().and_then(|p| p.parse().ok()),
            "--new-window" => out.new_window = true,
            flag if flag.starts_with("--") => {}
            path => out.paths.push(path.to_string()),
        }
    }
    // Absolute paths, so another window (with another working directory) can open them.
    out.paths = out
        .paths
        .iter()
        .map(|p| {
            let abs = api::absolute(std::path::Path::new(p));
            std::fs::canonicalize(&abs).unwrap_or(abs).display().to_string()
        })
        .collect();
    out
}

fn startup(data: &std::path::Path, paths: &[String]) -> J {
    let mut s = api::resolve_paths(data, paths);
    s["folder"] = s["folders"].get(0).cloned().unwrap_or(J::Null);
    s
}

fn main() {
    // WebKit's GPU renderer can stop painting on hybrid Intel/NVIDIA laptops: the page keeps
    // running but the window shows a stale frame, so prompts never appear. A text editor
    // doesn't need the GPU. AXN_GPU=1 keeps it on.
    #[cfg(target_os = "linux")]
    if std::env::var_os("AXN_GPU").is_none() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    let args = parse_args(&std::env::args().skip(1).collect::<Vec<_>>());
    let data = files::data_dir();
    let _ = std::fs::create_dir_all(&data);

    if let Some(port) = args.serve {
        let _ = CTX.set(api::Ctx { data: data.clone(), primary: true, startup: startup(&data, &args.paths) });
        dev_server(port);
        return;
    }

    #[cfg(unix)]
    let (listener, socket) = {
        let socket = instance::socket_path(&data);
        match if args.new_window { instance::Claim::Alone } else { instance::claim(&socket, &args.paths) } {
            instance::Claim::Forwarded => return,
            instance::Claim::Primary(l) => (Some(l), Some(socket)),
            instance::Claim::Alone => (None, None),
        }
    };
    #[cfg(unix)]
    let primary = listener.is_some();
    #[cfg(not(unix))]
    let primary = !args.new_window;
    let _ = CTX.set(api::Ctx { data: data.clone(), primary, startup: startup(&data, &args.paths) });

    #[cfg(target_os = "linux")]
    {
        // Desktop launchers match windows by this name (axentra-notepad.desktop).
        gtk::glib::set_prgname(Some("axentra-notepad"));
        gtk::glib::set_application_name(TITLE);
    }
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "linux")]
    gtk::gdk::set_program_class("Axentra-notepad");
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    #[cfg(unix)]
    if let Some(l) = listener {
        instance::serve(l, |paths| send(UserEvent::Paths(paths, true)));
    }
    let window = WindowBuilder::new()
        .with_title(TITLE)
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .with_window_icon(Some(window_icon()))
        .build(&event_loop)
        .expect("create window");

    let web_dir = data.join("WebKit");
    let _ = std::fs::create_dir_all(&web_dir);
    let mut web_context = wry::WebContext::new(Some(web_dir));
    let webview = WebViewBuilder::with_web_context(&mut web_context)
        .with_ipc_handler(|request| {
            if let Some(action) = window_chrome::parse(&request) {
                send(UserEvent::Window(action));
            }
        })
        .with_asynchronous_custom_protocol("axn".into(), |_id, req, responder| {
            let body = || String::from_utf8_lossy(req.body()).into_owned();
            match req.uri().path() {
                "/dialog/open" => send(UserEvent::OpenDialog(body(), responder)),
                "/dialog/save" => send(UserEvent::SaveDialog(body(), responder)),
                "/dialog/folder" => send(UserEvent::FolderDialog(body(), responder)),
                "/clip/get" => send(UserEvent::ClipGet(responder)),
                _ => spawn_handler(req, responder),
            }
        })
        .with_drag_drop_handler(|e| {
            if let DragDropEvent::Drop { paths, .. } = e {
                send(UserEvent::Paths(paths.iter().map(|p| p.display().to_string()).collect(), false));
            }
            true
        })
        .with_navigation_handler(|url| url == ORIGIN || url.starts_with(&format!("{ORIGIN}/")) || url == "about:blank")
        .with_background_color((11, 14, 30, 255))
        .with_url(format!("{ORIGIN}/"))
        .with_devtools(cfg!(debug_assertions));
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        webview.build_gtk(window.default_vbox().expect("GTK container"))
    };
    #[cfg(not(target_os = "linux"))]
    let webview = webview.build(&window);
    let webview = match webview {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Axentra Notepad: couldn't start the system webview: {e}");
            return;
        }
    };
    #[cfg(target_os = "linux")]
    if std::env::var_os("AXN_GPU").is_none() {
        use webkit2gtk::{SettingsExt, WebViewExt};
        use wry::WebViewExtUnix;
        if let Some(settings) = WebViewExt::settings(&webview.webview()) {
            settings.set_hardware_acceleration_policy(webkit2gtk::HardwareAccelerationPolicy::Never);
        }
    }

    let mut last_state = None;
    // Closing: GTK asks about unsaved changes (it draws even if the page can't), the page then
    // saves or discards and quits, and the window exits by itself if the page doesn't finish.
    #[cfg(target_os = "linux")]
    let mut close_prompt: Option<gtk::MessageDialog> = None;
    let mut exit_at: Option<Instant> = None;
    // After "Save" the page must say it started within CLOSE_GRACE, or we offer to close anyway.
    let mut save_ack_by: Option<Instant> = None;
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::Window(window_chrome::Action::Close)) => {
                #[cfg(target_os = "linux")]
                {
                    use gtk::prelude::*;
                    if let Some(d) = &close_prompt {
                        d.present();
                    } else if ASK.load(Ordering::SeqCst) {
                        let names = DIRTY.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        close_prompt = Some(native::close_prompt(&window, &names));
                    } else if GUARD.load(Ordering::SeqCst) {
                        let _ = webview.evaluate_script("window.axnCloseChoice && window.axnCloseChoice('flush')");
                        exit_at.get_or_insert_with(|| wake_at(Instant::now() + CLOSE_GRACE));
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                #[cfg(not(target_os = "linux"))]
                if ASK.load(Ordering::SeqCst) || GUARD.load(Ordering::SeqCst) {
                    let _ = webview.evaluate_script("window.axnRequestClose && window.axnRequestClose()");
                } else {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(UserEvent::CloseChoice(choice)) => {
                #[cfg(target_os = "linux")]
                {
                    close_prompt = None;
                }
                match choice {
                    "save" => {
                        // Saving may open Save As dialogs, so once the page answers there's no time limit.
                        let _ = webview.evaluate_script("window.axnCloseChoice && window.axnCloseChoice('save')");
                        save_ack_by = Some(wake_at(Instant::now() + CLOSE_GRACE));
                    }
                    "force" => *control_flow = ControlFlow::Exit,
                    "discard" => {
                        let _ = webview.evaluate_script("window.axnCloseChoice && window.axnCloseChoice('discard')");
                        exit_at.get_or_insert_with(|| wake_at(Instant::now() + CLOSE_GRACE));
                    }
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::Window(action)) => {
                use window_chrome::Action;
                match action {
                    Action::State => last_state = None,
                    Action::Minimize => window.set_minimized(true),
                    Action::ToggleMaximize => window.set_maximized(!window.is_maximized()),
                    Action::Fullscreen => {
                        window.set_fullscreen(if window.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) })
                    }
                    Action::Drag => {
                        let _ = window.drag_window();
                    }
                    Action::Resize { edge } if !window.is_maximized() => {
                        let _ = window.drag_resize_window(edge.into());
                    }
                    Action::Quit => *control_flow = ControlFlow::Exit,
                    Action::Closing => save_ack_by = None,
                    Action::Resize { .. } | Action::Close => {}
                }
            }
            Event::UserEvent(UserEvent::Title(t)) => window.set_title(&t),
            Event::UserEvent(UserEvent::Paths(paths, raise)) => {
                let list = serde_json::to_string(&paths).unwrap_or("[]".into());
                let _ = webview.evaluate_script(&format!("window.axnOpenPaths && window.axnOpenPaths({list})"));
                if raise {
                    window.set_minimized(false);
                    window.set_visible(true);
                    window.set_focus();
                }
            }
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::OpenDialog(body, r)) => native::open_dialog(&window, &body, r),
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::SaveDialog(body, r)) => native::save_dialog(&window, &body, r),
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::FolderDialog(body, r)) => native::folder_dialog(&window, &body, r),
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::ClipGet(r)) => native::clip_get(r),
            #[cfg(target_os = "linux")]
            Event::UserEvent(UserEvent::ClipSet(t)) => native::clip_set(&t),
            #[cfg(not(target_os = "linux"))]
            Event::UserEvent(UserEvent::OpenDialog(_, r) | UserEvent::SaveDialog(_, r) | UserEvent::FolderDialog(_, r) | UserEvent::ClipGet(r)) => {
                r.respond(json_response(&json!({ "path": null, "paths": [], "text": "" })))
            }
            #[cfg(not(target_os = "linux"))]
            Event::UserEvent(UserEvent::ClipSet(_)) => {}
            Event::LoopDestroyed => {
                #[cfg(unix)]
                if let Some(s) = &socket {
                    let _ = std::fs::remove_file(s);
                }
            }
            _ => {}
        }
        let state = (window.is_maximized(), window.fullscreen().is_some());
        if last_state != Some(state) {
            let _ = webview.evaluate_script(&format!("window.axnWindowState && window.axnWindowState({{maximized:{},fullscreen:{}}})", state.0, state.1));
            last_state = Some(state);
        }
        if save_ack_by.is_some_and(|t| Instant::now() >= t) {
            save_ack_by = None;
            #[cfg(target_os = "linux")]
            if close_prompt.is_none() {
                close_prompt = Some(native::unresponsive_prompt(&window));
            }
            #[cfg(not(target_os = "linux"))]
            {
                *control_flow = ControlFlow::Exit;
            }
        }
        if !matches!(*control_flow, ControlFlow::Exit) {
            if exit_at.is_some_and(|t| Instant::now() >= t) {
                *control_flow = ControlFlow::Exit;
            } else if let Some(t) = [exit_at, save_ack_by].into_iter().flatten().min() {
                *control_flow = ControlFlow::WaitUntil(t);
            }
        }
    });
}

#[cfg(target_os = "linux")]
mod native {
    //! GTK file choosers and the clipboard. Everything here runs on the main thread.
    use std::cell::RefCell;
    use std::path::PathBuf;

    use gtk::prelude::*;
    use serde_json::{json, Value as J};
    use tao::platform::unix::WindowExtUnix;
    use wry::RequestAsyncResponder;

    thread_local! {
        static DIALOG: RefCell<Option<gtk::FileChooserNative>> = const { RefCell::new(None) };
    }

    fn filter(name: &str, patterns: &[&str]) -> gtk::FileFilter {
        let f = gtk::FileFilter::new();
        f.set_name(Some(name));
        for p in patterns {
            f.add_pattern(p);
            if *p != "*" {
                f.add_pattern(&p.to_uppercase());
            }
        }
        f
    }

    const SOURCE: &[&str] = &[
        "*.c", "*.h", "*.cpp", "*.cc", "*.cxx", "*.hpp", "*.hh", "*.cs", "*.java", "*.kt", "*.kts", "*.swift", "*.go", "*.rs", "*.py", "*.pyw", "*.rb",
        "*.php", "*.pl", "*.lua", "*.dart", "*.scala", "*.js", "*.mjs", "*.cjs", "*.jsx", "*.ts", "*.tsx", "*.sh", "*.bash", "*.zsh", "*.fish", "*.ps1",
        "*.psm1", "*.bat", "*.cmd", "*.sql", "*.r", "*.vb", "*.json", "*.jsonc", "*.yaml", "*.yml", "*.toml", "*.ini", "*.cfg", "*.conf", "*.xml",
        "*.html", "*.htm", "*.css", "*.scss", "*.less", "*.md", "Makefile", "Dockerfile", "CMakeLists.txt", "*.cmake",
    ];

    fn busy() -> bool {
        DIALOG.with(|slot| slot.borrow().is_some())
    }

    fn start_dir(req: &J) -> Option<PathBuf> {
        req["dir"]
            .as_str()
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
            .or_else(|| gtk::glib::user_special_dir(gtk::glib::UserDirectory::Documents).filter(|d| d.is_dir()))
            .or_else(|| Some(gtk::glib::home_dir()))
    }

    fn run(dialog: gtk::FileChooserNative, responder: RequestAsyncResponder, cancel: J, accept: impl Fn(&gtk::FileChooserNative) -> J + 'static) {
        let responder = RefCell::new(Some(responder));
        dialog.connect_response(move |d, resp| {
            let reply = if resp == gtk::ResponseType::Accept { accept(d) } else { cancel.clone() };
            if let Some(r) = responder.borrow_mut().take() {
                r.respond(super::json_response(&reply));
            }
            DIALOG.with(|slot| slot.borrow_mut().take());
        });
        dialog.set_modal(true);
        dialog.show();
        DIALOG.with(|slot| *slot.borrow_mut() = Some(dialog));
    }

    pub fn open_dialog(window: &tao::window::Window, body: &str, responder: RequestAsyncResponder) {
        if busy() {
            return responder.respond(super::json_response(&json!({ "paths": [] })));
        }
        let req: J = serde_json::from_str(body).unwrap_or(J::Null);
        let d = gtk::FileChooserNative::new(Some("Open"), Some(window.gtk_window()), gtk::FileChooserAction::Open, Some("_Open"), Some("_Cancel"));
        d.set_select_multiple(true);
        let all = filter("All files", &["*"]);
        d.add_filter(all.clone());
        d.add_filter(filter("Text documents (*.txt)", &["*.txt", "*.text", "*.log"]));
        d.add_filter(filter("Source code", SOURCE));
        d.add_filter(filter("Web (*.html, *.css, *.js, *.ts, *.json)", &["*.html", "*.htm", "*.css", "*.js", "*.mjs", "*.ts", "*.tsx", "*.jsx", "*.json"]));
        d.add_filter(filter("Markdown (*.md)", &["*.md", "*.markdown"]));
        d.set_filter(&all);
        if let Some(dir) = start_dir(&req) {
            let _ = d.set_current_folder(dir);
        }
        run(d, responder, json!({ "paths": [] }), |d| {
            json!({ "paths": d.filenames().iter().map(|p| p.display().to_string()).collect::<Vec<_>>() })
        });
    }

    pub fn save_dialog(window: &tao::window::Window, body: &str, responder: RequestAsyncResponder) {
        if busy() {
            return responder.respond(super::json_response(&json!({ "path": null })));
        }
        let req: J = serde_json::from_str(body).unwrap_or(J::Null);
        let title = req["title"].as_str().unwrap_or("Save As");
        let d = gtk::FileChooserNative::new(Some(title), Some(window.gtk_window()), gtk::FileChooserAction::Save, Some("_Save"), Some("_Cancel"));
        d.set_do_overwrite_confirmation(true);
        d.set_current_name(req["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Untitled.txt"));
        if let Some(dir) = start_dir(&req) {
            let _ = d.set_current_folder(dir);
        }
        run(d, responder, json!({ "path": null }), |d| json!({ "path": d.filename().map(|p| p.display().to_string()) }));
    }

    pub fn folder_dialog(window: &tao::window::Window, body: &str, responder: RequestAsyncResponder) {
        if busy() {
            return responder.respond(super::json_response(&json!({ "path": null })));
        }
        let req: J = serde_json::from_str(body).unwrap_or(J::Null);
        let d = gtk::FileChooserNative::new(Some("Open Folder"), Some(window.gtk_window()), gtk::FileChooserAction::SelectFolder, Some("_Open"), Some("_Cancel"));
        if let Some(dir) = start_dir(&req) {
            let _ = d.set_current_folder(dir);
        }
        run(d, responder, json!({ "path": null }), |d| json!({ "path": d.filename().map(|p| p.display().to_string()) }));
    }

    pub fn clip_get(responder: RequestAsyncResponder) {
        let clip = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        let responder = RefCell::new(Some(responder));
        clip.request_text(move |_, text| {
            if let Some(r) = responder.borrow_mut().take() {
                r.respond(super::json_response(&json!({ "text": text.map(|t| t.to_string()) })));
            }
        });
    }

    /// The unsaved-changes question when closing the window. GTK draws it, so it shows even
    /// when the page can't paint; the answer comes back as UserEvent::CloseChoice.
    pub fn close_prompt(window: &tao::window::Window, names: &[String]) -> gtk::MessageDialog {
        let (primary, secondary) = if names.len() <= 1 {
            let name = names.first().map(String::as_str).unwrap_or("this document");
            (format!("Do you want to save the changes you made to “{name}”?"), "Your changes will be lost if you don't save them.".to_string())
        } else {
            let mut list: Vec<String> = names.iter().take(8).map(|n| format!("• {n}")).collect();
            if names.len() > 8 {
                list.push(format!("…and {} more", names.len() - 8));
            }
            (format!("Do you want to save the changes to {} files?", names.len()), list.join("\n"))
        };
        let d = gtk::MessageDialog::new(
            Some(window.gtk_window()),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Warning,
            gtk::ButtonsType::None,
            &primary,
        );
        d.set_title(super::TITLE);
        d.set_secondary_text(Some(&secondary));
        d.add_button("Cancel", gtk::ResponseType::Cancel);
        d.add_button("Don't Save", gtk::ResponseType::Reject);
        d.add_button(if names.len() > 1 { "Save All" } else { "Save" }, gtk::ResponseType::Accept);
        d.set_default_response(gtk::ResponseType::Accept);
        d.connect_response(|d, r| {
            let choice = match r {
                gtk::ResponseType::Accept => "save",
                gtk::ResponseType::Reject => "discard",
                _ => "cancel",
            };
            super::send(super::UserEvent::CloseChoice(choice));
            d.close();
        });
        d.show_all();
        d.present();
        d
    }

    /// Shown when the page doesn't respond to "Save" while closing. Unsaved tabs are backed up
    /// as you type, so closing anyway keeps them for next time.
    pub fn unresponsive_prompt(window: &tao::window::Window) -> gtk::MessageDialog {
        let d = gtk::MessageDialog::new(
            Some(window.gtk_window()),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Warning,
            gtk::ButtonsType::None,
            "Axentra Notepad isn't responding",
        );
        d.set_title(super::TITLE);
        d.set_secondary_text(Some("It couldn't save your changes. If you close it now, your unsaved tabs are kept and come back the next time you open Axentra Notepad."));
        d.add_button("Keep Waiting", gtk::ResponseType::Cancel);
        d.add_button("Close Anyway", gtk::ResponseType::Accept);
        d.set_default_response(gtk::ResponseType::Accept);
        d.connect_response(|d, r| {
            super::send(super::UserEvent::CloseChoice(if r == gtk::ResponseType::Accept { "force" } else { "cancel" }));
            d.close();
        });
        d.show_all();
        d.present();
        d
    }

    pub fn clip_set(text: &str) {
        let clip = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clip.set_text(text);
        clip.store();
    }
}

fn dev_server(port: u16) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind dev server");
    eprintln!("Axentra Notepad dev server on http://127.0.0.1:{port}/");
    for stream in listener.incoming().flatten() {
        let _ = std::thread::Builder::new().stack_size(16 << 20).spawn(move || {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                return;
            }
            let mut parts = line.split_whitespace();
            let (method, target) = (parts.next().unwrap_or("GET").to_string(), parts.next().unwrap_or("/").to_string());
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h == "\r\n" || h.is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len.min(256 << 20)];
            let _ = reader.read_exact(&mut body);
            let req = Request::builder().method(method.as_str()).uri(format!("http://127.0.0.1{target}")).body(body).unwrap();
            let resp = handle(req);
            let mut out = stream;
            let ct = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("text/plain").to_string();
            let body = resp.body();
            let _ = write!(out, "HTTP/1.1 {}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", resp.status(), body.len());
            let _ = out.write_all(body);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        let a = parse_args(&["--new-window".into(), "/tmp".into(), "--".into(), "--odd-name".into()]);
        assert!(a.new_window && a.serve.is_none());
        assert_eq!(a.paths.len(), 2);
        assert!(a.paths[1].ends_with("--odd-name") && std::path::Path::new(&a.paths[1]).is_absolute());
        let a = parse_args(&["--serve".into(), "8797".into(), "--".into(), "x.txt".into()]);
        assert_eq!(a.serve, Some(8797));
        assert!(a.paths[0].ends_with("/x.txt"));
    }
}
