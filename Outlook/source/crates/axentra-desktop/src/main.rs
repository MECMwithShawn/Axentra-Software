//! Desktop entry point: opens the local database (seeding demo data on first
//! run unless AXENTRA_NO_DEMO is set), starts sync and opens the window.
//!
//! The window has no native decorations: the app header is the title bar
//! (`titlebar.rs`). "Use the system title bar" in Settings, or
//! AXENTRA_SYSTEM_TITLEBAR=1, brings the desktop's own title bar back.
//! AXENTRA_BUTTON_LAYOUT (GNOME `button-layout` syntax) overrides where the
//! window buttons go.
//!
//! One app runs per database (`instance.rs`): launching it again shows the
//! open window and exits.
//!
//! Command line: `--version` and `--help` print and exit. `--compose` (with
//! `--to`, `--subject`, `--body-file`, `--attach`…) or a `mailto:` link
//! opens a filled-in new message for the user to review; it is never sent
//! from here (`axentra_core::compose_request`). With `--send` (and only
//! with explicit options, never a link) the message is sent instead, by
//! the app (started in the background if it isn't open), and the command
//! prints how it went ([`send_from_command_line`]). Anything else is
//! ignored with a note.

mod appearance;
mod icon;
mod instance;
mod links;
mod pictures;
mod pin;
mod titlebar;

use std::sync::Arc;

use axentra_core::Store;
use axentra_core::compose_request::{self, ComposeRequest};
use dioxus::desktop::tao::window::{Icon, Theme};
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};

/// Minimum window size (docs/spec/shell.md §2).
const MIN_SIZE: (f64, f64) = (640.0, 480.0);
/// First-run window size: the reference layout's 1440 × 900.
const DEFAULT_SIZE: (f64, f64) = (1440.0, 900.0);

const TEXT_ONLY_CONTEXT_MENU: &str = "<script>document.addEventListener('contextmenu', e => { \
if (!(e.target instanceof Element) || !e.target.closest('input, textarea, [contenteditable]:not([contenteditable=\"false\"])')) e.preventDefault(); \
}, true);</script>";

const USAGE: &str = "Usage: axentra-outlook [--version | --help]
       axentra-outlook --compose [--to ADDR]... [--cc ADDR]... [--bcc ADDR]...
                       [--subject TEXT] [--body TEXT | --body-file FILE]
                       [--attach FILE]...
       axentra-outlook --compose --send --to ADDR --subject TEXT
                       (--body TEXT | --body-file FILE) [--attach FILE]...
       axentra-outlook mailto:ADDR?subject=...&body=...

Opens Axentra Outlook, or shows its window when it is already open on this
profile.

--compose and mailto: links open a new message, filled in, for you to
review and send yourself. Nothing is sent unless --send is given. Addresses
may be given as \"Name <addr>\" and several at once (\"a@x, b@y\"). Files
are attached only with --attach, never from a mailto: link.

--send sends the message as given, without opening it: it needs a
recipient, a subject, a body and readable attachments, and never works with
a mailto: link. Axentra Outlook is started in the background if it isn't
open. The command waits (up to about 10 minutes), prints one line saying
what was sent or why not, and exits 0 when the message is in Sent Items,
1 when it wasn't sent (it stays in Drafts when it got that far), 2 for a
mistake on the command line. Each send is logged in <profile>.sends.log.

Environment:
  AXENTRA_DB=<file>          use another profile database (a test profile)
  AXENTRA_NO_DEMO=1          don't add demo data to a new profile
  AXENTRA_SYSTEM_TITLEBAR=1  use the desktop's own title bar
  RUST_LOG=debug             log to the terminal";

