//! Makes the tray panel behave like the Windows one on X11 desktops: it
//! opens just above the tray icon, stays out of the taskbar and window
//! switcher, and closes when you click elsewhere (see `app.rs`). Wayland
//! doesn't let apps place their windows, so there it opens where the
//! compositor puts it.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask};
use x11rb::rust_connection::RustConnection;

/// Space between the panel and the taskbar or screen edge, as on Windows.
const GAP: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn contains_x(&self, x: i32) -> bool {
        (self.x..self.x + self.w).contains(&x)
    }

    /// How far `(x, y)` is from this rectangle (0 inside).
    fn distance(&self, (x, y): (i32, i32)) -> i64 {
        let dx = (self.x - x).max(x - (self.x + self.w - 1)).max(0) as i64;
        let dy = (self.y - y).max(y - (self.y + self.h - 1)).max(0) as i64;
        dx * dx + dy * dy
    }
}

pub fn is_x11() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some()
}

/// Where a `w` x `h` panel goes for a tray click at `anchor` (or, with no
/// click, the bottom-right corner of the screen holding `anchor`): above the
/// taskbar if it's at the bottom, below it if it's at the top, centred on
/// the icon but kept on screen.
pub fn place(w: i32, h: i32, anchor: (i32, i32), areas: &[Rect]) -> (i32, i32) {
    // The monitor above or below the click (the taskbar is outside its work area).
    let Some(area) = areas
        .iter()
        .min_by_key(|a| (!a.contains_x(anchor.0), a.distance(anchor)))
    else {
        return (anchor.0 - w / 2, anchor.1 - h - GAP);
    };
    let x = (anchor.0 - w / 2)
        .min(area.x + area.w - w - GAP)
        .max(area.x + GAP);
    let below_middle = anchor.1 >= area.y + area.h / 2;
    let y = if below_middle {
        area.y + area.h - h - GAP
    } else {
        area.y + GAP
    };
    (x, y.max(area.y))
}

fn atom(conn: &RustConnection, name: &str) -> Option<u32> {
    Some(
        conn.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom,
    )
}

fn cardinals(conn: &RustConnection, root: u32, name: &str) -> Vec<u32> {
    let Some(prop) = atom(conn, name) else {
        return Vec::new();
    };
    conn.get_property(false, root, prop, AtomEnum::CARDINAL, 0, 4096)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect()))
        .unwrap_or_default()
}

fn rects(v: &[u32]) -> Vec<Rect> {
    v.as_chunks::<4>()
        .0
        .iter()
        .map(|c| Rect {
            x: c[0] as i32,
            y: c[1] as i32,
            w: c[2] as i32,
            h: c[3] as i32,
        })
        .collect()
}

/// The usable area of each monitor (the screen minus panels and taskbars).
pub fn work_areas() -> Vec<Rect> {
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return Vec::new();
    };
    let root = conn.setup().roots[screen].root;
    let desktop = cardinals(&conn, root, "_NET_CURRENT_DESKTOP")
        .first()
        .copied()
        .unwrap_or(0);
    // Mutter publishes one work area per monitor for GTK; other window
    // managers only the combined one.
    let per_monitor = rects(&cardinals(
        &conn,
        root,
        &format!("_GTK_WORKAREAS_D{desktop}"),
    ));
    if !per_monitor.is_empty() {
        return per_monitor;
    }
    let all = rects(&cardinals(&conn, root, "_NET_WORKAREA"));
    all.get(desktop as usize).copied().into_iter().collect()
}

/// The primary monitor's rectangle, for opening without a tray click.
pub fn primary_monitor() -> Option<Rect> {
    use x11rb::protocol::randr::ConnectionExt as _;
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    let monitors = conn
        .randr_get_monitors(root, true)
        .ok()?
        .reply()
        .ok()?
        .monitors;
    let m = monitors.iter().find(|m| m.primary).or(monitors.first())?;
    Some(Rect {
        x: m.x.into(),
        y: m.y.into(),
        w: m.width.into(),
        h: m.height.into(),
    })
}

