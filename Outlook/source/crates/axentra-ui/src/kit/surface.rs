//! Resizable dialogs and movable in-app windows ("surfaces").
//!
//! A surface opts in with a stable size key ([`SurfaceSpec`]): `Dialog {
//! resize_key: "settings", .. }`, `AcctDialog { resize_key: .. }`, or
//! [`use_surface`] + [`SurfaceGrips`] on a container of its own (the compose
//! pop-out and the read window).
//!
//! * Pointer handling is page script (the surface section of `dom.js`): eight
//!   invisible grips (6px edges, 12px corners, `aria-hidden`, never
//!   focusable) straddle the surface's border. Dragging one moves that edge
//!   under the pointer; the surface switches from centred to an explicit
//!   rectangle when the gesture starts. A content-height surface whose
//!   width is dragged stays content-height (and vertically centred). There
//!   is no keyboard path: the default sizes fit, and Alt+F7 / Alt+F8 belong
//!   to the window manager.
//! * Sizes are clamped to the surface's minimum and to the window minus
//!   [`MARGIN`] (below the title bar when the header is the title bar).
//!   CSS does the window clamp (`.ax-surface` in `kit.css`), so a surface
//!   stays inside when the window shrinks and gets its size back when it
//!   grows again. A gesture keeps the sides it doesn't drag as they were
//!   saved ([`kept`]), so moving a squeezed surface doesn't shrink it.
//! * At the end of a gesture the page reports the rectangle; Rust keeps it
//!   for the open surface and saves the size (and, for movable surfaces,
//!   the position) under the key in the store's `shell.surfaces` entry.
//!   Modal dialogs open centred at the remembered size. Double-clicking any
//!   grip goes back to the default size (and forgets the saved one).
//!
//! The geometry functions below are the reference for the page script's
//! arithmetic (`dom.js`, "resizable surfaces"); keep the two in step.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Gap between a surface and the window edges, in CSS px.
pub const MARGIN: i32 = 16;
/// The store entry that holds every surface's saved geometry.
pub const STORE_KEY: &str = "shell.surfaces";
/// Saved sizes beyond this are treated as corrupt and clamped.
const MAX_SIDE: u32 = 16_384;

/// The page script for one surface: wait for the runtime, then route the
/// surface's reports to Rust until the surface leaves the page.
const WATCH_JS: &str = r#"
for (let i = 0; i < 200 && !(window.axShell && window.axShell.watchSurface); i++) await new Promise((r) => setTimeout(r, 25));
if (!(window.axShell && window.axShell.watchSurface)) return;
await window.axShell.watchSurface(/*ID*/, (m) => dioxus.send(m));
"#;

/// How a surface sizes, and whether it moves.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSpec {
    /// Stable key the size is saved under ("settings", "compose-popout", ...).
    pub key: String,
    /// Default width in px.
    pub width: u32,
    /// Default height in px; `None` fits the content (up to the window).
    pub height: Option<u32>,
    pub min_width: u32,
    pub min_height: u32,
    /// The surface can be moved by its `data-ax-surface-move` area, and its
    /// position is remembered (pop-out windows). Modal dialogs open centred.
    pub movable: bool,
}

impl SurfaceSpec {
    /// A surface with the default minimum for its default size:
    /// [`default_min`].
    pub fn new(key: impl Into<String>, width: u32, height: Option<u32>) -> Self {
        let (min_width, min_height) = default_min(width, height);
        SurfaceSpec { key: key.into(), width, height, min_width, min_height, movable: false }
    }

    pub fn min(mut self, width: Option<u32>, height: Option<u32>) -> Self {
        if let Some(w) = width {
            self.min_width = w;
        }
        if let Some(h) = height {
            self.min_height = h;
        }
        self
    }

    pub fn movable(mut self) -> Self {
        self.movable = true;
        self
    }

