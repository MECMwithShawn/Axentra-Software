//! Client-side window decorations (docs/spec/shell.md §3.1 #3, #5, #12 and
//! §12.4): the header doubles as the title bar.
//!
//! The UI never touches the native window. Whoever hosts it provides a
//! [`WindowControls`] context (the desktop crate, backed by the tao window);
//! the shell then draws caption buttons in the header, makes the header's
//! empty space a drag region and puts thin resize handles around the window.
//! Without the context (liveview, or Settings › General › Appearance › "Use
//! the system title bar") the header is a plain app header.
//!
//! Mouse handling lives in a small page script ([`TITLEBAR_JS`]) so clicks
//! and double-clicks stay with the page: a press on a drag region becomes a
//! window move only once the pointer has moved a few pixels with the button
//! held (like GTK header bars), a second press toggles maximise (or whatever
//! the desktop's double-click action is), and presses on caption buttons or
//! resize handles never take focus from what the user was working in.

use dioxus::prelude::*;

use crate::kit::Icon;

/// One caption button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// Which caption buttons appear on which side of the header, in order.
///
/// Parsed from GNOME's `org.gnome.desktop.wm.preferences button-layout` (or
/// GTK's `gtk-decoration-layout`, same format): `appmenu:minimize,maximize,close`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonLayout {
    start: [Option<WindowButton>; 3],
    end: [Option<WindowButton>; 3],
}

impl Default for ButtonLayout {
    /// Minimize, Maximize, Close at the right, as on Windows and most desktops.
    fn default() -> Self {
        use WindowButton::*;
        ButtonLayout { start: [None; 3], end: [Some(Minimize), Some(Maximize), Some(Close)] }
    }
}

impl ButtonLayout {
    /// No caption buttons at all.
    pub const NONE: ButtonLayout = ButtonLayout { start: [None; 3], end: [None; 3] };

    /// Parse a GNOME / GTK button layout. Buttons before the colon go to the
    /// start (left) side and buttons after it to the end (right) side; with no
    /// colon everything is on the start side, as in GTK. `appmenu`, `menu`,
    /// `icon`, `spacer` and unknown names are skipped, a button named twice
    /// keeps its first place, and surrounding whitespace and the quotes
    /// `gsettings get` prints are ignored. Buttons that are not listed are not
    /// shown (GNOME's own default, `appmenu:close`, has only Close).
    pub fn parse(desc: &str) -> ButtonLayout {
        let desc = desc.trim().trim_matches(|c| c == '\'' || c == '"').trim();
        let mut out = ButtonLayout::NONE;
        let mut seen: Vec<WindowButton> = Vec::with_capacity(3);
        for (side, part) in desc.splitn(2, ':').enumerate() {
            let row = if side == 0 { &mut out.start } else { &mut out.end };
            let mut n = 0;
            for token in part.split(',') {
                let b = match token.trim() {
                    "minimize" => WindowButton::Minimize,
                    "maximize" => WindowButton::Maximize,
                    "close" => WindowButton::Close,
                    _ => continue,
                };
                if seen.contains(&b) {
                    continue;
                }
                seen.push(b);
                row[n] = Some(b);
                n += 1;
            }
        }
        out
    }

    /// Buttons at the start (left) of the header, in order.
    pub fn start(&self) -> impl Iterator<Item = WindowButton> + '_ {
        self.start.iter().flatten().copied()
    }

    /// Buttons at the end (right) of the header, in order.
    pub fn end(&self) -> impl Iterator<Item = WindowButton> + '_ {
        self.end.iter().flatten().copied()
    }

    /// The buttons on one side (`end` = right).
    pub fn side(&self, end: bool) -> impl Iterator<Item = WindowButton> + '_ {
        let row = if end { &self.end } else { &self.start };
        row.iter().flatten().copied()
    }
}

/// What a double-click (or middle-click) on empty title-bar space does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlebarAction {
    ToggleMaximize,
    Minimize,
    Nothing,
}

