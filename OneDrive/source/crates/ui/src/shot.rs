//! Headless rendering of the UI to PNG (for docs and visual checks):
//! `AxentraOneDrive --screenshot <view> <out.png> [light|dark]`
//! Views: flyout, flyout-syncing, flyout-menu, flyout-paused,
//! flyout-signed-out, flyout-empty, settings-0 .. settings-4.

use std::rc::Rc;

use anyhow::{bail, Context, Result};
use axentra_onedrive_core::config::Config;
use axentra_onedrive_core::db::Activity;
use axentra_onedrive_core::status::{Account, Status, Transfer};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};

use crate::{present, Flyout, SettingsWindow, Theme};

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Render without a display, into one software window. `None` if this
/// thread already has a Slint platform.
pub fn headless() -> Option<Rc<MinimalSoftwareWindow>> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(window.clone()))).ok()?;
    Some(window)
}

const SCALE: f32 = 2.0;

fn demo_account() -> Account {
    Account {
        signed_in: true,
        name: "Alex Rivera".into(),
        email: "alex.rivera@outlook.com".into(),
        drive_type: "personal".into(),
        used: 1_932_735_283,
        total: 5 * 1024 * 1024 * 1024,
        remaining: 3_436_015_437,
        quota_state: "normal".into(),
        web_url: "https://onedrive.live.com/".into(),
    }
}

fn demo_activity(now: i64) -> Vec<Activity> {
    let a =
        |mins: i64, kind: &str, name: &str, folder: &str, is_dir: bool, detail: &str| Activity {
            seq: 0,
            ts: now - mins * 60,
            kind: kind.into(),
            name: name.into(),
            folder: folder.into(),
            path: format!("/{folder}/{name}"),
            is_dir,
            detail: detail.into(),
        };
    vec![
        a(0, "uploaded", "Q3 report.docx", "Documents", false, ""),
        a(3, "downloaded", "Budget 2026.xlsx", "Documents", false, ""),
        a(
            12,
            "added_online",
            "Team offsite.jpg",
            "Pictures",
            false,
            "",
        ),
        a(
            41,
            "uploaded",
            "Kickoff slides.pptx",
            "Client projects 2026 - Northwind",
            false,
            "",
        ),
        a(95, "uploaded", "Invoices", "Documents", true, ""),
        a(
            180,
            "updated_online",
            "Lease agreement.pdf",
            "Documents",
            false,
            "",
        ),
        a(300, "deleted", "old-notes.txt", "Desktop", false, ""),
        a(1500, "uploaded", "main.rs", "Projects", false, ""),
    ]
}