    /// `data-ax-default`: "1080 760", or "480 auto" for a content-height surface.
    pub fn default_attr(&self) -> String {
        match self.height {
            Some(h) => format!("{} {h}", self.width),
            None => format!("{} auto", self.width),
        }
    }

    /// `data-ax-min`: "720 480".
    pub fn min_attr(&self) -> String {
        format!("{} {}", self.min_width, self.min_height)
    }
}

/// Minimum size for a surface whose default is `width` × `height`: as wide
/// as the default up to 360px, as tall as the default up to 240px (a title,
/// a few rows and the buttons).
pub fn default_min(width: u32, height: Option<u32>) -> (u32, u32) {
    (width.min(360), height.unwrap_or(240).min(240))
}

/// A rectangle in CSS px (viewport coordinates).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn right(self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(self) -> i32 {
        self.y + self.h
    }
}

/// Where a surface may be: the viewport minus margins. `top` is larger
/// when the header is the window's title bar (surfaces stay below it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    /// A `vw` × `vh` viewport with [`MARGIN`] all round and `top` at the top.
    pub fn viewport(vw: i32, vh: i32, top: i32) -> Bounds {
        Bounds { left: MARGIN, top, right: vw - MARGIN, bottom: vh - MARGIN }
    }
    fn width(self) -> i32 {
        (self.right - self.left).max(0)
    }
    fn height(self) -> i32 {
        (self.bottom - self.top).max(0)
    }
}

/// The edge or corner a resize grip moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

impl Edge {
    pub const ALL: [Edge; 8] = [Edge::N, Edge::S, Edge::E, Edge::W, Edge::NE, Edge::NW, Edge::SE, Edge::SW];

    /// Code used in `data-ax-grip` and class names ("n", "se", ...).
    pub fn code(self) -> &'static str {
        match self {
            Edge::N => "n",
            Edge::S => "s",
            Edge::E => "e",
            Edge::W => "w",
            Edge::NE => "ne",
            Edge::NW => "nw",
            Edge::SE => "se",
            Edge::SW => "sw",
        }
    }

    pub fn from_code(code: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| e.code() == code)
    }

    fn north(self) -> bool {
        matches!(self, Edge::N | Edge::NE | Edge::NW)
    }
    fn south(self) -> bool {
        matches!(self, Edge::S | Edge::SE | Edge::SW)
    }
    fn east(self) -> bool {
        matches!(self, Edge::E | Edge::NE | Edge::SE)
    }
    fn west(self) -> bool {
        matches!(self, Edge::W | Edge::NW | Edge::SW)
    }
}

/// `v` limited to `lo..=hi`; when the range is empty the lower bound wins
/// (as CSS `clamp()`), so the window bounds beat the minimum size.
fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.min(hi).max(lo)
}

/// Drag `edge` of `start` by (`dx`, `dy`): that edge follows the pointer,
/// the opposite one stays, the size never drops below `min` (unless the
/// window is smaller) and the surface stays within `b`.
pub fn resize(start: Rect, edge: Edge, dx: i32, dy: i32, min: (u32, u32), b: Bounds) -> Rect {
    let min_w = (min.0 as i32).min(b.width());
    let min_h = (min.1 as i32).min(b.height());
    let (mut l, mut t, mut r, mut bt) = (start.x, start.y, start.right(), start.bottom());
    if edge.west() {
        l = clamp(l + dx, b.left, r - min_w);
    }
    if edge.east() {
        r = clamp(r + dx, l + min_w, b.right);
    }
    if edge.north() {
        t = clamp(t + dy, b.top, bt - min_h);
    }
    if edge.south() {
        bt = clamp(bt + dy, t + min_h, b.bottom);
    }
    Rect { x: l, y: t, w: r - l, h: bt - t }
}

/// Move `start` by (`dx`, `dy`), keeping it within `b`.
pub fn moved(start: Rect, dx: i32, dy: i32, b: Bounds) -> Rect {
    Rect { x: clamp(start.x + dx, b.left, b.right - start.w), y: clamp(start.y + dy, b.top, b.bottom - start.h), ..start }
}