impl TitlebarAction {
    /// A GNOME `action-double-click-titlebar` / `action-middle-click-titlebar`
    /// value (quotes allowed). Actions the app cannot perform itself (`lower`,
    /// `menu`, `toggle-shade`) do nothing; the one-axis maximise variants
    /// maximise fully. `None` for values GNOME does not define.
    pub fn parse(v: &str) -> Option<TitlebarAction> {
        match v.trim().trim_matches('\'') {
            "toggle-maximize" | "toggle-maximize-horizontally" | "toggle-maximize-vertically" => Some(TitlebarAction::ToggleMaximize),
            "minimize" => Some(TitlebarAction::Minimize),
            "none" | "lower" | "menu" | "toggle-shade" => Some(TitlebarAction::Nothing),
            _ => None,
        }
    }
}

/// The desktop's title-bar preferences, read once at startup by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TitlebarPrefs {
    pub layout: ButtonLayout,
    pub double_click: TitlebarAction,
    pub middle_click: TitlebarAction,
}

impl Default for TitlebarPrefs {
    /// GNOME's defaults for the click actions, Windows-style buttons.
    fn default() -> Self {
        TitlebarPrefs { layout: ButtonLayout::default(), double_click: TitlebarAction::ToggleMaximize, middle_click: TitlebarAction::Nothing }
    }
}

/// The window edge or corner a resize starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl ResizeEdge {
    pub const ALL: [ResizeEdge; 8] = [
        ResizeEdge::North,
        ResizeEdge::South,
        ResizeEdge::East,
        ResizeEdge::West,
        ResizeEdge::NorthEast,
        ResizeEdge::NorthWest,
        ResizeEdge::SouthEast,
        ResizeEdge::SouthWest,
    ];

    /// Short code used in class names and page messages ("n", "se", ...).
    pub fn code(self) -> &'static str {
        match self {
            ResizeEdge::North => "n",
            ResizeEdge::South => "s",
            ResizeEdge::East => "e",
            ResizeEdge::West => "w",
            ResizeEdge::NorthEast => "ne",
            ResizeEdge::NorthWest => "nw",
            ResizeEdge::SouthEast => "se",
            ResizeEdge::SouthWest => "sw",
        }
    }

    pub fn from_code(code: &str) -> Option<ResizeEdge> {
        ResizeEdge::ALL.into_iter().find(|e| e.code() == code)
    }
}

/// The window edges the compositor has tiled: placed against a screen edge
/// or a neighbouring window, as in GNOME's half-screen snapping (GDK's
/// `*_TILED` window states). Like GTK's own decorations, a tiled window has
/// square corners, and its tiled edges have no frame line and no resize
/// handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TiledEdges {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl TiledEdges {
    pub const NONE: TiledEdges = TiledEdges { top: false, right: false, bottom: false, left: false };
    /// Tiled without per-edge detail (older compositors): treat every edge as tiled.
    pub const ALL: TiledEdges = TiledEdges { top: true, right: true, bottom: true, left: true };

    pub fn any(self) -> bool {
        self.top || self.right || self.bottom || self.left
    }

    /// Whether the resize handle at `edge` sits on a tiled edge (and is left out).
    pub fn covers(self, edge: ResizeEdge) -> bool {
        match edge {
            ResizeEdge::North => self.top,
            ResizeEdge::South => self.bottom,
            ResizeEdge::East => self.right,
            ResizeEdge::West => self.left,
            ResizeEdge::NorthEast => self.top || self.right,
            ResizeEdge::NorthWest => self.top || self.left,
            ResizeEdge::SouthEast => self.bottom || self.right,
            ResizeEdge::SouthWest => self.bottom || self.left,
        }
    }

    /// The tiled edges as a space-separated list ("top left"), for `data-tiled`.
    pub fn names(self) -> String {
        [(self.top, "top"), (self.right, "right"), (self.bottom, "bottom"), (self.left, "left")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The window frame the page draws (`data-window` on `.ax-app`): square and
/// edge to edge when maximised, square with partial frame when tiled,
/// rounded with a full frame and resize handles otherwise.
pub fn frame_state(maximized: bool, tiled: TiledEdges) -> &'static str {
    if maximized {
        "maximized"
    } else if tiled.any() {
        "tiled"
    } else {
        "restored"
    }
}

/// Native window operations, provided as context by the host (see the
/// module docs). All fields are cheap handles, so the struct is `Copy`.
#[derive(Clone, Copy, PartialEq)]
pub struct WindowControls {
    pub minimize: Callback,
    pub toggle_maximize: Callback,
    pub close: Callback,
    /// Start an interactive move; call while the primary button is held.
    pub start_drag: Callback,
    /// Start an interactive resize from an edge; call while the primary button is held.
    pub start_resize: Callback<ResizeEdge>,
    /// Kept in sync by the host with the window's real state (including
    /// maximise and restore done by the compositor, e.g. Super+Up).
    pub is_maximized: Signal<bool>,
    /// Edges tiled by the compositor (none when the host cannot tell).
    pub tiled: Signal<TiledEdges>,
    pub layout: ButtonLayout,
    pub double_click: TitlebarAction,
    pub middle_click: TitlebarAction,
}

impl WindowControls {
    /// Perform a title-bar click action.
    pub fn run(self, action: TitlebarAction) {
        match action {
            TitlebarAction::ToggleMaximize => self.toggle_maximize.call(()),
            TitlebarAction::Minimize => self.minimize.call(()),
            TitlebarAction::Nothing => {}
        }
    }

    /// [`frame_state`] for the window's current state (subscribes to it).
    pub fn frame(self) -> &'static str {
        frame_state((self.is_maximized)(), (self.tiled)())
    }
}

