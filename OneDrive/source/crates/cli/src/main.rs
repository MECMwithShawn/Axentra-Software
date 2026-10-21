//! `axentra-onedrive`: set up, sign in, run the sync service, and control it.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axentra_onedrive_core::config::{Config, Paths};
use axentra_onedrive_core::ipc::AxentraOneDriveProxy;
use axentra_onedrive_core::status::{human_size, Account, Status};
use axentra_onedrive_graph::{auth, Graph, TokenStore};
use clap::{Parser, Subcommand};

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
    Logout,
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
        } => setup(&paths, client_id, tenant, folder),
        Cmd::Login { device } => login(&paths, device).await,
        Cmd::Logout => {
            let p = proxy().await?;
            p.sign_out().await?;
            println!("This PC is unlinked. Your files are still on OneDrive.");
            Ok(())
        }
        Cmd::Daemon => axentra_onedrive_core::daemon::run(paths, Default::default()).await,
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
            let dir = Config::load(&paths.config_file)?.mount_path();
            if !print {
                let _ = std::process::Command::new("xdg-open").arg(&dir).status();
            }
            println!("{}", dir.display());
            Ok(())
        }
        Cmd::Online { path } => {
            let p = proxy().await?;
            let target = match path {
                Some(p) => abs(p)?,
                None => Config::load(&paths.config_file)?
                    .mount_path()
                    .display()
                    .to_string(),
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
        Cmd::Reload => Ok(proxy().await?.reload().await?),
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

fn abs(p: PathBuf) -> Result<String> {
    Ok(std::path::absolute(p)?.display().to_string())
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

fn setup(
    paths: &Paths,
    client_id: Option<String>,
    tenant: Option<String>,
    folder: Option<String>,
) -> Result<()> {
    let mut cfg = Config::load(&paths.config_file)?;
    let client_id = match client_id {
        Some(c) => c,
        None => {
            println!("{REGISTRATION_HELP}");
            prompt("Application (client) ID: ")?
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
        cfg.mount_point = f;
    }
    cfg.save(&paths.config_file)?;
    println!(
        "Saved {}. Next: axentra-onedrive login",
        paths.config_file.display()
    );
    Ok(())
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
    let store = Arc::new(TokenStore::open(&paths.tokens_file, auth_cfg));
    store.set(tokens).await?;
    let graph = Graph::new(&cfg.graph_base, store);
    let me = graph.me().await?;
    println!(
        "Signed in as {} ({}).",
        me.display_name.unwrap_or_default(),
        me.mail.or(me.user_principal_name).unwrap_or_default()
    );
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