/// Where a remembered rectangle shows in the current window: the size
/// capped to the window, the position pulled inside. This is what the
/// `.ax-surface` CSS computes; it never changes what is saved.
pub fn fit(r: Rect, b: Bounds) -> Rect {
    let w = r.w.min(b.width());
    let h = r.h.min(b.height());
    Rect { x: clamp(r.x, b.left, b.right - w), y: clamp(r.y, b.top, b.bottom - h), w, h }
}

/// What a gesture leaves: the sides it dragged as `shown` (the rectangle
/// on screen at the end, from [`resize`] or [`moved`]), the others as
/// `stored` (width, and height or `None` for a content-height surface),
/// which can be larger than what a small window shows. `edge` is `None`
/// for a move. So moving a surface in a small window, or dragging only its
/// width, never changes its remembered height.
pub fn kept(stored: (u32, Option<u32>), shown: Rect, edge: Option<Edge>) -> Geometry {
    let side = |v: i32| v.clamp(1, MAX_SIDE as i32) as u32;
    let horizontal = edge.is_some_and(|e| e.east() || e.west());
    let vertical = edge.is_some_and(|e| e.north() || e.south());
    Geometry {
        w: if horizontal { side(shown.w) } else { stored.0 },
        h: if vertical { Some(side(shown.h)) } else { stored.1 },
        pos: Some((shown.x, shown.y)),
    }
}

/// A surface's remembered geometry, as saved in the store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub w: u32,
    /// `None`: the surface keeps its content height (only its width was set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
}

/// The geometry of an open surface: `None` while it has its default size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub w: u32,
    /// `None`: content height (the spec's default height, if it has one).
    pub h: Option<u32>,
    /// Explicit position (after a gesture, or remembered for movable surfaces).
    pub pos: Option<(i32, i32)>,
}

/// What to show for a surface opening with `saved`: its size raised to the
/// minimum (and capped to a sane maximum); the position only for movable
/// surfaces. The window clamp happens in CSS.
pub fn restore(saved: Option<Saved>, spec: &SurfaceSpec) -> Option<Geometry> {
    let s = saved?;
    if s.w == 0 || s.h == Some(0) {
        return None;
    }
    let w = s.w.clamp(spec.min_width.min(MAX_SIDE), MAX_SIDE);
    let h = s.h.map(|h| h.clamp(spec.min_height.min(MAX_SIDE), MAX_SIDE));
    let pos = match (spec.movable, s.x, s.y) {
        (true, Some(x), Some(y)) => Some((x.clamp(-(MAX_SIDE as i32), MAX_SIDE as i32), y.clamp(-(MAX_SIDE as i32), MAX_SIDE as i32))),
        _ => None,
    };
    Some(Geometry { w, h, pos })
}

/// What gets saved for a surface after a gesture: modal dialogs keep only
/// their size.
pub fn to_saved(g: Geometry, movable: bool) -> Saved {
    let pos = g.pos.filter(|_| movable);
    Saved { w: g.w, h: g.h, x: pos.map(|p| p.0), y: pos.map(|p| p.1) }
}

/// Parse the store entry (missing or unreadable: nothing saved).
pub fn parse_saved(json: Option<&str>) -> BTreeMap<String, Saved> {
    json.and_then(|j| serde_json::from_str::<BTreeMap<String, serde_json::Value>>(j).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| Some((k, serde_json::from_value::<Saved>(v).ok()?))).collect())
        .unwrap_or_default()
}

/// `map` with `key` set to `value`, or removed for `None`.
pub fn with_saved(mut map: BTreeMap<String, Saved>, key: &str, value: Option<Saved>) -> BTreeMap<String, Saved> {
    match value {
        Some(v) => {
            map.insert(key.to_string(), v);
        }
        None => {
            map.remove(key);
        }
    }
    map
}