/// The host's window controls, when the header is the title bar.
pub fn use_window_controls() -> Option<WindowControls> {
    try_use_context::<WindowControls>()
}

/// Page side of the title bar: drag regions, title-bar clicks, resize
/// handles and caption-button presses. Installed once per page; a remounted
/// shell only swaps the channel the messages go to.
///
/// Once a move starts, the window manager owns the pointer until the button
/// is released, so the page usually never sees that release. The script
/// therefore remembers only what it must (`moved`, `armed`) and lets go of
/// it as soon as the gesture is over.
const TITLEBAR_JS: &str = r#"
window.axWindowSend = (m) => dioxus.send(m);
if (!window.axWindowInstalled) {
  window.axWindowInstalled = true;
  const send = (m) => window.axWindowSend && window.axWindowSend(m);
  const THRESHOLD = 4;
  // A title-bar press that becomes a move once the pointer travels THRESHOLD px.
  let down = null;
  // This press moved the window: its own mouse click (if the window manager
  // lets the release through) must not reach the page. Keyboard and scripted
  // clicks (detail 0) always pass, and the flag ends with the gesture.
  let moved = false;
  // The last title-bar press ended as a plain click, so the next one there
  // may complete a double-click. A press that moved the window never pairs
  // with the next: the window followed the pointer, so WebKit sees both
  // presses on the same spot and counts a double-click (GTK resets its
  // click count when a move starts for the same reason).
  let armed = false;
  const region = (t) => t instanceof Element && t.hasAttribute("data-ax-drag-region");
  // A modal overlay's, flyout's or menu's strip over the header moves the
  // window too (a press without movement still reaches it as a click, which
  // dismisses it as usual).
  const overHeader = (t, e) => {
    if (!t.matches(".ax-dialog-overlay, .ax-acct-overlay, .ax-flyout-backdrop, .ax-menu-backdrop, .ax-ribbon-overlay-backdrop")) return false;
    const h = document.querySelector(".ax-header");
    return !!h && e.clientY < h.getBoundingClientRect().bottom;
  };
  document.addEventListener("mousedown", (e) => {
    moved = false;
    const pairs = armed;
    armed = false;
    down = null;
    const t = e.target instanceof Element ? e.target : null;
    if (!t) return;
    const edge = t.closest("[data-ax-resize]");
    if (edge) {
      e.preventDefault();
      e.stopPropagation();
      if (e.button === 0) send("resize:" + edge.getAttribute("data-ax-resize"));
      return;
    }
    // Caption buttons click as usual but leave focus where it was.
    if (t.closest(".ax-wc-btn")) {
      e.preventDefault();
      return;
    }
    if (e.button === 0 && overHeader(t, e)) {
      down = { x: e.screenX, y: e.screenY };
      return;
    }
    if (!region(t)) return;
    // Empty title-bar space: no focus change, no text selection.
    e.preventDefault();
    if (e.button === 0) {
      if (e.detail >= 2 && pairs) {
        send("double");
      } else {
        armed = true;
        down = { x: e.screenX, y: e.screenY };
      }
    } else if (e.button === 1) {
      send("middle");
    }
  }, true);
  document.addEventListener("mousemove", (e) => {
    if (!(e.buttons & 1)) {
      // The button is up again: whatever the press was, it is over.
      down = null;
      moved = false;
      return;
    }
    if (down && Math.abs(e.screenX - down.x) + Math.abs(e.screenY - down.y) >= THRESHOLD) {
      down = null;
      armed = false;
      moved = true;
      send("drag");
    }
  }, true);
  document.addEventListener("mouseup", () => {
    down = null;
    // The release's click (if any) follows in the same task.
    if (moved) setTimeout(() => { moved = false; }, 0);
  }, true);
  document.addEventListener("click", (e) => {
    if (!moved) return;
    moved = false;
    if (e.detail === 0) return;
    e.preventDefault();
    e.stopPropagation();
  }, true);
  document.addEventListener("keydown", () => { moved = false; armed = false; }, true);
  window.addEventListener("blur", () => { down = null; armed = false; });
}
await new Promise(() => {});
"#;

