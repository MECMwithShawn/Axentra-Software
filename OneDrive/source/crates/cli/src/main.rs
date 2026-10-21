//! `axentra-onedrive`: set up, sign in, run the sync service, and control it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use axentra_onedrive_core::config::{normalize, Config, NeedsUser, Paths, DBUS_NAME};
use axentra_onedrive_core::engine::{Engine, Options};
use axentra_onedrive_core::ipc::AxentraOneDriveProxy;
use axentra_onedrive_core::notify::Notifier;
use axentra_onedrive_core::status::{human_size, Account, Status};
use axentra_onedrive_graph::{auth, Graph, TokenStore};
use clap::{Parser, Subcommand};

/// How `daemon` exits on a [`NeedsUser`] problem (EX_CONFIG). The systemd
/// unit doesn't restart on it, since nothing changes until the user acts.
const EXIT_NEEDS_USER: i32 = 78;

/// How to start the service again after a [`NeedsUser`] problem. Failed
/// starts count towards the unit's start limit as well, so without
/// reset-failed a few tries in a row at a fix would be refused.
const START_AGAIN: &str =
    "systemctl --user reset-failed axentra-onedrive; systemctl --user restart axentra-onedrive";

#[derive(Parser)]
#[command(
    name = "axentra-onedrive",
    version,
    about = "Axentra OneDrive: OneDrive for Linux, modelled on the Windows client"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Connect Axentra OneDrive to your Microsoft Entra app registration.
    Setup {
        /// Application (client) ID of the app registration.
        #[arg(long)]
        client_id: Option<String>,
        /// common, consumers (personal accounts only), organizations, or a tenant ID.
        #[arg(long)]
        tenant: Option<String>,
        /// Where your OneDrive should appear (default ~/OneDrive).
        #[arg(long)]
        folder: Option<String>,
    },
    /// Sign in to OneDrive in your browser.
    Login {
        /// Show a code to enter on another device instead of opening a browser.
        #[arg(long)]
        device: bool,
    },
    /// Unlink this PC: sign out and remove the local copies of your files.
    Logout {
        /// Don't ask first, even if some changes haven't uploaded yet.
        #[arg(long, short)]
        yes: bool,
    },
    /// Run the sync service in the foreground (systemd runs this for you).
    Daemon,
    /// Show sync status and storage.
    Status,
    /// Recent activity, newest first.
    Activity {
        #[arg(short, default_value_t = 20)]
        n: u32,
    },
    /// Always keep on this device.
    Pin {
        paths: Vec<PathBuf>,
    },
    /// Stop always keeping these on this device (they stay local for now).
    Unpin {
        paths: Vec<PathBuf>,
    },
    /// Free up space (keep online only).
    Free {
        paths: Vec<PathBuf>,
    },
    /// Copy a sharing link (prints it).
    Share {
        path: PathBuf,
        /// Let people edit, not just view.
        #[arg(long)]
        edit: bool,
    },
    /// Open your OneDrive folder in the file manager.
    Folder {
        /// Only print its location.
        #[arg(long)]
        print: bool,
    },
    /// Open the file or folder on onedrive.live.com.
    Online {
        path: Option<PathBuf>,
    },
    /// List a file's previous versions.
    Versions {
        path: PathBuf,
    },
    /// Print the path of a thumbnail of a photo or video, getting OneDrive's
    /// if the file is online-only (it isn't downloaded).
    Thumbnail {
        path: PathBuf,
        /// normal (128 px), large (256), x-large (512), xx-large (1024) or pixels.
        #[arg(long, default_value = "large")]
        size: String,
    },
    /// Restore a previous version (ID from `axentra-onedrive versions`).
    Restore {
        path: PathBuf,
        version: String,
    },
    /// Pause syncing for 2, 8 or 24 hours.
    Pause {
        #[arg(default_value_t = 2)]
        hours: u32,
    },
    Resume,
    /// Check for changes now.
    Sync,
    /// Re-read the settings file.
    Reload,
    /// Stop the sync service.
    Quit,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Exit quietly when piped into `head` and the like.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,axentra_onedrive_core=info,fuser=warn".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let paths = Paths::user();
    match cli.cmd {
        Cmd::Setup {
            client_id,
            tenant,
            folder,
        } => setup(&paths, client_id, tenant, folder).await,
        Cmd::Login { device } => login(&paths, device).await,
        Cmd::Logout { yes } => logout(&paths, yes).await,
        Cmd::Daemon => {
            let result = axentra_onedrive_core::daemon::run(paths, Default::default()).await;
            if let Some(NeedsUser(why)) = result.as_ref().err().and_then(|e| e.downcast_ref()) {
                // Say why rather than leave the tray waiting for a service
                // that can't start until this is fixed.
                let why =
                    format!("{why}\n\nOnce that's fixed, start it again with:\n{START_AGAIN}");
                eprintln!("Error: {why}");
                Notifier::new(true)
                    .send("Axentra OneDrive can't start", &why)
                    .await;
                std::process::exit(EXIT_NEEDS_USER);
            }
            result
        }
        Cmd::Status => status().await,
        Cmd::Activity { n } => {
            let p = proxy().await?;
            let items: Vec<serde_json::Value> = serde_json::from_str(&p.activity(n).await?)?;
            for a in items {
                let when = chrono::DateTime::from_timestamp(a["ts"].as_i64().unwrap_or(0), 0)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%b %-d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                println!(
                    "{when}  {:<15} {}  ({})",
                    a["kind"].as_str().unwrap_or(""),
                    a["path"].as_str().unwrap_or(""),
                    a["detail"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
        Cmd::Pin { paths } => {
            proxy().await?.pin(abs_all(paths)?).await?;
            println!("These will always be kept on this device.");
            Ok(())
        }
        Cmd::Unpin { paths } => Ok(proxy().await?.unpin(abs_all(paths)?).await?),
        Cmd::Free { paths } => {
            let kept = proxy().await?.free_up_space(abs_all(paths)?).await?;
            if kept > 0 {
                println!("Freed up space. {kept} files have changes that haven't synced yet, so they stay local.");
            } else {
                println!("Freed up space. The files are still available online.");
            }
            Ok(())
        }
        Cmd::Share { path, edit } => {
            let link = proxy().await?.share_link(abs(path)?, edit).await?;
            println!("{link}");
            Ok(())
        }
        Cmd::Folder { print } => {
            // Where it's mounted now: a new location only applies on restart.
            let dir = match service().await.ok().flatten() {
                Some(p) => serde_json::from_str::<Status>(&p.status().await?)?.mount_point,
                None => Config::load(&paths.config_file)?
                    .mount_path()
                    .display()
                    .to_string(),
            };
            if !print {
                let _ = std::process::Command::new("xdg-open").arg(&dir).status();
            }
            println!("{dir}");
            Ok(())
        }
        Cmd::Online { path } => {
            let p = proxy().await?;
            let target = match path {
                Some(p) => abs(p)?,
                None => serde_json::from_str::<Status>(&p.status().await?)?.mount_point,
            };
            let url = p.web_url(target).await?;
            let _ = std::process::Command::new("xdg-open").arg(&url).status();
            println!("{url}");
            Ok(())
        }
        Cmd::Versions { path } => {
            let v: Vec<serde_json::Value> =
                serde_json::from_str(&proxy().await?.versions(abs(path)?).await?)?;
            for (i, ver) in v.iter().enumerate() {
                println!(
                    "{:<8} {}  {:>10}  {}{}",
                    ver["id"].as_str().unwrap_or(""),
                    ver["lastModifiedDateTime"].as_str().unwrap_or(""),
                    human_size(ver["size"].as_u64().unwrap_or(0)),
                    ver["lastModifiedBy"]["user"]["displayName"]
                        .as_str()
                        .unwrap_or(""),
                    if i == 0 { "  (current)" } else { "" }
                );
            }
            Ok(())
        }
        Cmd::Thumbnail { path, size } => {
            let thumb = proxy().await?.get_thumbnail(abs(path)?, size).await?;
            if thumb.is_empty() {
                bail!("There's no thumbnail of this file");
            }
            println!("{thumb}");
            Ok(())
        }
        Cmd::Restore { path, version } => {
            proxy().await?.restore_version(abs(path)?, version).await?;
            println!("Restored. The current version was kept in the history.");
            Ok(())
        }
        Cmd::Pause { hours } => {
            proxy().await?.pause(hours * 3600).await?;
            println!("Syncing is paused for {hours} hours.");
            Ok(())
        }
        Cmd::Resume => Ok(proxy().await?.resume().await?),
        Cmd::Sync => Ok(proxy().await?.sync_now().await?),
        Cmd::Reload => {
            proxy().await?.reload().await?;
            note_folder_move(&paths).await
        }
        Cmd::Quit => Ok(proxy().await?.quit().await?),
    }
}

async fn proxy() -> Result<AxentraOneDriveProxy<'static>> {
    let conn = zbus::Connection::session().await?;
    let p = AxentraOneDriveProxy::new(&conn).await?;
    // Fail early with a clear message if the service isn't running.
    p.status().await.context(
        "the Axentra OneDrive service isn't running (start it with `systemctl --user start axentra-onedrive`)",
    )?;
    Ok(p)
}

/// The sync service, or `None` when it isn't running.
async fn service() -> Result<Option<AxentraOneDriveProxy<'static>>> {
    let conn = zbus::Connection::session().await?;
    let bus = zbus::fdo::DBusProxy::new(&conn).await?;
    if !bus.name_has_owner(DBUS_NAME.try_into()?).await? {
        return Ok(None);
    }
    Ok(Some(AxentraOneDriveProxy::new(&conn).await?))
}

/// The service's own words, without the D-Bus error name in front.
fn remote(e: zbus::Error) -> anyhow::Error {
    match e {
        zbus::Error::MethodError(_, Some(detail), _) => anyhow!(detail),
        e => e.into(),
    }
}

/// A new folder location set in config.toml takes effect when the service
/// restarts: say so if it's running somewhere else.
async fn note_folder_move(paths: &Paths) -> Result<()> {
    let Some(p) = service().await.ok().flatten() else {
        return Ok(());
    };
    let st: Status = serde_json::from_str(&p.status().await?)?;
    let wanted = Config::load(&paths.config_file)?.mount_path();
    if Path::new(&st.mount_point) != wanted {
        println!(
            "Your OneDrive moves to {} when the sync service restarts:\n  \
             systemctl --user restart axentra-onedrive",
            wanted.display()
        );
    }
    Ok(())
}

/// `..` is worked out here, as the shell's `cd ..` would.
fn abs(p: PathBuf) -> Result<String> {
    Ok(normalize(&std::path::absolute(p)?).display().to_string())
}

fn abs_all(ps: Vec<PathBuf>) -> Result<Vec<String>> {
    if ps.is_empty() {
        bail!("give at least one file or folder");
    }
    ps.into_iter().map(abs).collect()
}

fn prompt(q: &str) -> Result<String> {
    print!("{q}");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

const REGISTRATION_HELP: &str = "\
Axentra OneDrive signs in through your own (free) Microsoft Entra app registration:

  1. Open https://entra.microsoft.com > App registrations > New registration
  2. Name it \"Axentra OneDrive\". Under supported account types choose
     \"Accounts in any organizational directory and personal Microsoft accounts\".
  3. Redirect URI: platform \"Public client/native (mobile & desktop)\",
     URI  http://localhost
  4. Register, then under Authentication turn on \"Allow public client flows\".
  5. Under API permissions add Microsoft Graph delegated permissions
     Files.ReadWrite.All, User.Read and offline_access.
  6. Copy the Application (client) ID from Overview.
";

async fn setup(
    paths: &Paths,
    client_id: Option<String>,
    tenant: Option<String>,
    folder: Option<String>,
) -> Result<()> {
    configure(paths, client_id, tenant, folder, &mut prompt)?;
    note_folder_move(paths).await
}

/// Save what `setup` was given, asking with `ask` for the app registration
/// when it's needed.
fn configure(
    paths: &Paths,
    client_id: Option<String>,
    tenant: Option<String>,
    folder: Option<String>,
    ask: &mut dyn FnMut(&str) -> Result<String>,
) -> Result<()> {
    let mut cfg = Config::load(&paths.config_file)?;
    let client_id = match client_id {
        Some(c) => c,
        // `setup --folder` or `--tenant` alone changes just that.
        None if !cfg.client_id.is_empty() && (folder.is_some() || tenant.is_some()) => {
            cfg.client_id.clone()
        }
        // Run again on its own, as after a sign-in that failed for a wrong
        // ID: Enter keeps the one there is.
        None if !cfg.client_id.is_empty() => {
            println!("Paste another Application (client) ID, or press Enter to keep this one.");
            let typed = ask(&format!("Application (client) ID [{}]: ", cfg.client_id))?;
            if typed.is_empty() {
                cfg.client_id.clone()
            } else {
                typed
            }
        }
        None => {
            println!("{REGISTRATION_HELP}");
            ask("Application (client) ID: ")?
        }
    };
    if client_id.is_empty() {
        bail!("a client ID is required");
    }
    cfg.client_id = client_id;
    if let Some(t) = tenant {
        cfg.tenant = t;
    }
    if let Some(f) = folder {
        cfg.mount_point = folder_setting(&f)?;
    }
    cfg.save(&paths.config_file)?;
    if paths.tokens_file.exists() {
        println!("Saved {}.", paths.config_file.display());
    } else {
        println!(
            "Saved {}. Next: axentra-onedrive login",
            paths.config_file.display()
        );
    }
    Ok(())
}

/// The folder as config.toml keeps it: `~/…` as typed, anything else made
/// absolute from where the command runs (not from where the service does).
fn folder_setting(f: &str) -> Result<String> {
    if f == "~" || f.starts_with("~/") {
        return Ok(f.to_string());
    }
    Ok(normalize(&std::path::absolute(f)?).display().to_string())
}

/// "Unlink this PC". It deletes changes that haven't uploaded yet, so it
/// asks first, as Settings does.
async fn logout(paths: &Paths, yes: bool) -> Result<()> {
    let unsynced = match service().await? {
        Some(p) => {
            let st: Status = serde_json::from_str(&p.status().await?)?;
            confirm_unlink(&st, yes)?;
            p.sign_out().await.map_err(remote)?;
            st.pending
        }
        // Not running: unlink here, the same way the service would (this
        // also removes the photo thumbnails only its database knows about).
        None => {
            let cfg = Config::load(&paths.config_file).unwrap_or_default();
            let engine = Engine::new(
                paths.clone(),
                cfg,
                Options {
                    notifications: false,
                },
            )?;
            let st = engine.status();
            confirm_unlink(&st, yes)?;
            engine.sign_out().await?;
            st.pending
        }
    };
    if unsynced > 0 {
        println!(
            "This PC is unlinked. What had uploaded is still on OneDrive; the copies on this PC \
             are removed, with {} that hadn't uploaded.",
            changes(unsynced)
        );
    } else {
        println!("This PC is unlinked. Your files are still on OneDrive; the copies on this PC are removed.");
    }
    Ok(())
}

fn changes(n: u64) -> String {
    if n == 1 {
        "1 change".into()
    } else {
        format!("{n} changes")
    }
}

fn confirm_unlink(st: &Status, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    println!(
        "Unlink this PC?\n\nFiles that are only online stay on OneDrive. Local copies are removed \
         from this PC, and changes that haven't uploaded yet are lost."
    );
    if st.pending > 0 {
        println!("\n{}", unsynced_warning(st));
    }
    let answer = prompt("\nUnlink account? [y/N] ")?;
    if !matches!(answer.to_lowercase().as_str(), "y" | "yes") {
        bail!("this PC is still linked (pass --yes to unlink without asking)");
    }
    Ok(())
}

fn unsynced_warning(st: &Status) -> String {
    let mut s = format!(
        "{} {} uploaded yet and will be lost.",
        changes(st.pending),
        if st.pending == 1 { "hasn't" } else { "haven't" }
    );
    for t in st.transfers.iter().filter(|t| t.upload) {
        s.push_str(&format!("\n  ↑ {} (uploading)", t.name));
    }
    s
}

async fn login(paths: &Paths, device: bool) -> Result<()> {
    let cfg = Config::load(&paths.config_file)?;
    if cfg.client_id.is_empty() {
        bail!("run `axentra-onedrive setup` first to connect an app registration");
    }
    let auth_cfg = cfg.auth();
    let tokens = if device {
        auth::login_device_code(&auth_cfg, |code| {
            println!(
                "{}",
                if code.message.is_empty() {
                    format!(
                        "Go to {} and enter the code {}",
                        code.verification_uri, code.user_code
                    )
                } else {
                    code.message.clone()
                }
            );
        })
        .await?
    } else {
        auth::login_browser(&auth_cfg, |url| {
            println!("Opening your browser to sign in. If it doesn't open, visit:\n\n{url}\n");
            let _ = std::process::Command::new("xdg-open")
                .arg(url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        })
        .await?
    };
    // Keep the sign-in aside until it's known to be for the OneDrive this PC
    // syncs: another account's files must not be synced into this one's.
    let pending = paths.tokens_file.with_file_name("tokens-new.json");
    let store = Arc::new(TokenStore::open(&pending, auth_cfg));
    store.set(tokens).await?;
    let graph = Graph::new(&cfg.graph_base, store);
    let (name, email) = match check_account(&graph).await {
        Ok(who) => who,
        Err(e) => {
            let _ = std::fs::remove_file(&pending);
            // The tray's Sign in runs this with no terminal to show the error.
            Notifier::new(true)
                .send("Couldn't sign in", &format!("{e:#}"))
                .await;
            return Err(e);
        }
    };
    std::fs::rename(&pending, &paths.tokens_file)?;
    println!("Signed in as {name} ({email}).");
    if let Ok(conn) = zbus::Connection::session().await {
        if let Ok(p) = AxentraOneDriveProxy::new(&conn).await {
            let _ = p.reload().await;
        }
    }
    println!(
        "Your OneDrive will appear at {}",
        cfg.mount_path().display()
    );
    Ok(())
}

/// Who signed in, once the running service has agreed it's the account this
/// PC syncs. (If it isn't running, it checks when it starts.)
async fn check_account(graph: &Graph) -> Result<(String, String)> {
    let me = graph.me().await?;
    let drive = graph.drive().await?;
    let email = me.mail.or(me.user_principal_name).unwrap_or_default();
    if let Some(p) = service().await.ok().flatten() {
        let drive_type = drive.drive_type.unwrap_or_default();
        p.check_account(drive.id, drive_type, email.clone())
            .await
            .map_err(remote)?;
    }
    Ok((me.display_name.unwrap_or_default(), email))
}

async fn status() -> Result<()> {
    let p = proxy().await?;
    let st: Status = serde_json::from_str(&p.status().await?)?;
    let acct: Account = serde_json::from_str(&p.account().await?)?;
    if acct.signed_in {
        println!("{}  ·  {} <{}>", acct.title(), acct.name, acct.email);
    }
    println!("{}", st.message);
    if !st.detail.is_empty() {
        println!("  {}", st.detail);
    }
    for t in &st.transfers {
        let arrow = if t.upload { "↑" } else { "↓" };
        println!(
            "  {arrow} {}  {} of {}",
            t.name,
            human_size(t.done),
            human_size(t.total)
        );
    }
    for i in &st.issues {
        println!("  ✗ {}: {}", i.path, i.message);
    }
    if acct.total > 0 {
        println!(
            "{} of {} used  ·  {} on this device",
            human_size(acct.used),
            human_size(acct.total),
            human_size(p.local_usage().await.unwrap_or(0))
        );
    }
    println!("Folder: {}", st.mount_point);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axentra_onedrive_core::status::Transfer;

    #[test]
    fn paths_from_the_command_line_are_worked_out() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(abs("/a/b/..".into()).unwrap(), "/a");
        assert_eq!(
            abs("x/../y".into()).unwrap(),
            cwd.join("y").display().to_string()
        );
        assert_eq!(folder_setting("~/OneDrive").unwrap(), "~/OneDrive");
        assert_eq!(
            folder_setting("OneDrive2/").unwrap(),
            cwd.join("OneDrive2").display().to_string()
        );
    }

    #[test]
    fn setup_moves_the_folder_without_asking_for_the_app_again() {
        let dir = std::env::temp_dir().join(format!("axentra-cli-test-{}", std::process::id()));
        let paths = Paths::under(&dir);
        let cfg = Config {
            client_id: "app".into(),
            ..Default::default()
        };
        cfg.save(&paths.config_file).unwrap();
        let mut never = |q: &str| -> Result<String> { panic!("asked {q:?}") };
        configure(&paths, None, None, Some("/srv/OneDrive".into()), &mut never).unwrap();
        let cfg = Config::load(&paths.config_file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(cfg.client_id, "app");
        assert_eq!(cfg.mount_point, "/srv/OneDrive");
    }

    #[test]
    fn setup_on_its_own_can_replace_the_app() {
        let dir = std::env::temp_dir().join(format!("axentra-cli-app-{}", std::process::id()));
        let paths = Paths::under(&dir);
        let cfg = Config {
            client_id: "wrong-id".into(),
            ..Default::default()
        };
        cfg.save(&paths.config_file).unwrap();
        let client_id = |answer: &'static str| {
            let mut asked = Vec::new();
            let mut ask = |q: &str| -> Result<String> {
                asked.push(q.to_string());
                Ok(answer.into())
            };
            configure(&paths, None, None, None, &mut ask).unwrap();
            (Config::load(&paths.config_file).unwrap().client_id, asked)
        };
        // Enter keeps the one there is, which the question shows.
        let (kept, asked) = client_id("");
        assert_eq!(kept, "wrong-id");
        assert_eq!(asked, ["Application (client) ID [wrong-id]: "]);
        // Pasting another replaces it.
        assert_eq!(client_id("right-id").0, "right-id");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unlinking_warns_about_changes_that_would_be_lost() {
        let st = Status {
            pending: 2,
            transfers: vec![Transfer {
                name: "report.docx".into(),
                upload: true,
                done: 0,
                total: 10,
            }],
            ..Default::default()
        };
        let warning = unsynced_warning(&st);
        assert!(
            warning.starts_with("2 changes haven't uploaded yet"),
            "{warning}"
        );
        assert!(warning.contains("report.docx"), "{warning}");
        let one = Status {
            pending: 1,
            ..Default::default()
        };
        assert!(unsynced_warning(&one).starts_with("1 change hasn't"));
    }
}