fn load(key: &str) -> Option<Saved> {
    let json = crate::state::store().kv_get(STORE_KEY).ok().flatten();
    parse_saved(json.as_deref()).get(key).copied()
}

fn save(key: &str, value: Option<Saved>) {
    let store = crate::state::store();
    let json = store.kv_get(STORE_KEY).ok().flatten();
    let map = with_saved(parse_saved(json.as_deref()), key, value);
    match serde_json::to_string(&map) {
        Ok(j) => {
            if let Err(e) = store.kv_set(STORE_KEY, &j) {
                tracing::warn!(?e, key, "couldn't save the window size");
            }
        }
        Err(e) => tracing::warn!(?e, "couldn't encode window sizes"),
    }
}

/// A report from the page: the rectangle at the end of a gesture (`h`
/// null for a content-height surface), or a reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
struct Report {
    #[serde(default)]
    reset: bool,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    w: f64,
    #[serde(default)]
    h: Option<f64>,
}

impl Report {
    fn geometry(self) -> Option<Geometry> {
        if self.reset || self.w.is_nan() || self.w < 1.0 || self.h.is_some_and(|h| h.is_nan() || h < 1.0) {
            return None;
        }
        let side = |v: f64| (v.round() as i64).clamp(1, MAX_SIDE as i64) as u32;
        let pos = |v: f64| (v.round() as i64).clamp(-(MAX_SIDE as i64), MAX_SIDE as i64) as i32;
        Some(Geometry { w: side(self.w), h: self.h.map(side), pos: Some((pos(self.x), pos(self.y))) })
    }
}

/// An open surface: its spec and current geometry. On the surface element:
/// `class: "ax-surface"` (plus `ax-surface-float` for a fixed in-app
/// window), `style:` [`Surface::style`], `..surface.attributes()`, and
/// [`SurfaceGrips`] inside.
#[derive(Clone, Copy, PartialEq)]
pub struct Surface {
    spec: Signal<Option<SurfaceSpec>>,
    geom: Signal<Option<Geometry>>,
}

impl Surface {
    pub fn enabled(&self) -> bool {
        self.spec.read().is_some()
    }

    pub fn spec(&self) -> Option<SurfaceSpec> {
        self.spec.read().clone()
    }

    pub fn geometry(&self) -> Option<Geometry> {
        *self.geom.read()
    }

    /// Has an explicit height (`data-ax-sized`).
    pub fn sized(&self) -> bool {
        self.geometry().is_some_and(|g| g.h.is_some()) || self.spec.read().as_ref().is_some_and(|s| s.height.is_some())
    }

    /// Has an explicit position (`data-ax-placed`).
    pub fn placed(&self) -> bool {
        self.geometry().is_some_and(|g| g.pos.is_some())
    }

    /// The data attributes the page script and CSS read (none when the
    /// surface is not resizable). Always the same names, sorted, so Dioxus
    /// diffs them as a spread.
    pub fn attributes(&self) -> Vec<Attribute> {
        let spec = self.spec();
        let on = |b: bool| b.then_some("true");
        let mut attrs = vec![
            Attribute::new("data-ax-movable", on(spec.as_ref().is_some_and(|s| s.movable)), None, false),
            Attribute::new("data-ax-placed", on(self.placed()), None, false),
            Attribute::new("data-ax-size-default", spec.as_ref().map(|s| s.default_attr()), None, false),
            Attribute::new("data-ax-size-min", spec.as_ref().map(|s| s.min_attr()), None, false),
            Attribute::new("data-ax-sized", on(self.sized()), None, false),
            Attribute::new("data-ax-surface", spec.as_ref().map(|s| s.key.clone()), None, false),
        ];
        attrs.sort_by_key(|a| a.name);
        attrs
    }