/// What the command line asks for (see [`USAGE`]).
#[derive(Debug, PartialEq)]
enum Args {
    Run { compose: Option<ComposeRequest>, ignored: Vec<String> },
    Version,
    Help,
    /// A compose option without its value, an unreadable body file or a
    /// missing attachment: said on stderr, and nothing opens.
    Bad(String),
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    let mut ignored = Vec::new();
    let mut compose: Option<ComposeRequest> = None;
    let (mut send_flag, mut from_link) = (false, false);
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        // `--to=x` as well as `--to x`.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        match flag.as_str() {
            "--version" | "-V" => return Args::Version,
            "--help" | "-h" => return Args::Help,
            "--compose" => {
                compose.get_or_insert_with(ComposeRequest::default);
            }
            "--send" => {
                compose.get_or_insert_with(ComposeRequest::default).send = true;
                send_flag = true;
            }
            "--to" | "--cc" | "--bcc" | "--subject" | "--body" | "--body-file" | "--attach" => {
                let Some(value) = inline.or_else(|| args.next()) else {
                    return Args::Bad(format!("{flag} needs a value"));
                };
                let req = compose.get_or_insert_with(ComposeRequest::default);
                match flag.as_str() {
                    "--to" => compose_request::push_addresses(&mut req.to, &value),
                    "--cc" => compose_request::push_addresses(&mut req.cc, &value),
                    "--bcc" => compose_request::push_addresses(&mut req.bcc, &value),
                    "--subject" => req.subject = compose_request::one_line(&value),
                    "--body" => req.body = value,
                    "--body-file" => match read_body_file(&value) {
                        Ok(body) => req.body = body,
                        Err(e) => return Args::Bad(e),
                    },
                    _ => match std::path::absolute(&value) {
                        Ok(p) if p.is_file() => req.attachments.push(p),
                        _ => return Args::Bad(format!("can't attach {value}: no such file")),
                    },
                }
            }
            _ if a.len() >= 7 && a[..7].eq_ignore_ascii_case("mailto:") => match compose_request::parse_mailto(&a) {
                Some(link) => {
                    from_link = true;
                    let req = compose.get_or_insert_with(ComposeRequest::default);
                    req.to.extend(link.to);
                    req.cc.extend(link.cc);
                    req.bcc.extend(link.bcc);
                    if req.subject.is_empty() {
                        req.subject = link.subject;
                    }
                    if req.body.is_empty() {
                        req.body = link.body;
                    }
                }
                None => ignored.push(a),
            },
            _ => ignored.push(a),
        }
    }
    if send_flag {
        if from_link {
            return Args::Bad("--send can't be used with a mailto: link".into());
        }
        let problems = compose.as_ref().map(compose_request::send_problems).unwrap_or_default();
        if !problems.is_empty() {
            return Args::Bad(format!("not sent: {}", problems.join("; ")));
        }
    }
    Args::Run { compose, ignored }
}

/// How long `--send` waits for the app to take the request (it may have to
/// start first), and for the whole send.
const SEND_PICKUP: std::time::Duration = std::time::Duration::from_secs(120);
const SEND_TOTAL: std::time::Duration = std::time::Duration::from_secs(11 * 60);

/// `--send`: hand the message to the app (starting it in the background
/// when it doesn't take the request within a few seconds), wait for its
/// answer, print it, log it. Returns the exit code.
fn send_from_command_line(db: &std::path::Path, req: &ComposeRequest) -> i32 {
    let request = match compose_request::submit(db, req) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("axentra-outlook: not sent: couldn't hand the message to Axentra Outlook: {e}");
            return 1;
        }
    };
    let mut started = false;
    let waited = compose_request::wait_result(&request, SEND_PICKUP, SEND_TOTAL, |waited| {
        // Not taken within a few seconds: the app isn't open (or is still
        // starting). Start it once, detached, without a compose request.
        if !started && waited >= std::time::Duration::from_secs(3) && request.exists() {
            started = true;
            eprintln!("axentra-outlook: starting Axentra Outlook to send the message…");
            if let Err(e) = start_app_detached() {
                eprintln!("axentra-outlook: couldn't start Axentra Outlook: {e}");
            }
        }
    });
    let (code, line) = match waited {
        compose_request::Waited::Done(r) if r.sent => (0, r.text),
        compose_request::Waited::Done(r) => (1, r.text),
        compose_request::Waited::NotTaken => (1, "Not sent: Axentra Outlook didn't take the message (is an older version open? Close it and try again).".to_string()),
        compose_request::Waited::NoAnswer => (1, "No answer from Axentra Outlook yet; the message may still be in Outbox. Check Sent Items before sending again.".to_string()),
    };
    if code == 0 {
        println!("{line}");
    } else {
        eprintln!("axentra-outlook: {line}");
    }
    log_send(db, req, code == 0, &line);
    code
}

