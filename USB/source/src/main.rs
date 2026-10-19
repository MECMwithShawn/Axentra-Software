#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod gui;
use anyhow::{Context, Result};
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("Axentra USB: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--download") {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed))?;
        let mut last = std::time::Instant::now();
        let result = axentra_usb::download::download(
            args.get(2)
                .context("Usage: --download HTTPS_URL SHA256 NEW_OUTPUT.iso")?,
            args.get(3).context("Missing publisher SHA-256")?,
            &PathBuf::from(args.get(4).context("Missing new output filename")?),
            &mut |phase, done, total| {
                if last.elapsed().as_secs() >= 1 || (total > 0 && done == total) {
                    eprintln!("{phase}: {done} / {total} bytes");
                    last = std::time::Instant::now();
                }
            },
            &|| cancel.load(Ordering::Relaxed),
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--worker") {
        return axentra_usb::operation::worker(&PathBuf::from(
            args.get(2).context("Missing worker request")?,
        ));
    }
    if args.get(1).is_some_and(|a| a == "--inspect") {
        let path = PathBuf::from(args.get(2).context("Missing ISO path")?);
        let r = axentra_usb::image::inspect(&path, &mut |_, _, _| {}, &|| false)?;
        let report = serde_json::to_string_pretty(&r)?;
        if let Some(output) = args.get(3) {
            std::fs::write(output, report)?;
        } else {
            println!("{report}");
        }
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--list-devices") {
        let report = serde_json::to_string_pretty(&axentra_usb::device::list()?)?;
        if let Some(output) = args.get(2) {
            std::fs::write(output, report)?;
        } else {
            println!("{report}");
        }
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--hash-entry") {
        use sha2::{Digest, Sha256};
        let iso =
            axentra_usb::image::Iso::open(&PathBuf::from(args.get(2).context("Missing ISO")?))?;
        let entry = args.get(3).context("Missing entry path")?;
        let mut hash = Sha256::new();
        iso.stream(entry, &|| false, |b| {
            hash.update(b);
            Ok(())
        })?;
        let result = hex::encode(hash.finalize());
        if let Some(output) = args.get(4) {
            std::fs::write(output, result)?;
        } else {
            println!("{result}");
        }
        return Ok(());
    }
    let image = if args.get(1).is_some_and(|a| a == "--image") {
        args.get(2).map(PathBuf::from)
    } else {
        None
    };
    gui::launch(image).map_err(|e| anyhow::anyhow!(e.to_string()))
}