    /// The custom properties the surface CSS reads (`--ax-sw`, `--ax-sh`,
    /// `--ax-sx`, `--ax-sy`). Empty for a surface that is not resizable.
    pub fn style(&self) -> String {
        let Some(spec) = self.spec() else { return String::new() };
        surface_style(&spec, self.geometry())
    }
}

/// [`Surface::style`] for a spec and geometry.
pub fn surface_style(spec: &SurfaceSpec, g: Option<Geometry>) -> String {
    let (w, h) = match g {
        Some(g) => (g.w, g.h.or(spec.height)),
        None => (spec.width, spec.height),
    };
    let mut s = format!("--ax-sw:{w}px;");
    if let Some(h) = h {
        s.push_str(&format!("--ax-sh:{h}px;"));
    }
    if let Some((x, y)) = g.and_then(|g| g.pos) {
        s.push_str(&format!("--ax-sx:{x}px;--ax-sy:{y}px;"));
    }
    s
}

/// Make the element with DOM id `dom_id` a resizable surface (`spec`
/// `None`: an ordinary fixed-size one). Loads the remembered size, listens
/// for the page's reports and saves them.
pub fn use_surface(dom_id: &str, spec: Option<SurfaceSpec>) -> Surface {
    let spec = use_signal(|| spec);
    let mut geom = use_signal(|| spec.peek().as_ref().and_then(|s| restore(load(&s.key), s)));
    let id = dom_id.to_string();
    use_future(move || {
        let id = id.clone();
        async move {
            let Some(s) = spec.peek().clone() else { return };
            let js = WATCH_JS.replace("/*ID*/", &super::dom::js_str(&id));
            let mut ev = document::eval(&js);
            loop {
                let report = match ev.recv::<Report>().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::debug!(?e, key = %s.key, "surface channel closed");
                        break;
                    }
                };
                let g = report.geometry();
                geom.set(g);
                save(&s.key, g.map(|g| to_saved(g, s.movable)));
            }
        }
    });
    Surface { spec, geom }
}