/// Start this program again with no arguments, in its own process group,
/// so it keeps running after the command exits.
fn start_app_detached() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    Command::new(std::env::current_exe()?).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn().map(|_| ())
}

/// One line per command-line send in `<db>.sends.log` (time, result, to,
/// subject, what happened). Private to the user.
fn log_send(db: &std::path::Path, req: &ComposeRequest, sent: bool, line: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let name = db.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "axentra.db".into());
    let path = db.with_file_name(format!("{name}.sends.log"));
    let to: Vec<&str> = req.to.iter().chain(&req.cc).chain(&req.bcc).map(|a| a.address.as_str()).collect();
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let entry = format!(
        "{}\t{}\t{}\t{}\t{}\n",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        if sent { "sent" } else { "not-sent" },
        clean(&to.join(", ")),
        clean(&req.subject),
        clean(line)
    );
    let written = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&path).and_then(|mut f| f.write_all(entry.as_bytes()));
    if let Err(e) = written {
        eprintln!("axentra-outlook: couldn't write {}: {e}", path.display());
    }
}

/// The text of `--body-file` (UTF-8, at most [`compose_request::MAX_BODY`]).
fn read_body_file(path: &str) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("can't read {path}: {e}"))?;
    if data.len() > compose_request::MAX_BODY {
        return Err(format!("{path} is too long for a message body"));
    }
    String::from_utf8(data).map_err(|_| format!("{path} isn't UTF-8 text"))
}

/// The database this copy opens (`AXENTRA_DB`, else the default profile).
fn database_path() -> std::path::PathBuf {
    std::env::var_os("AXENTRA_DB").map(Into::into).unwrap_or_else(Store::default_path)
}