fn demo_status(view: &str) -> Status {
    let base = Status {
        state: "up_to_date".into(),
        message: "Your files are synced".into(),
        mount_point: "/home/alex/OneDrive".into(),
        last_sync: Some(0),
        ..Default::default()
    };
    match view {
        "flyout-syncing" => Status {
            state: "syncing".into(),
            message: "Uploading 3 files".into(),
            detail: "18.4 MB of 52.0 MB".into(),
            uploading: 3,
            bytes_done: 18_400_000,
            bytes_total: 52_000_000,
            transfers: vec![Transfer {
                name: "Holiday video.mp4".into(),
                upload: true,
                done: 18_400_000,
                total: 52_000_000,
            }],
            ..base
        },
        "flyout-paused" => Status {
            state: "paused".into(),
            message: "Syncing is paused".into(),
            detail: "Syncing will resume at 4:30 PM".into(),
            ..base
        },
        "flyout-signed-out" => Status {
            state: "signed_out".into(),
            message: "You're not signed in".into(),
            detail: "Sign in to start syncing your files".into(),
            ..base
        },
        _ => base,
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let (Some(view), Some(out)) = (args.first(), args.get(1)) else {
        bail!("usage: AxentraOneDrive --screenshot <view> <out.png> [light|dark]");
    };
    let dark = args.get(2).map(String::as_str) != Some("light");
    if view == "tray" {
        return tray_sheet(out, dark);
    }

    let window = headless().context("setting up headless rendering")?;
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: SCALE,
        });

    let now = chrono::Utc::now().timestamp();
    let (w, h, _keep): (u32, u32, Box<dyn std::any::Any>) = if let Some(size) =
        view.strip_prefix("logo-")
    {
        // Square app icon; SCALE doubles it, so "logo-256" gives 512 px.
        let ui = crate::LogoWindow::new()?;
        let size: u32 = size.parse().context("logo size")?;
        ui.show()?;
        (size, size, Box::new(ui))
    } else if view == "versions" {
        let ui = crate::VersionsWindow::new()?;
        ui.global::<Theme>().invoke_use_dark(dark);
        ui.set_file_name("Q3 report.docx".into());
        let t = |d: i64| (chrono::Utc::now() - chrono::Duration::hours(d)).to_rfc3339();
        let json = serde_json::json!([
            { "id": "4.0", "size": 48213, "lastModifiedDateTime": t(0), "lastModifiedBy": { "user": { "displayName": "Alex Rivera" } } },
            { "id": "3.0", "size": 47102, "lastModifiedDateTime": t(26), "lastModifiedBy": { "user": { "displayName": "Alex Rivera" } } },
            { "id": "2.0", "size": 39876, "lastModifiedDateTime": t(75), "lastModifiedBy": { "user": { "displayName": "Jordan Lee" } } },
            { "id": "1.0", "size": 12034, "lastModifiedDateTime": t(240), "lastModifiedBy": { "user": { "displayName": "Alex Rivera" } } },
        ]);
        ui.set_versions(slint::ModelRc::new(slint::VecModel::from(
            crate::versions::rows(&json.to_string()),
        )));
        ui.show()?;
        (540, 460, Box::new(ui))
    } else if let Some(page) = view.strip_prefix("settings-") {
        let ui = SettingsWindow::new()?;
        ui.global::<Theme>().invoke_use_dark(dark);
        let cfg = Config {
            mount_point: "/home/alex/OneDrive".into(),
            ..Default::default()
        };
        present::apply_settings(
            &ui,
            &cfg,
            &demo_status("flyout"),
            &demo_account(),
            Some(734_003_200),
        );
        ui.set_page(page.parse().context("settings page number")?);
        ui.show()?;
        (880, 620, Box::new(ui))
    } else {
        let ui = Flyout::new()?;
        ui.global::<Theme>().invoke_use_dark(dark);
        let mut acct = demo_account();
        let mut activity = demo_activity(now);
        if view == "flyout-signed-out" {
            acct = Account::default();
            activity.clear();
        }
        if view == "flyout-empty" {
            activity.clear();
        }
        present::apply_flyout(&ui, &demo_status(view), &acct, Some(&activity));
        // As it looks on Wayland, where it has a close button.
        ui.set_show_close(view == "flyout-wayland");
        ui.show()?;
        (360, 600, Box::new(ui))
    };

    let (pw, ph) = ((w as f32 * SCALE) as u32, (h as f32 * SCALE) as u32);
    window.window().set_size(PhysicalSize::new(pw, ph));
    if view == "flyout-menu" {
        if let Some(ui) = _keep.downcast_ref::<Flyout>() {
            ui.invoke_show_menu();
        }
    }
    slint::platform::update_timers_and_animations();
    let mut buffer = vec![PremultipliedRgbaColor::default(); (pw * ph) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, pw as usize);
    });

    let file = std::fs::File::create(out).with_context(|| format!("creating {out}"))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = buffer
        .iter()
        .flat_map(|p| {
            let a = p.alpha.max(1) as u16;
            let un = |c: u8| ((c as u16 * 255 + a / 2) / a).min(255) as u8;
            [un(p.red), un(p.green), un(p.blue), p.alpha]
        })
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}

/// The tray icon in each state at 64 px, on a panel-coloured strip.
fn tray_sheet(out: &str, dark: bool) -> Result<()> {
    let states = ["up_to_date", "syncing", "paused", "error", "signed_out"];
    let (size, pad) = (64u32, 16u32);
    let (w, h) = (states.len() as u32 * (size + pad) + pad, size + 2 * pad);
    let bg = if dark {
        [0x24u8, 0x24, 0x24]
    } else {
        [0xf0u8, 0xf0, 0xf0]
    };
    let mut img = vec![0u8; (w * h * 4) as usize];
    for p in img.chunks_mut(4) {
        p.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
    }
    for (i, st) in states.iter().enumerate() {
        let icon = crate::tray::render(
            st,
            size as i32,
            axentra_onedrive_core::config::icon_accent("ice"),
        );
        let ox = pad + i as u32 * (size + pad);
        for y in 0..size {
            for x in 0..size {
                let s = ((y * size + x) * 4) as usize;
                let [a, r, g, b] = [
                    icon.data[s],
                    icon.data[s + 1],
                    icon.data[s + 2],
                    icon.data[s + 3],
                ];
                let d = (((pad + y) * w + ox + x) * 4) as usize;
                let af = a as f32 / 255.0;
                for (k, c) in [r, g, b].iter().enumerate() {
                    img[d + k] = (*c as f32 * af + img[d + k] as f32 * (1.0 - af)).round() as u8;
                }
            }
        }
    }
    let file = std::fs::File::create(out)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img)?;
    Ok(())
}
