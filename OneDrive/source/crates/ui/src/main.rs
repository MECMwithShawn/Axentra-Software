//! `AxentraOneDrive`: the tray icon, activity flyout and settings window.
//!
//! `AxentraOneDrive`             tray icon (click for the panel)
//! `AxentraOneDrive --panel`     also open the panel right away
//! `AxentraOneDrive --settings`  open Settings
//! `AxentraOneDrive --versions <file>`  version history of a file
//! `AxentraOneDrive --screenshot <view> <out.png> [light|dark]`

slint::include_modules!();

mod app;
mod icons;
mod present;
mod shot;
mod tray;
mod versions;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,arboard=error".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--screenshot") => shot::run(&args[1..]),
        Some("--settings") => app::run(true, false),
        Some("--panel") => app::run(false, true),
        Some("--versions") => versions::run(args.get(1).cloned().unwrap_or_default()),
        Some("--help") | Some("-h") => {
            println!("usage: AxentraOneDrive [--panel | --settings | --screenshot <view> <out.png> [light|dark]]");
            Ok(())
        }
        _ => app::run(false, false),
    }
}