fn main() {
    let compose = match parse_args(std::env::args().skip(1)) {
        Args::Version => {
            println!("{} {}", axentra_ui::APP_NAME, env!("CARGO_PKG_VERSION"));
            return;
        }
        Args::Help => {
            println!("{USAGE}");
            return;
        }
        Args::Bad(e) => {
            eprintln!("axentra-outlook: {e}. See axentra-outlook --help.");
            std::process::exit(2);
        }
        Args::Run { compose, ignored } => {
            if !ignored.is_empty() {
                eprintln!("axentra-outlook: ignoring {}: only mailto: links can be opened from the command line.", ignored.join(" "));
            }
            compose
        }
    };
    let path = database_path();
    if let Some(req) = compose.as_ref().filter(|r| r.send) {
        std::process::exit(send_from_command_line(&path, req));
    }
    // The app that ends up with the window (this one, or the copy already
    // open, which is asked to show itself below) opens it as a draft.
    if let Some(req) = &compose
        && let Err(e) = compose_request::submit(&path, req)
    {
        tell_user("Couldn't start the new message.", &format!("{e}."));
        std::process::exit(1);
    }
    // Pin to desktop on Wayland: GNOME ignores "keep below" from Wayland
    // windows, so a pinned window opens through XWayland, where it is
    // honoured. Decided from a marker next to the database, before any
    // thread or GTK starts (the setting itself is in the store).
    if pin::wants_xwayland(&path, |k| std::env::var_os(k)) {
        // SAFETY: still single-threaded; nothing has read the environment yet.
        unsafe { std::env::set_var("GDK_BACKEND", "x11") };
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    // First, before anything slow: is this profile already open? A copy the
    // lock can't see (one started before this version, which takes no lock,
    // or one whose lock is elsewhere) is checked for too.
    let primary = match instance::start(&path, instance::launch_token().as_deref()) {
        Ok(instance::Start::Run(primary, db_lock)) => {
            // Held until exit: another launch on this database, whatever
            // its runtime directory, sees it.
            std::mem::forget(db_lock);
            primary
        }
        Ok(instance::Start::Shown) => {
            eprintln!("{} is already running; showing its window.", axentra_ui::APP_NAME);
            return;
        }
        // The lock has been let go: a launch made while this dialog is up
        // (after closing the other copy, as it says) opens the app.
        Ok(instance::Start::Refused(other)) => {
            let detail = if other.outdated {
                format!(
                    "A copy started before {} was updated is still running on this mailbox (process {}). Close its window, then open {} again.",
                    axentra_ui::APP_NAME,
                    other.pid,
                    axentra_ui::APP_NAME
                )
            } else {
                format!(
                    "Another copy is already running on this mailbox (process {}). Switch to its window, or close it and open {} again.",
                    other.pid,
                    axentra_ui::APP_NAME
                )
            };
            tell_user(&format!("{} is already open.", axentra_ui::APP_NAME), &detail);
            std::process::exit(1);
        }
        Err(e) => {
            tell_user(&format!("{e}."), "Wait for it to finish what it is doing, or close it, and try again.");
            std::process::exit(1);
        }
    };
    let _instance = primary.map(|primary| {
        #[cfg(target_os = "linux")]
        instance::watch_gtk();
        primary.serve(|req| {
            tracing::debug!("instance: show requested");
            #[cfg(target_os = "linux")]
            instance::show_on_gtk_thread(req);
            #[cfg(not(target_os = "linux"))]
            req.answer(instance::Answer::Shown);
        })
    });
    let force_system_title_bar = std::env::var("AXENTRA_SYSTEM_TITLEBAR").ok();
    // The desktop's title-bar preferences, read while the store opens.
    let titlebar_prefs = std::thread::spawn(titlebar::read_prefs);
    let store = Arc::new(Store::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display())));
    if std::env::var_os("AXENTRA_NO_DEMO").is_none() {
        axentra_core::seed::seed_demo(&store, chrono::Local::now()).expect("seed demo data");
    }
    let settings = store.settings();
    let dark = appearance::is_dark(settings.theme, appearance::system_scheme);
    let csd = !titlebar::system_title_bar(settings.system_title_bar, force_system_title_bar.as_deref());
    axentra_ui::shell::theme::set_system_prefs(appearance::system_prefs());
    let sync = axentra_sync::SyncHandle::start(store.clone());
    axentra_ui::init(store, Some(sync));

    let mut window = WindowBuilder::new()
        .with_title(axentra_ui::APP_NAME)
        .with_inner_size(LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1))
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        // GTK's dark preference also drives the webview's prefers-color-scheme.
        .with_theme(Some(if dark { Theme::Dark } else { Theme::Light }))
        .with_decorations(!csd)
        // The page draws the frame: rounded corners need a see-through window.
        .with_transparent(csd);
    match dioxus::desktop::icon_from_memory::<Icon>(icon::APP_ICON_PNG) {
        Ok(i) => window = window.with_window_icon(Some(i)),
        Err(e) => tracing::warn!("app icon: {e}"),
    }
    let mut cfg = Config::new()
        .with_menu(None)
        .with_window(window)
        // The webview's own menu (cut / copy / paste) only inside editable
        // fields; everywhere else the app shows its own menus or nothing.
        .with_disable_context_menu(false)
        .with_custom_head(TEXT_ONLY_CONTEXT_MENU.to_string());
    let mut launch = dioxus::LaunchBuilder::desktop();
    let root: fn() -> dioxus::prelude::Element = if csd {
        launch = launch.with_context(titlebar_prefs.join().unwrap_or_default());
        titlebar::CsdRoot
    } else {
        // A transparent webview ignores the background colour, so only here.
        cfg = cfg.with_background_color(if dark { appearance::CANVAS_DARK } else { appearance::CANVAS_LIGHT });
        titlebar::SystemRoot
    };
    launch.with_cfg(cfg).launch(root);
}

/// Say why the app is not opening: on stderr, and in a small dialog, since
/// launches from Command or the app menu have no terminal.
fn tell_user(headline: &str, detail: &str) {
    eprintln!("{}: {headline} {detail}", axentra_ui::APP_NAME);
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        if gtk::init().is_err() {
            return;
        }
        let dialog = gtk::MessageDialog::new(None::<&gtk::Window>, gtk::DialogFlags::MODAL, gtk::MessageType::Info, gtk::ButtonsType::Ok, headline);
        dialog.set_title(axentra_ui::APP_NAME);
        dialog.set_secondary_text(Some(detail));
        dialog.set_icon_name(Some("axentra-outlook"));
        dialog.run();
        dialog.close();
    }
}