/// Route the page's title-bar messages to the window controls (no-op
/// without them). Call once, from the shell root.
pub fn use_titlebar_events(controls: Option<WindowControls>) {
    use_future(move || async move {
        let Some(wc) = controls else { return };
        let mut ev = document::eval(TITLEBAR_JS);
        loop {
            let msg = match ev.recv::<String>().await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(?e, "title bar channel closed");
                    break;
                }
            };
            tracing::debug!(%msg, "title bar message");
            match msg.as_str() {
                "drag" => wc.start_drag.call(()),
                "double" => wc.run(wc.double_click),
                "middle" => wc.run(wc.middle_click),
                m => {
                    if let Some(edge) = m.strip_prefix("resize:").and_then(ResizeEdge::from_code) {
                        wc.start_resize.call(edge);
                    }
                }
            }
        }
    });
    // Diagnostics (AXENTRA_TITLEBAR_PROBE=1): report what the page sees at the
    // window edge and send one harmless message through the title bar channel.
    use_future(move || async move {
        if controls.is_none() || std::env::var_os("AXENTRA_TITLEBAR_PROBE").is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let probe = document::eval(PROBE_JS);
        match probe.join::<String>().await {
            Ok(v) => tracing::info!(probe = %v, "title bar probe"),
            Err(e) => tracing::warn!(?e, "title bar probe failed"),
        }
        let _ = document::eval("window.axWindowSend && window.axWindowSend('probe'); return 1;").join::<i32>().await;
    });
}

const PROBE_JS: &str = r#"
const h = document.querySelector('[data-ax-resize=e]');
const at = document.elementFromPoint(window.innerWidth - 2, Math.round(window.innerHeight / 2));
const head = document.elementFromPoint(Math.round(window.innerWidth * 0.62), 24);
return JSON.stringify({
  installed: !!window.axWindowInstalled,
  sendFn: typeof window.axWindowSend,
  handles: document.querySelectorAll('[data-ax-resize]').length,
  handleRect: h ? [h.getBoundingClientRect().x, h.getBoundingClientRect().width, h.getBoundingClientRect().height] : null,
  handlePointerEvents: h ? getComputedStyle(h).pointerEvents : null,
  atRightEdge: at ? at.tagName + '.' + at.className + (at.hasAttribute('data-ax-resize') ? ' [resize]' : '') : null,
  atHeader: head ? head.tagName + '.' + head.className + (head.hasAttribute('data-ax-drag-region') ? ' [drag]' : '') : null,
  dragRegions: document.querySelectorAll('[data-ax-drag-region]').length,
  inner: [window.innerWidth, window.innerHeight],
});
"#;