/// Open the hidden `window` at (x, y). Marking the position as chosen by the
/// user stops the window manager from centring it when it's shown.
pub fn set_open_position(window: u32, x: i32, y: i32) {
    use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
    use x11rb::protocol::xproto::ConfigureWindowAux;
    let Ok((conn, _)) = x11rb::connect(None) else {
        return;
    };
    let mut hints = WmSizeHints::get_normal_hints(&conn, window)
        .ok()
        .and_then(|c| c.reply().ok())
        .flatten()
        .unwrap_or_default();
    hints.position = Some((WmSizeHintsSpecification::UserSpecified, x, y));
    if let Ok(c) = hints.set_normal_hints(&conn, window) {
        let _ = c.check();
    }
    let moved = conn.configure_window(window, &ConfigureWindowAux::new().x(x).y(y));
    if let Ok(c) = moved {
        let _ = c.check();
    }
}

pub fn is_mapped(window: u32) -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    conn.get_window_attributes(window)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|a| a.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
}

/// Keep `window` out of the taskbar and window switcher. Sets the state
/// directly while the window is hidden and asks the window manager once it
/// is shown, so call it on both sides of showing.
pub fn keep_out_of_taskbar(window: u32) {
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen].root;
    let (Some(state), Some(skip_taskbar), Some(skip_pager), Some(above)) = (
        atom(&conn, "_NET_WM_STATE"),
        atom(&conn, "_NET_WM_STATE_SKIP_TASKBAR"),
        atom(&conn, "_NET_WM_STATE_SKIP_PAGER"),
        atom(&conn, "_NET_WM_STATE_ABOVE"),
    ) else {
        return;
    };
    // Each request waits for the server: it may drop requests still queued
    // when a connection closes.
    let sent = if is_mapped(window) {
        // _NET_WM_STATE_ADD, two states, "from an application".
        let msg = ClientMessageEvent::new(32, window, state, [1, skip_taskbar, skip_pager, 1, 0]);
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            msg,
        )
    } else {
        use x11rb::wrapper::ConnectionExt as _;
        conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            window,
            state,
            AtomEnum::ATOM,
            &[skip_taskbar, skip_pager, above],
        )
    };
    if let Ok(c) = sent {
        let _ = c.check();
    }
}

/// Ask the window manager to give `window` the focus, as the Windows panel
/// has it when it opens.
pub fn activate(window: u32) {
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen].root;
    let Some(active) = atom(&conn, "_NET_ACTIVE_WINDOW") else {
        return;
    };
    // Sent as a tool acting for the user ("pager"), since it answers a click
    // on the tray icon.
    let msg = ClientMessageEvent::new(32, window, active, [2, 0, 0, 0, 0]);
    let sent = conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        msg,
    );
    if let Ok(c) = sent {
        let _ = c.check();
    }
    // Mutter still refuses that without the click's timestamp, which the tray
    // host doesn't pass on, so also take the focus directly (ICCCM lets a
    // client focus its own window).
    let focused = conn.set_input_focus(
        x11rb::protocol::xproto::InputFocus::PARENT,
        window,
        x11rb::CURRENT_TIME,
    );
    if let Ok(c) = focused {
        let _ = c.check();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A laptop screen with a 48 px taskbar along the bottom, and an
    // external monitor without one to its right.
    const AREAS: [Rect; 2] = [
        Rect {
            x: 2560,
            y: 0,
            w: 2560,
            h: 1440,
        },
        Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1552,
        },
    ];

    #[test]
    fn opens_above_the_tray_icon() {
        // A click on a tray icon near the right end of the taskbar.
        assert_eq!(place(360, 600, (2380, 1576), &AREAS), (2188, 940));
    }

    #[test]
    fn stays_on_screen_near_the_corner() {
        assert_eq!(place(360, 600, (2550, 1590), &AREAS), (2188, 940));
        assert_eq!(place(360, 600, (5, 1590), &AREAS), (12, 940));
    }

    #[test]
    fn without_a_click_uses_the_bottom_right_corner() {
        assert_eq!(place(360, 600, (2559, 1599), &AREAS), (2188, 940));
    }

    #[test]
    fn a_taskbar_at_the_top_opens_downwards() {
        let top = [Rect {
            x: 0,
            y: 32,
            w: 1920,
            h: 1048,
        }];
        assert_eq!(place(360, 600, (1800, 16), &top), (1548, 44));
    }

    #[test]
    fn the_other_monitor() {
        assert_eq!(place(360, 600, (5000, 1430), &AREAS), (4748, 828));
    }
}