/// The eight invisible resize grips. Put them inside the surface element
/// (which must be positioned); the page script handles them.
#[component]
pub fn SurfaceGrips() -> Element {
    rsx! {
        div { class: "ax-grips", "aria-hidden": "true",
            for e in Edge::ALL {
                div { key: "{e.code()}", class: "ax-grip ax-grip-{e.code()}", "data-ax-grip": e.code() }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Bounds = Bounds { left: 16, top: 16, right: 1424, bottom: 884 }; // 1440×900

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn edges_follow_the_pointer_and_the_opposite_edge_stays() {
        let s = r(180, 70, 1080, 760);
        // East edge 50px right: only the width grows.
        assert_eq!(resize(s, Edge::E, 50, 9, (720, 480), VP), r(180, 70, 1130, 760));
        // West edge 100px right: left moves, right edge stays at 1260.
        let w = resize(s, Edge::W, 100, 0, (720, 480), VP);
        assert_eq!((w.x, w.right()), (280, 1260));
        // North-west corner: both the left and top edges move.
        assert_eq!(resize(s, Edge::NW, -40, -30, (720, 480), VP), r(140, 40, 1120, 790));
        // South edge up: only the height shrinks.
        assert_eq!(resize(s, Edge::S, 0, -100, (720, 480), VP), r(180, 70, 1080, 660));
    }

    #[test]
    fn resize_stops_at_the_minimum() {
        let s = r(180, 70, 1080, 760);
        let e = resize(s, Edge::E, -900, 0, (720, 480), VP);
        assert_eq!(e.w, 720);
        // Dragging the west edge too far right pins the left edge at right - min.
        let w = resize(s, Edge::W, 900, 0, (720, 480), VP);
        assert_eq!((w.x, w.w, w.right()), (540, 720, 1260));
        let n = resize(s, Edge::N, 0, 900, (720, 480), VP);
        assert_eq!((n.y, n.h, n.bottom()), (350, 480, 830));
    }

    #[test]
    fn resize_stays_inside_the_window_margins() {
        let s = r(180, 70, 1080, 760);
        assert_eq!(resize(s, Edge::E, 5000, 0, (720, 480), VP).right(), 1424);
        assert_eq!(resize(s, Edge::W, -5000, 0, (720, 480), VP).x, 16);
        assert_eq!(resize(s, Edge::N, 0, -5000, (720, 480), VP).y, 16);
        assert_eq!(resize(s, Edge::SE, 5000, 5000, (720, 480), VP), r(180, 70, 1244, 814));
        // Below the title bar when the header is the window's title bar.
        let csd = Bounds::viewport(1440, 900, 48);
        assert_eq!(resize(s, Edge::N, 0, -5000, (720, 480), csd).y, 48);
    }

    #[test]
    fn a_window_smaller_than_the_minimum_wins() {
        // 640×400 window: the usable area is 608×368, less than Settings' minimum.
        let b = Bounds::viewport(640, 400, MARGIN);
        let s = r(16, 16, 608, 368);
        let e = resize(s, Edge::W, 50, 0, (720, 480), b);
        assert_eq!((e.x, e.w), (16, 608));
        let e = resize(s, Edge::S, 0, -50, (720, 480), b);
        assert_eq!(e.h, 368);
    }

    #[test]
    fn moving_keeps_the_surface_inside() {
        let s = r(270, 100, 900, 700);
        assert_eq!(moved(s, 30, -20, VP), r(300, 80, 900, 700));
        assert_eq!(moved(s, -1000, -1000, VP), r(16, 16, 900, 700));
        assert_eq!(moved(s, 1000, 1000, VP), r(524, 184, 900, 700));
    }

    #[test]
    fn fit_shrinks_to_the_window_and_pulls_inside() {
        // Saved at 1440×900, the window is now 1024×700.
        let b = Bounds::viewport(1024, 700, MARGIN);
        assert_eq!(fit(r(500, 150, 900, 700), b), r(108, 16, 900, 668));
        // Already inside: unchanged.
        assert_eq!(fit(r(100, 100, 500, 400), b), r(100, 100, 500, 400));
        // Off the top-left.
        assert_eq!(fit(r(-50, -50, 300, 200), b), r(16, 16, 300, 200));
    }

    #[test]
    fn dragging_only_the_width_keeps_a_content_height_surface_content_high() {
        // Add account ("480 auto"), 228px of content, centred at 1440×900.
        let acct = SurfaceSpec::new("add-account", 480, None).min(Some(360), Some(200));
        let shown = r(480, 336, 480, 228);
        let min = (acct.min_width, acct.min_height);
        let e = kept((480, None), resize(shown, Edge::E, 100, 0, min, VP), Some(Edge::E));
        assert_eq!(e, Geometry { w: 580, h: None, pos: Some((480, 336)) });
        let w = kept((480, None), resize(shown, Edge::W, -60, 0, min, VP), Some(Edge::W));
        assert_eq!((w.w, w.h, w.pos), (540, None, Some((420, 336))));
        // Saved without a height, it reopens content-high at the new width.
        let saved = to_saved(e, false);
        assert_eq!(serde_json::to_string(&saved).unwrap(), r#"{"w":580}"#);
        let back = restore(parse_saved(Some(r#"{"add-account":{"w":580}}"#)).get("add-account").copied(), &acct);
        assert_eq!(back, Some(Geometry { w: 580, h: None, pos: None }));
        assert_eq!(surface_style(&acct, back), "--ax-sw:580px;");
        // An edge with a north or south side sets the height.
        let s = kept((480, None), resize(shown, Edge::S, 0, 50, min, VP), Some(Edge::S));
        assert_eq!((s.w, s.h), (480, Some(278)));
        let ne = kept((480, None), resize(shown, Edge::NE, 20, -20, min, VP), Some(Edge::NE));
        assert_eq!((ne.w, ne.h), (500, Some(248)));
        // A spec with a height keeps it when the saved entry has none.
        let settings = SurfaceSpec::new("settings", 1080, Some(760));
        assert_eq!(surface_style(&settings, Some(Geometry { w: 900, h: None, pos: None })), "--ax-sw:900px;--ax-sh:760px;");
    }

    #[test]
    fn a_gesture_in_a_small_window_keeps_the_remembered_size() {
        // The read window (900×760) in a 1100×620 window below a 48px title
        // bar shows 900×556.
        let b = Bounds::viewport(1100, 620, 48);
        let shown = fit(r(100, 70, 900, 760), b);
        assert_eq!(shown, r(100, 48, 900, 556));
        let stored = (900, Some(760));
        // Moving it keeps 900×760, so it gets its height back when the window grows.
        assert_eq!(kept(stored, moved(shown, 30, 0, b), None), Geometry { w: 900, h: Some(760), pos: Some((130, 48)) });
        // Widening it keeps the height it had; shortening it keeps the width.
        let e = kept(stored, resize(shown, Edge::E, 40, 0, (480, 360), b), Some(Edge::E));
        assert_eq!((e.w, e.h), (940, Some(760)));
        let s = kept(stored, resize(shown, Edge::S, 0, -100, (480, 360), b), Some(Edge::S));
        assert_eq!((s.w, s.h), (900, Some(456)));
        // A corner sets both from what is shown.
        let se = kept(stored, resize(shown, Edge::SE, -100, -100, (480, 360), b), Some(Edge::SE));
        assert_eq!((se.w, se.h), (800, Some(456)));
    }

    #[test]
    fn restore_applies_the_minimum_and_drops_modal_positions() {
        let settings = SurfaceSpec::new("settings", 1080, Some(760)).min(Some(720), Some(480));
        assert_eq!(restore(None, &settings), None);
        assert_eq!(restore(Some(Saved { w: 900, h: Some(600), x: Some(10), y: Some(20) }), &settings), Some(Geometry { w: 900, h: Some(600), pos: None }));
        assert_eq!(restore(Some(Saved { w: 300, h: Some(100), x: None, y: None }), &settings), Some(Geometry { w: 720, h: Some(480), pos: None }));
        assert_eq!(restore(Some(Saved { w: 99_999, h: Some(900), x: None, y: None }), &settings).unwrap().w, MAX_SIDE);
        assert_eq!(restore(Some(Saved { w: 0, h: Some(0), x: None, y: None }), &settings), None);
        let popout = SurfaceSpec::new("compose-popout", 900, Some(700)).min(Some(520), Some(400)).movable();
        assert_eq!(restore(Some(Saved { w: 700, h: Some(500), x: Some(40), y: Some(60) }), &popout), Some(Geometry { w: 700, h: Some(500), pos: Some((40, 60)) }));
        // A movable surface saved without a position opens centred.
        assert_eq!(restore(Some(Saved { w: 700, h: Some(500), x: None, y: None }), &popout).unwrap().pos, None);
    }

    #[test]
    fn saved_geometry_keeps_positions_only_for_movable_surfaces() {
        let g = Geometry { w: 800, h: Some(600), pos: Some((30, 40)) };
        assert_eq!(to_saved(g, false), Saved { w: 800, h: Some(600), x: None, y: None });
        assert_eq!(to_saved(g, true), Saved { w: 800, h: Some(600), x: Some(30), y: Some(40) });
    }

    #[test]
    fn store_entry_round_trips_and_resets() {
        let mut map = parse_saved(None);
        assert!(map.is_empty());
        map = with_saved(map, "settings", Some(Saved { w: 900, h: Some(600), x: None, y: None }));
        map = with_saved(map, "read-window", Some(Saved { w: 700, h: Some(500), x: Some(10), y: Some(20) }));
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"{"read-window":{"w":700,"h":500,"x":10,"y":20},"settings":{"w":900,"h":600}}"#);
        let back = parse_saved(Some(&json));
        assert_eq!(back, map);
        let reset = with_saved(back, "settings", None);
        assert_eq!(reset.keys().collect::<Vec<_>>(), vec!["read-window"]);
        // Unreadable entries are ignored, readable ones kept.
        let mixed = parse_saved(Some(r#"{"a":{"w":1,"h":2},"b":"junk","c":{"w":3,"h":4,"extra":true}}"#));
        assert_eq!(mixed.keys().collect::<Vec<_>>(), vec!["a", "c"]);
        assert!(parse_saved(Some("not json")).is_empty());
    }

    #[test]
    fn page_reports() {
        let r: Report = serde_json::from_str(r#"{"x":12.4,"y":30.6,"w":800.2,"h":600.5}"#).unwrap();
        assert_eq!(r.geometry(), Some(Geometry { w: 800, h: Some(601), pos: Some((12, 31)) }));
        let reset: Report = serde_json::from_str(r#"{"reset":true}"#).unwrap();
        assert_eq!(reset.geometry(), None);
        let bad: Report = serde_json::from_str(r#"{"w":0,"h":10}"#).unwrap();
        assert_eq!(bad.geometry(), None);
        let bad: Report = serde_json::from_str(r#"{"w":500,"h":0}"#).unwrap();
        assert_eq!(bad.geometry(), None);
        // Content height: the page sends h as null.
        let auto: Report = serde_json::from_str(r#"{"x":480,"y":336,"w":580,"h":null}"#).unwrap();
        assert_eq!(auto.geometry(), Some(Geometry { w: 580, h: None, pos: Some((480, 336)) }));
    }

    #[test]
    fn style_and_attributes() {
        let s = SurfaceSpec::new("add-account", 480, None);
        assert_eq!((s.min_width, s.min_height), (360, 240));
        assert_eq!(s.default_attr(), "480 auto");
        assert_eq!(surface_style(&s, None), "--ax-sw:480px;");
        let g = Geometry { w: 600, h: Some(500), pos: Some((20, 30)) };
        assert_eq!(surface_style(&s, Some(g)), "--ax-sw:600px;--ax-sh:500px;--ax-sx:20px;--ax-sy:30px;");
        let settings = SurfaceSpec::new("settings", 1080, Some(760)).min(Some(720), Some(480));
        assert_eq!(settings.default_attr(), "1080 760");
        assert_eq!(settings.min_attr(), "720 480");
        assert_eq!(surface_style(&settings, Some(Geometry { w: 900, h: Some(600), pos: None })), "--ax-sw:900px;--ax-sh:600px;");
        assert_eq!(default_min(300, Some(200)), (300, 200));
    }

    #[test]
    fn edge_codes_round_trip() {
        for e in Edge::ALL {
            assert_eq!(Edge::from_code(e.code()), Some(e));
        }
        assert_eq!(Edge::from_code("x"), None);
    }

    #[test]
    fn page_script_follows_the_reference_math() {
        let js = super::super::dom::RUNTIME_JS;
        // The runtime handles the grips and move areas this module renders,
        // reports through watchSurface, and clamps like `resize` / `moved`.
        // Gestures keep the undragged sides as stored (`kept`), and a grip
        // release after the window lost focus still clicks nothing.
        for needle in [
            "data-ax-grip",
            "data-ax-surface-move",
            "ax.watchSurface =",
            "data-ax-placed",
            "data-ax-sized",
            "--ax-sw",
            "--ax-sx",
            "reset: true",
            "const keptRect =",
            "storedSize(el, start)",
            "removeProperty(\"--ax-sh\")",
            "endGesture(true)",
            "pressHeld",
        ] {
            assert!(js.contains(needle), "dom.js lacks {needle}");
        }
        assert!(WATCH_JS.contains("watchSurface") && WATCH_JS.contains("dioxus.send"));
    }
}