/// Caption buttons for one side of the header (§3.1 #12): 46×48, 16px
/// glyphs, in the desktop's order. Renders nothing when that side is empty.
#[component]
pub fn CaptionButtons(end: bool) -> Element {
    let Some(wc) = use_window_controls() else { return rsx! {} };
    let buttons: Vec<WindowButton> = wc.layout.side(end).collect();
    if buttons.is_empty() {
        return rsx! {};
    }
    let maximized = (wc.is_maximized)();
    rsx! {
        div {
            class: if end { "ax-wc ax-wc-end" } else { "ax-wc ax-wc-start" },
            role: "group",
            "aria-label": "Window controls",
            "data-ax-drag-region": "true",
            for b in buttons {
                {
                    let (label, icon, class) = match b {
                        WindowButton::Minimize => ("Minimize", "subtract", "ax-wc-min"),
                        WindowButton::Maximize if maximized => ("Restore", "square_multiple", "ax-wc-max"),
                        WindowButton::Maximize => ("Maximize", "square", "ax-wc-max"),
                        WindowButton::Close => ("Close", "dismiss", "ax-wc-close"),
                    };
                    rsx! {
                        button {
                            key: "{class}",
                            r#type: "button",
                            class: "ax-wc-btn {class}",
                            title: label,
                            "aria-label": label,
                            onclick: move |_| match b {
                                WindowButton::Minimize => wc.minimize.call(()),
                                WindowButton::Maximize => wc.toggle_maximize.call(()),
                                WindowButton::Close => wc.close.call(()),
                            },
                            Icon { name: icon, size: 16 }
                        }
                    }
                }
            }
        }
    }
}