#[cfg(test)]
mod tests {
    use axentra_core::EmailAddress;

    use super::*;

    fn args(a: &[&str]) -> Args {
        parse_args(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn command_line() {
        assert_eq!(args(&[]), Args::Run { compose: None, ignored: vec![] });
        assert_eq!(args(&["--version"]), Args::Version);
        assert_eq!(args(&["-V"]), Args::Version);
        assert_eq!(args(&["--help"]), Args::Help);
        assert_eq!(args(&["mailto:someone@example.com", "--help"]), Args::Help);
        assert_eq!(
            args(&["mailto:someone@example.com", "/etc/hostname"]),
            Args::Run {
                compose: Some(ComposeRequest { to: vec![EmailAddress::new("", "someone@example.com")], ..Default::default() }),
                ignored: vec!["/etc/hostname".into()]
            }
        );
        assert!(USAGE.contains("--version") && USAGE.contains("AXENTRA_DB") && USAGE.contains("--attach"));
    }

    #[test]
    fn compose_options() {
        let dir = std::env::temp_dir().join(format!("axentra-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let body = dir.join("body.txt");
        std::fs::write(&body, "Hello\nthere").unwrap();
        let file = dir.join("guide.zip");
        std::fs::write(&file, b"zip").unwrap();
        let (b, f) = (body.to_str().unwrap(), file.to_str().unwrap());
        let Args::Run { compose: Some(r), ignored } =
            args(&["--compose", "--to", "a@example.com, B <b@example.com>", "--cc=c@example.com", "--subject", "Guide", "--body-file", b, "--attach", f, "--attach", f])
        else {
            panic!("not a compose request");
        };
        assert!(ignored.is_empty());
        assert_eq!(r.to, vec![EmailAddress::new("", "a@example.com"), EmailAddress::new("B", "b@example.com")]);
        assert_eq!(r.cc, vec![EmailAddress::new("", "c@example.com")]);
        assert_eq!((r.subject.as_str(), r.body.as_str()), ("Guide", "Hello\nthere"));
        assert_eq!(r.attachments, vec![file.clone(), file.clone()]);

        // --to alone means compose too.
        assert!(matches!(args(&["--to", "a@example.com"]), Args::Run { compose: Some(_), .. }));
        assert!(matches!(args(&["--compose"]), Args::Run { compose: Some(r), .. } if r == ComposeRequest::default()));
        // Problems stop before anything opens.
        assert!(matches!(args(&["--compose", "--to"]), Args::Bad(_)));
        assert!(matches!(args(&["--attach", "/no/such/file.zip"]), Args::Bad(_)));
        assert!(matches!(args(&["--attach", dir.to_str().unwrap()]), Args::Bad(_)));
        assert!(matches!(args(&["--body-file", "/no/such/body.txt"]), Args::Bad(_)));

        // --send: explicit and complete only; plain --compose never sends.
        let Args::Run { compose: Some(r), .. } = args(&["--compose", "--send", "--to", "a@example.com", "--subject", "S", "--body", "B", "--attach", f]) else {
            panic!("send request refused");
        };
        assert!(r.send);
        assert!(matches!(args(&["--compose", "--to", "a@example.com"]), Args::Run { compose: Some(r), .. } if !r.send));
        assert!(matches!(args(&["--send", "--to", "a@example.com", "--subject", "S"]), Args::Bad(e) if e.contains("body is empty")));
        assert!(matches!(args(&["--send", "--subject", "S", "--body", "B"]), Args::Bad(e) if e.contains("no recipient")));
        assert!(matches!(args(&["--send", "mailto:a@example.com?subject=S&body=B"]), Args::Bad(e) if e.contains("mailto")));
        assert!(matches!(args(&["mailto:a@example.com?subject=S&body=B&send=1"]), Args::Run { compose: Some(r), .. } if !r.send));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
