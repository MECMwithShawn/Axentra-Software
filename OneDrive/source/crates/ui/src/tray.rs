//! The notification-area icon (StatusNotifierItem), drawn in code so it
//! needs no installed icon theme. Blue cloud when connected, grey when
//! paused or signed out, with a badge for syncing, paused and problems.

use std::sync::Mutex;

use ksni::menu::{StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::Cmd;

pub struct AxentraOneDriveTray {
    pub state: String,
    pub tooltip: String,
    /// Accent of the chosen app icon colour.
    pub accent: [u8; 3],
    pub tx: UnboundedSender<Cmd>,
}

impl AxentraOneDriveTray {
    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

impl ksni::Tray for AxentraOneDriveTray {
    fn id(&self) -> String {
        "axentra-onedrive".into()
    }

    fn title(&self) -> String {
        "Axentra OneDrive".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // ksni asks on every update to see whether the icon changed, and
        // rendering is supersampled, so keep each one drawn.
        type Drawn = Vec<((String, [u8; 3]), Vec<Icon>)>;
        static DRAWN: Mutex<Drawn> = Mutex::new(Vec::new());
        let key = (self.state.clone(), self.accent);
        let mut drawn = DRAWN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, icons)) = drawn.iter().find(|(k, _)| *k == key) {
            return icons.clone();
        }
        let icons: Vec<Icon> = [22, 32, 48, 64]
            .iter()
            .map(|&s| render(&self.state, s, self.accent))
            .collect();
        drawn.push((key, icons.clone()));
        icons
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Axentra OneDrive".into(),
            description: self.tooltip.clone(),
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    fn status(&self) -> ksni::Status {
        if self.state == "error" {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        // The tray host passes where it was clicked; (0, 0) means it didn't say.
        self.send(Cmd::ToggleFlyout((x != 0 || y != 0).then_some((x, y))));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let item = |label: &str, cmd: fn() -> Cmd| -> MenuItem<Self> {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |t: &mut Self| t.send(cmd())),
                ..Default::default()
            }
            .into()
        };
        let pause_menu: MenuItem<Self> = if self.state == "paused" {
            item("Resume syncing", || Cmd::Resume)
        } else if self.state == "stopped" {
            item("Start Axentra OneDrive", || Cmd::StartService)
        } else {
            SubMenu {
                label: "Pause syncing".into(),
                submenu: vec![
                    item("2 hours", || Cmd::Pause(2)),
                    item("8 hours", || Cmd::Pause(8)),
                    item("24 hours", || Cmd::Pause(24)),
                ],
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Open Axentra OneDrive", || Cmd::ShowFlyout(None)),
            item("Open your OneDrive folder", || Cmd::OpenFolder),
            item("View online", || Cmd::ViewOnline),
            MenuItem::Separator,
            pause_menu,
            item("Settings", || Cmd::ShowSettings),
            MenuItem::Separator,
            item("Quit Axentra OneDrive", || Cmd::Quit),
        ]
    }
}

// ----- icon rasteriser ------------------------------------------------------

type Rgba = [f32; 4];

fn over(dst: Rgba, src: Rgba) -> Rgba {
    let a = src[3] + dst[3] * (1.0 - src[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mix = |i: usize| (src[i] * src[3] + dst[i] * dst[3] * (1.0 - src[3])) / a;
    [mix(0), mix(1), mix(2), a]
}

fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn in_circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> bool {
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn seg_dist(px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Sample the icon at (x, y) in the 24-unit design grid.
fn sample(state: &str, accent: [u8; 3], x: f32, y: f32) -> Rgba {
    let grey = matches!(state, "paused" | "offline" | "signed_out" | "stopped");
    let (top, bottom) = if grey {
        (hex(0xB9C0C8), hex(0x7C858F))
    } else {
        // A gradient around the app icon's accent: lighter at the top.
        let a = [
            accent[0] as f32 / 255.0,
            accent[1] as f32 / 255.0,
            accent[2] as f32 / 255.0,
        ];
        (lerp(a, [1.0, 1.0, 1.0], 0.45), lerp(a, hex(0x0E285A), 0.45))
    };
    let mut px: Rgba = [0.0; 4];

    let cloud = in_circle(x, y, 6.5, 14.5, 4.5)
        || in_circle(x, y, 17.5, 14.5, 4.5)
        || in_circle(x, y, 12.0, 10.0, 5.5)
        || ((6.5..=17.5).contains(&x) && (10.0..=19.0).contains(&y));
    if cloud {
        let c = lerp(top, bottom, ((y - 4.5) / 14.5).clamp(0.0, 1.0));
        px = [c[0], c[1], c[2], 1.0];
    }

    let badge = match state {
        "syncing" | "starting" => Some(0x0F6CBD),
        "error" => Some(0xD13438),
        "paused" => Some(0x6B7280),
        _ => None,
    };
    if let Some(color) = badge {
        let (cx, cy) = (18.0, 17.5);
        if in_circle(x, y, cx, cy, 6.2) {
            px = over(px, [1.0, 1.0, 1.0, 1.0]);
        }
        if in_circle(x, y, cx, cy, 5.0) {
            let c = hex(color);
            px = over(px, [c[0], c[1], c[2], 1.0]);
            let w = 0.85;
            let glyph = match state {
                "error" => {
                    seg_dist(x, y, (16.0, 15.5), (20.0, 19.5)) < w
                        || seg_dist(x, y, (20.0, 15.5), (16.0, 19.5)) < w
                }
                "paused" => {
                    seg_dist(x, y, (16.8, 15.3), (16.8, 19.7)) < w
                        || seg_dist(x, y, (19.2, 15.3), (19.2, 19.7)) < w
                }
                _ => {
                    // Two arrow arcs: a ring with gaps plus heads.
                    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                    let ang = (y - cy).atan2(x - cx).to_degrees();
                    let ring = (d - 2.6).abs() < 0.7
                        && !(-30.0..15.0).contains(&ang)
                        && !(150.0..195.0).contains(&ang)
                        && !(-195.0..-165.0).contains(&ang);
                    ring || seg_dist(x, y, (20.6, 15.2), (20.6, 17.0)) < 0.6
                        || seg_dist(x, y, (15.4, 18.0), (15.4, 19.8)) < 0.6
                }
            };
            if glyph {
                px = over(px, [1.0, 1.0, 1.0, 1.0]);
            }
        }
    }
    px
}

/// ARGB32 (network byte order) pixmap of `size` x `size`.
pub fn render(state: &str, size: i32, accent: [u8; 3]) -> Icon {
    const SS: i32 = 4;
    let scale = 24.0 / size as f32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    let s = sample(state, accent, x, y);
                    for i in 0..3 {
                        acc[i] += s[i] * s[3];
                    }
                    acc[3] += s[3];
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let un = |c: f32| {
                if acc[3] > 0.0 {
                    ((c / acc[3]) * 255.0).round() as u8
                } else {
                    0
                }
            };
            data.extend_from_slice(&[
                (a * 255.0).round() as u8,
                un(acc[0]),
                un(acc[1]),
                un(acc[2]),
            ]);
        }
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}