/// Invisible resize handles along the window edges and corners (6px edges,
/// 10px corners; `--ax-resize-edge`, `--ax-resize-corner`). None while the
/// window is maximised, and none on edges the compositor has tiled.
/// Presses are handled by [`TITLEBAR_JS`].
#[component]
pub fn ResizeHandles() -> Element {
    let Some(wc) = use_window_controls() else { return rsx! {} };
    if (wc.is_maximized)() {
        tracing::debug!("resize handles hidden: maximized");
        return rsx! {};
    }
    let tiled = (wc.tiled)();
    tracing::debug!(?tiled, "resize handles shown");
    rsx! {
        div { class: "ax-resize-handles", "aria-hidden": "true",
            for edge in ResizeEdge::ALL.into_iter().filter(|e| !tiled.covers(*e)) {
                div { key: "{edge.code()}", class: "ax-resize ax-resize-{edge.code()}", "data-ax-resize": edge.code() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WindowButton::*;

    fn sides(l: ButtonLayout) -> (Vec<WindowButton>, Vec<WindowButton>) {
        (l.start().collect(), l.end().collect())
    }

    #[test]
    fn gnome_layouts() {
        assert_eq!(sides(ButtonLayout::parse("'appmenu:minimize,maximize,close'\n")), (vec![], vec![Minimize, Maximize, Close]));
        assert_eq!(sides(ButtonLayout::parse("close,minimize,maximize:")), (vec![Close, Minimize, Maximize], vec![]));
        assert_eq!(sides(ButtonLayout::parse("close:maximize")), (vec![Close], vec![Maximize]));
        assert_eq!(sides(ButtonLayout::parse(":minimize,maximize,close")), (vec![], vec![Minimize, Maximize, Close]));
    }

    #[test]
    fn missing_buttons_stay_hidden() {
        // GNOME's upstream default has only Close.
        assert_eq!(sides(ButtonLayout::parse("'appmenu:close'")), (vec![], vec![Close]));
        assert_eq!(ButtonLayout::parse("appmenu:"), ButtonLayout::NONE);
        assert_eq!(ButtonLayout::parse("''"), ButtonLayout::NONE);
        assert_eq!(ButtonLayout::parse(""), ButtonLayout::NONE);
    }

    #[test]
    fn unknown_tokens_spaces_and_duplicates() {
        assert_eq!(sides(ButtonLayout::parse(" icon , menu:spacer, minimize ,bogus,close ")), (vec![], vec![Minimize, Close]));
        assert_eq!(sides(ButtonLayout::parse("close:close,maximize,maximize")), (vec![Close], vec![Maximize]));
        // Only the first colon splits the sides (GTK).
        assert_eq!(sides(ButtonLayout::parse("minimize:close:maximize")), (vec![Minimize], vec![]));
    }

    #[test]
    fn no_colon_means_start_side() {
        assert_eq!(sides(ButtonLayout::parse("minimize,maximize,close")), (vec![Minimize, Maximize, Close], vec![]));
    }

    #[test]
    fn default_layout_is_right_side_min_max_close() {
        assert_eq!(sides(ButtonLayout::default()), (vec![], vec![Minimize, Maximize, Close]));
        assert_eq!(ButtonLayout::default().side(true).count(), 3);
        assert_eq!(ButtonLayout::default().side(false).count(), 0);
    }

    #[test]
    fn titlebar_actions() {
        assert_eq!(TitlebarAction::parse("'toggle-maximize'\n"), Some(TitlebarAction::ToggleMaximize));
        assert_eq!(TitlebarAction::parse("'toggle-maximize-vertically'"), Some(TitlebarAction::ToggleMaximize));
        assert_eq!(TitlebarAction::parse("'minimize'"), Some(TitlebarAction::Minimize));
        assert_eq!(TitlebarAction::parse("'lower'"), Some(TitlebarAction::Nothing));
        assert_eq!(TitlebarAction::parse("'none'"), Some(TitlebarAction::Nothing));
        assert_eq!(TitlebarAction::parse("No such key"), None);
    }

    #[test]
    fn resize_codes_round_trip() {
        for e in ResizeEdge::ALL {
            assert_eq!(ResizeEdge::from_code(e.code()), Some(e));
        }
        assert_eq!(ResizeEdge::from_code("x"), None);
    }

    #[test]
    fn page_script_sends_what_the_listener_handles() {
        for m in ["\"drag\"", "\"double\"", "\"middle\"", "\"resize:\""] {
            assert!(TITLEBAR_JS.contains(m), "script lacks {m}");
        }
        assert!(TITLEBAR_JS.contains("data-ax-drag-region") && TITLEBAR_JS.contains("data-ax-resize"));
    }

    #[test]
    fn page_script_keeps_keyboard_clicks_and_ends_the_move_gesture() {
        // Only the moving press's own mouse click is eaten: keyboard and
        // scripted clicks have detail 0 and always pass.
        let click = TITLEBAR_JS.split("addEventListener(\"click\"").nth(1).expect("click listener");
        let pass = click.find("e.detail === 0) return").expect("detail-0 clicks pass");
        assert!(pass < click.find("preventDefault").unwrap());
        // The flag also ends with a key press or the pointer back without the button.
        assert!(TITLEBAR_JS.contains("addEventListener(\"keydown\""));
        // A press after a move is never the second half of a double-click.
        assert!(TITLEBAR_JS.contains("e.detail >= 2 && pairs"));
        assert!(!TITLEBAR_JS.contains("% 2"));
    }

    #[test]
    fn tiled_edges_hide_their_handles() {
        let left_half = TiledEdges { top: true, bottom: true, left: true, right: false };
        let kept: Vec<ResizeEdge> = ResizeEdge::ALL.into_iter().filter(|e| !left_half.covers(*e)).collect();
        assert_eq!(kept, vec![ResizeEdge::East]);
        assert!(ResizeEdge::ALL.into_iter().all(|e| TiledEdges::ALL.covers(e)));
        assert!(ResizeEdge::ALL.into_iter().all(|e| !TiledEdges::NONE.covers(e)));
        assert_eq!(left_half.names(), "top bottom left");
        assert_eq!(TiledEdges::NONE.names(), "");
    }

    #[test]
    fn frame_states() {
        let right = TiledEdges { right: true, ..TiledEdges::NONE };
        assert_eq!(frame_state(false, TiledEdges::NONE), "restored");
        assert_eq!(frame_state(false, right), "tiled");
        // Compositors also report maximised windows as tiled.
        assert_eq!(frame_state(true, TiledEdges::ALL), "maximized");
        assert_eq!(frame_state(true, TiledEdges::NONE), "maximized");
    }
}
