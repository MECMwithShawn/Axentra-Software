//! Circuit Halo: the approved Axentra Vault emblem and its 2.4 s startup entrance, drawn natively
//! from Astra's numeric spec (docs/circuit-halo-SPEC.md, assets/circuit-halo/geometry.json).
//!
//! Everything lives in the mark's 256 x 256 space with the circuit centre at (128, 118); coordinates,
//! strokes and blur radii scale by size/256. The lock is closed on every frame, the identity region
//! is allocated at full size from frame zero (so the form below never moves or waits), and with
//! Windows "Animation effects" off the composition is sampled at t = 2.4 s and never animates.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, Mesh, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2,
};
use serde::Deserialize;

const END: f32 = 2.4;

// Palette (geometry.json "palette").
const NAVY_BLUE: Color32 = Color32::from_rgb(0x1d, 0x54, 0x86);
const CYAN: Color32 = Color32::from_rgb(0x22, 0xd3, 0xee);
const GLOW: Color32 = Color32::from_rgb(0x5f, 0xe3, 0xff);
const LIGHT: Color32 = Color32::from_rgb(0x6f, 0xe8, 0xff);
const GLASS: Color32 = Color32::from_rgb(0xf2, 0xfd, 0xff);
const GLASS2: Color32 = Color32::from_rgb(0xa6, 0xe9, 0xff);
const INK: Color32 = Color32::from_rgb(0x0d, 0x3a, 0x66);

/// Outlined wordmark and caption are 346 x 27 and 310 x 9 at the 392 px desktop emblem size.
const DESKTOP_EMBLEM: f32 = 392.0;
const WORDMARK: Vec2 = vec2(346.0, 27.0);
const CAPTION: Vec2 = vec2(310.0, 9.0);
const GAP_WORDMARK: f32 = 14.0;
const GAP_CAPTION: f32 = 13.0;

#[derive(Deserialize)]
struct Geometry {
    center: [f32; 2],
    rings: Vec<Ring>,
    #[serde(rename = "wirePaths")]
    wire_paths: Vec<Wire>,
    #[serde(rename = "meshNodes")]
    mesh_nodes: Vec<MeshNode>,
    branches: Vec<String>,
    curves: Curves,
}

#[derive(Deserialize)]
struct Ring {
    r: f32,
    w: f32,
    a: f32,
    arcs: Vec<[f32; 2]>,
    period: f32,
    dir: f32,
    nodes: Vec<usize>,
    #[serde(rename = "nodeEnds")]
    node_ends: Vec<u8>,
    start: f32,
    end: f32,
    initial: f32,
    dash: Option<[f32; 2]>,
}

#[derive(Deserialize)]
struct Wire {
    d: String,
    opacity: f32,
    w: f32,
}

#[derive(Deserialize)]
struct MeshNode {
    p: [f32; 2],
    r: f32,
}

#[derive(Deserialize)]
struct Curves {
    reveal: [f32; 4],
    settle: [f32; 4],
    soft: [f32; 4],
}

fn geometry() -> &'static Geometry {
    static G: OnceLock<Geometry> = OnceLock::new();
    G.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/circuit-halo/geometry.json"))
            .expect("circuit-halo geometry.json")
    })
}

/// Points of an "M x,y L x,y ..." polyline.
fn polyline(d: &str) -> Vec<Pos2> {
    d.split(['M', 'L'])
        .filter_map(|p| {
            let (x, y) = p.trim().split_once(',')?;
            Some(pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect()
}

/// CSS cubic-bezier(x1, y1, x2, y2) at progress x, by bisection on the x polynomial (18 steps).
fn bezier(c: [f32; 4], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let f = |t: f32, p1: f32, p2: f32| {
        3.0 * (1.0 - t).powi(2) * t * p1 + 3.0 * (1.0 - t) * t * t * p2 + t.powi(3)
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..18 {
        let mid = (lo + hi) / 2.0;
        if f(mid, c[0], c[2]) < x {
            lo = mid
        } else {
            hi = mid
        }
    }
    f((lo + hi) / 2.0, c[1], c[3])
}

/// E(t, a, b, curve): 0 before a, 1 after b, eased between.
fn e(t: f32, a: f32, b: f32, curve: [f32; 4]) -> f32 {
    if t <= a {
        0.0
    } else if t >= b {
        1.0
    } else {
        bezier(curve, (t - a) / (b - a))
    }
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        c.r(),
        c.g(),
        c.b(),
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// Elliptical radial gradient with stops `(offset, colour)`, like an SVG radialGradient.
fn radial(painter: &egui::Painter, c: Pos2, rx: f32, ry: f32, stops: &[(f32, Color32)]) {
    let n = 96u32;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, stops[0].1);
    for &(off, col) in stops.iter() {
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(c + vec2(a.cos() * rx * off, a.sin() * ry * off), col);
        }
    }
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    for ring in 0..stops.len() as u32 - 1 {
        let (a0, b0) = (1 + ring * n, 1 + (ring + 1) * n);
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.add_triangle(a0 + i, b0 + i, b0 + j);
            mesh.add_triangle(a0 + i, b0 + j, a0 + j);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Whether Windows "Animation effects" is on (Settings > Accessibility > Visual effects).
#[cfg(windows)]
pub fn system_animations_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut enabled: i32 = 1;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    ok == 0 || enabled != 0
}

/// What the vault is doing, for the unlock animation. All times in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Activity {
    /// Time since unlocking started (key stretching / passkey running), if it has.
    pub working: Option<f32>,
    /// Progress toward the expected unlock time (0..1), when an estimate exists.
    pub progress: Option<f32>,
    /// Time since the vault opened (the lock clicking open), if it has.
    pub opening: Option<f32>,
}

/// How long the opening flourish is held before the vault view appears.
pub const OPEN_HOLD: Duration = Duration::from_millis(380);

pub struct Intro {
    start: Instant,
    animate: bool,
}

/// Maps the mark's 256-unit space into an on-screen square.
#[derive(Clone, Copy)]
struct Frame {
    origin: Pos2,
    s: f32,
}

impl Frame {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.s
    }

    fn polar(&self, deg: f32, r: f32) -> Pos2 {
        let g = geometry();
        let a = deg.to_radians();
        self.p(g.center[0] + r * a.cos(), g.center[1] + r * a.sin())
    }
}

impl Intro {
    /// `animate` false shows the end frame (t = 2.4 s, idle 0) with no further repaints.
    pub fn new(animate: bool) -> Self {
        Intro {
            start: Instant::now(),
            animate: animate && system_animations_enabled(),
        }
    }

    fn t(&self) -> f32 {
        if self.animate {
            self.start.elapsed().as_secs_f32()
        } else {
            END
        }
    }

    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.t() >= END
    }

    /// Height of the whole identity region (emblem, wordmark, caption) for an emblem size.
    pub fn region_height(emblem: f32) -> f32 {
        let k = emblem / DESKTOP_EMBLEM;
        emblem + (GAP_WORDMARK + WORDMARK.y + GAP_CAPTION + CAPTION.y) * k
    }

    /// Allocates the identity region at full size and paints the current frame.
    pub fn show(&self, ui: &mut egui::Ui, emblem: f32, activity: Activity) {
        let t = self.t();
        // Reduced motion: no spinning or sweeping; the status text carries the state.
        let activity = if self.animate {
            activity
        } else {
            Activity::default()
        };
        let (rect, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), Self::region_height(emblem)),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect.expand(8.0));
        let mark = Rect::from_min_size(
            pos2(rect.center().x - emblem / 2.0, rect.top()),
            Vec2::splat(emblem),
        );
        paint_emblem(
            &painter,
            Frame {
                origin: mark.min,
                s: emblem / 256.0,
            },
            t,
            activity,
        );

        // Outlined wordmark and caption: fade up with an 8 px rise (paint-only, layout never moves).
        let c = &geometry().curves;
        let k = emblem / DESKTOP_EMBLEM;
        let word = e(t, 1.64, 2.20, c.reveal);
        let cap = e(t, 1.76, 2.32, c.reveal);
        let word_rect = Rect::from_min_size(
            pos2(
                rect.center().x - WORDMARK.x * k / 2.0,
                mark.bottom() + GAP_WORDMARK * k + 8.0 * (1.0 - word),
            ),
            WORDMARK * k,
        );
        egui::Image::new(egui::include_image!("../assets/circuit-halo/wordmark.svg"))
            .tint(Color32::from_white_alpha((word * 255.0) as u8))
            .paint_at(ui, word_rect);
        let cap_rect = Rect::from_min_size(
            pos2(
                rect.center().x - CAPTION.x * k / 2.0,
                word_rect.bottom() - 8.0 * (1.0 - word) + GAP_CAPTION * k + 8.0 * (1.0 - cap),
            ),
            CAPTION * k,
        );
        egui::Image::new(egui::include_image!("../assets/circuit-halo/caption.svg"))
            .tint(Color32::from_white_alpha((cap * 255.0) as u8))
            .paint_at(ui, cap_rect);

        if self.animate {
            // Entrance and unlock animations at full rate; the idle rotation is slow (~30 fps).
            if t < END || activity.working.is_some() || activity.opening.is_some() {
                ui.ctx().request_repaint()
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(33))
            }
        }
    }
}

/// Paints the untiled emblem at time `t` (seconds since start), animated by `act` while the
/// vault is unlocking or opening.
fn paint_emblem(painter: &egui::Painter, f: Frame, t: f32, act: Activity) {
    let g = geometry();
    let c = &g.curves;
    let s = f.s;
    let center = f.p(g.center[0], g.center[1]);
    let u = (t - END).max(0.0);

    // Atmosphere and central bloom.
    let atmos = e(t, 0.0, 0.48, c.reveal);
    radial(
        painter,
        center,
        125.0 * s,
        125.0 * s,
        &[
            (0.0, alpha(NAVY_BLUE, 0.9 * atmos)),
            (0.54, alpha(NAVY_BLUE, 0.42 * atmos)),
            (1.0, alpha(NAVY_BLUE, 0.0)),
        ],
    );
    radial(
        painter,
        f.p(128.0, 119.0),
        64.0 * s,
        64.0 * s,
        &[
            (0.0, alpha(CYAN, 0.28 * atmos)),
            (0.45, alpha(CYAN, 0.08 * atmos)),
            (1.0, alpha(CYAN, 0.0)),
        ],
    );

    // Triangulated mesh and its junctions.
    let mesh_a = e(t, 0.12, 0.95, c.reveal);
    if mesh_a > 0.0 {
        for w in &g.wire_paths {
            let pts: Vec<Pos2> = polyline(&w.d).into_iter().map(|p| f.p(p.x, p.y)).collect();
            painter.add(Shape::line(
                pts,
                Stroke::new(w.w * s, alpha(CYAN, w.opacity * mesh_a)),
            ));
        }
        for n in &g.mesh_nodes {
            painter.circle_filled(f.p(n.p[0], n.p[1]), n.r * s, alpha(GLOW, 0.3 * mesh_a));
        }
    }

    // Circuit branches.
    let branch_a = e(t, 0.45, 1.24, c.reveal);
    if branch_a > 0.0 {
        for b in &g.branches {
            let pts: Vec<Pos2> = polyline(b).into_iter().map(|p| f.p(p.x, p.y)).collect();
            painter.add(Shape::line(
                pts,
                Stroke::new(0.4 * s, alpha(LIGHT, 0.55 * branch_a)),
            ));
        }
    }

    // Ring tracks with their nodes.
    for (i, ring) in g.rings.iter().enumerate() {
        let reveal = e(t, ring.start, ring.end, c.reveal);
        if reveal <= 0.0 {
            continue;
        }
        // While unlocking, every ring spins up in its own direction (outer rings a little faster).
        let spin = act
            .working
            .map(|w| ring.dir * 70.0 * w * (1.0 + 0.04 * i as f32))
            .unwrap_or(0.0);
        let rot = ring.initial * (1.0 - e(t, ring.start, 1.5, c.reveal))
            + ring.dir * 360.0 * u / ring.period
            + spin;
        let colour = alpha(if ring.w > 0.6 { LIGHT } else { GLOW }, ring.a);
        for &[a0, a1] in &ring.arcs {
            let a1 = a0 + (a1 - a0) * reveal;
            let span = a1 - a0;
            if span.abs() < 0.01 {
                continue;
            }
            match ring.dash {
                // Round-capped zero-length dashes: dots of diameter w every 2.56 units of arc.
                Some([dash, gap]) => {
                    let pitch = dash + gap;
                    let len = span.to_radians().abs() * ring.r;
                    let count = (len / pitch).floor() as usize;
                    for k in 0..=count {
                        let deg =
                            a0 + rot + (k as f32 * pitch / ring.r).to_degrees() * span.signum();
                        painter.circle_filled(
                            f.polar(deg, ring.r),
                            (ring.w * s / 2.0).max(0.35),
                            colour,
                        );
                    }
                }
                None => {
                    let steps = (span.abs() / 1.5).ceil().max(2.0) as usize;
                    let pts: Vec<Pos2> = (0..=steps)
                        .map(|k| f.polar(a0 + rot + span * k as f32 / steps as f32, ring.r))
                        .collect();
                    let (first, last) = (pts[0], *pts.last().unwrap());
                    painter.add(Shape::line(pts, Stroke::new(ring.w * s, colour)));
                    // Round caps.
                    painter.circle_filled(first, ring.w * s / 2.0, colour);
                    painter.circle_filled(last, ring.w * s / 2.0, colour);
                }
            }
        }

        // Nodes light once their arc is complete, then shimmer gently while idle.
        let node_a = e(t, ring.end, ring.end + 0.12, c.soft);
        if node_a > 0.0 {
            let radius = if ring.r > 90.0 { 1.20 } else { 1.02 };
            for (k, (&arc, &end)) in ring.nodes.iter().zip(&ring.node_ends).enumerate() {
                let deg = ring.arcs[arc][end as usize] + rot;
                let shimmer = 1.0
                    + e(t, END, 3.2, c.soft)
                        * (-0.05
                            + 0.05
                                * (std::f32::consts::TAU * u / (5.0 + 0.2 * i as f32)
                                    + k as f32
                                    + 0.7 * i as f32)
                                    .cos());
                // Unlocking: nodes pulse in sequence around the halo.
                let pulse = act
                    .working
                    .map(|w| {
                        0.45 + 0.55
                            * (0.5 + 0.5 * (w * 6.0 - (i as f32 * 1.3 + k as f32 * 0.9)).sin())
                    })
                    .unwrap_or(1.0);
                let a = node_a * shimmer * pulse;
                let col = if k % 2 == 0 { GLASS } else { LIGHT };
                let p = f.polar(deg, ring.r);
                // Gaussian sigma 1.4 glow merged with the dot, approximated by soft halos.
                painter.circle_filled(p, (radius + 2.8) * s, alpha(col, 0.10 * a));
                painter.circle_filled(p, (radius + 1.4) * s, alpha(col, 0.28 * a));
                painter.circle_filled(p, radius * s, alpha(col, a));
            }
        }
    }

    // Unlocking sweep: a bright arc racing round the halo, or a progress ring toward the
    // expected unlock time when one is known.
    if let Some(w) = act.working {
        let r = 113.0;
        let fade_in =
            (w / 0.25).min(1.0) * act.opening.map(|o| 1.0 - (o / 0.3).min(1.0)).unwrap_or(1.0);
        let arc = |from: f32, to: f32, width: f32, a: f32| {
            let steps = ((to - from).abs() / 2.0).ceil().max(2.0) as usize;
            let pts: Vec<Pos2> = (0..=steps)
                .map(|k| f.polar(from + (to - from) * k as f32 / steps as f32, r))
                .collect();
            painter.add(Shape::line(
                pts,
                Stroke::new(width * s, alpha(CYAN, a * fade_in)),
            ));
        };
        let (start, end) = match act.progress {
            Some(p) => {
                arc(0.0, 360.0, 1.2, 0.12);
                (-90.0, -90.0 + 360.0 * p.clamp(0.0, 1.0))
            }
            None => {
                let head = -90.0 + w * 320.0;
                (head - 80.0, head)
            }
        };
        // Glow, body and a brighter leading edge.
        arc(start, end, 6.0, 0.10);
        arc(start, end, 3.0, 0.28);
        arc(start, end, 1.4, 0.95);
        let tip = f.polar(end, r);
        painter.circle_filled(tip, 4.5 * s, alpha(GLASS, 0.25 * fade_in));
        painter.circle_filled(tip, 2.0 * s, alpha(GLASS, fade_in));
    }

    // Closed lock: fade in, shackle seats, a tiny compression "click", then settle. Once the
    // vault has opened (and only then) the shackle lifts free.
    let lock_a = e(t, 0.92, 1.30, c.reveal);
    if lock_a > 0.0 {
        let open = act
            .opening
            .map(|o| e(o, 0.0, 0.28, c.settle))
            .unwrap_or(0.0);
        let seat = -0.8 * (1.0 - e(t, 1.18, 1.42, c.settle)) - 7.5 * open;
        let scale = 1.0 - 0.016 * e(t, 1.42, 1.49, c.settle) + 0.016 * e(t, 1.49, 1.62, c.settle);
        let lp = |x: f32, y: f32| f.p(128.0 + (x - 128.0) * scale, 120.0 + (y - 120.0) * scale);
        let ls = s * scale;

        // Lock glow: body duplicate in cyan, alpha .30, Gaussian sigma 3.8.
        let body = Rect::from_min_max(lp(112.0, 116.0), lp(144.0, 142.0));
        let glow = egui::epaint::Shadow {
            offset: [0, 0],
            blur: (3.8 * 2.5 * ls).round().clamp(1.0, 255.0) as u8,
            spread: 0,
            color: alpha(CYAN, (0.30 + 0.35 * open) * lock_a),
        };
        painter.add(glow.as_shape(body, CornerRadius::same((3.2 * ls).round() as u8)));

        // Shackle: M119 118 V108 C119 94 137 94 137 108 V118, stroke 3.5, round caps.
        let mut pts = vec![lp(119.0, 118.0 + seat)];
        for k in 0..=24 {
            let tt = k as f32 / 24.0;
            let b = |p0: f32, p1: f32, p2: f32, p3: f32| {
                (1.0 - tt).powi(3) * p0
                    + 3.0 * (1.0 - tt).powi(2) * tt * p1
                    + 3.0 * (1.0 - tt) * tt * tt * p2
                    + tt.powi(3) * p3
            };
            pts.push(lp(
                b(119.0, 119.0, 137.0, 137.0),
                b(108.0, 94.0, 94.0, 108.0) + seat,
            ));
        }
        pts.push(lp(137.0, 118.0 + seat));
        let shackle_col = alpha(lerp_color(GLASS, GLASS2, 0.25), lock_a);
        let (first, last) = (pts[0], *pts.last().unwrap());
        painter.add(Shape::line(pts, Stroke::new(3.5 * ls, shackle_col)));
        painter.circle_filled(first, 1.75 * ls, shackle_col);
        painter.circle_filled(last, 1.75 * ls, shackle_col);

        // Body: rounded rect with a vertical glass gradient.
        gradient_rounded_rect(
            painter,
            body,
            3.2 * ls,
            alpha(GLASS, lock_a),
            alpha(GLASS2, lock_a),
        );

        // Keyhole.
        painter.circle_filled(lp(128.0, 124.4), 3.9 * ls, alpha(INK, lock_a));
        painter.rect_filled(
            Rect::from_min_max(lp(126.4, 126.0), lp(129.6, 136.6)),
            CornerRadius::same((1.6 * ls).round() as u8),
            alpha(INK, lock_a),
        );
    }

    // Opening flash: a bright ring bursting out from the lock.
    if let Some(o) = act.opening {
        let p = (o / 0.5).min(1.0);
        let r = 30.0 + 90.0 * e(o, 0.0, 0.5, c.reveal);
        painter.circle_stroke(
            center,
            r * s,
            Stroke::new(1.6 * s, alpha(GLASS, 0.45 * (1.0 - p))),
        );
        painter.circle_stroke(
            center,
            r * s,
            Stroke::new(6.0 * s, alpha(CYAN, 0.12 * (1.0 - p))),
        );
    }

    // Seal pulse.
    if (1.46..=2.12).contains(&t) {
        let r = 24.0 + (108.0 - 24.0) * e(t, 1.46, 2.12, c.reveal);
        let a = 0.18 * e(t, 1.46, 1.57, c.soft) * (1.0 - e(t, 1.57, 2.12, c.soft));
        painter.circle_stroke(center, r * s, Stroke::new(0.9 * s, alpha(GLOW, a)));
    }

    // Horizon bloom and core line.
    let horizon = 0.2 * e(t, 0.2, 0.8, c.reveal) + 0.8 * e(t, 1.45, 2.1, c.reveal);
    if horizon > 0.0 {
        radial(
            painter,
            f.p(128.0, 240.0),
            52.0 * s,
            4.472 * s,
            &[
                (0.0, alpha(GLASS, 0.94 * horizon)),
                (0.12, alpha(LIGHT, 0.65 * horizon)),
                (0.45, alpha(GLOW, 0.18 * horizon)),
                (1.0, alpha(CYAN, 0.0)),
            ],
        );
        let core = alpha(LIGHT, 0.65 * horizon);
        painter.line_segment(
            [f.p(117.0, 240.0), f.p(139.0, 240.0)],
            Stroke::new(1.4 * 0.6 * s * 2.0, alpha(LIGHT, 0.12 * horizon)),
        );
        painter.line_segment(
            [f.p(117.0, 240.0), f.p(139.0, 240.0)],
            Stroke::new(0.6 * s, core),
        );
    }
}

/// Rounded rectangle filled with a vertical gradient (fan-triangulated outline).
fn gradient_rounded_rect(
    painter: &egui::Painter,
    rect: Rect,
    r: f32,
    top: Color32,
    bottom: Color32,
) {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (rect.right() - r, rect.top() + r, -90.0f32),
        (rect.right() - r, rect.bottom() - r, 0.0),
        (rect.left() + r, rect.bottom() - r, 90.0),
        (rect.left() + r, rect.top() + r, 180.0),
    ];
    let colour_at = |y: f32| {
        lerp_color(
            top,
            bottom,
            ((y - rect.top()) / rect.height()).clamp(0.0, 1.0),
        )
    };
    let mut mesh = Mesh::default();
    let c = rect.center();
    mesh.colored_vertex(c, colour_at(c.y));
    for (cx, cy, a0) in corners {
        for k in 0..=6 {
            let a = (a0 + 90.0 * k as f32 / 6.0).to_radians();
            let p = pos2(cx + r * a.cos(), cy + r * a.sin());
            mesh.colored_vertex(p, colour_at(p.y));
        }
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 1..=n {
        mesh.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_matches_spec() {
        let g = geometry();
        assert_eq!(g.rings.len(), 14);
        assert_eq!(g.wire_paths.len(), 134);
        assert_eq!(g.branches.len(), 8);
        assert_eq!(g.center, [128.0, 118.0]);
        let nodes: usize = g.rings.iter().map(|r| r.nodes.len()).sum();
        assert_eq!(nodes, 13, "13 nodes ride the rings");
        let dotted: Vec<usize> = g
            .rings
            .iter()
            .enumerate()
            .filter(|(_, r)| r.dash.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(dotted, vec![1, 5, 9, 12]);
        for (i, r) in g.rings.iter().enumerate() {
            assert!(
                (r.start - (0.16 + 0.026 * i as f32)).abs() < 1e-3,
                "ring {i} start"
            );
            assert!(
                (r.end - (0.88 + 0.043 * i as f32)).abs() < 1e-3,
                "ring {i} end"
            );
        }
        assert!(g.wire_paths.iter().all(|w| polyline(&w.d).len() == 2));
        assert!(g.branches.iter().all(|b| polyline(b).len() >= 2));
    }

    #[test]
    fn easing_matches_css() {
        let reveal = geometry().curves.reveal;
        assert!(bezier(reveal, 0.0).abs() < 1e-4);
        assert!((bezier(reveal, 1.0) - 1.0).abs() < 1e-4);
        // ease-out curve: well past halfway at x = .5.
        assert!(bezier(reveal, 0.5) > 0.85);
        // CSS "ease" (.25,.1,.25,1) at .5 is about .8024.
        assert!((bezier([0.25, 0.1, 0.25, 1.0], 0.5) - 0.8024).abs() < 2e-3);
    }

    #[test]
    fn timeline_rules() {
        let c = &geometry().curves;
        // Lock is never shown open: seating offset never lifts the shackle clear of the body.
        for k in 0..=240 {
            let t = k as f32 / 100.0;
            let seat = -0.8 * (1.0 - e(t, 1.18, 1.42, c.settle));
            assert!(
                118.0 + seat > 116.0,
                "shackle end overlaps the body at t={t}"
            );
        }
        // Pulse alpha peaks at .18 and is zero outside its window.
        let pulse = |t: f32| {
            if (1.46..=2.12).contains(&t) {
                0.18 * e(t, 1.46, 1.57, c.soft) * (1.0 - e(t, 1.57, 2.12, c.soft))
            } else {
                0.0
            }
        };
        assert!((pulse(1.57) - 0.18).abs() < 1e-4);
        assert_eq!(pulse(1.0), 0.0);
        assert_eq!(pulse(2.3), 0.0);
    }

    #[test]
    fn reduced_motion_is_end_frame() {
        assert!(Intro::new(false).finished());
    }

    #[test]
    fn identity_region_is_455_at_desktop_size() {
        assert!((Intro::region_height(392.0) - 455.0).abs() < 0.5);
    }
}

/// Read through `gsettings`, so cached for a few seconds: it is asked every frame while unlocking.
#[cfg(target_os="linux")]
pub fn system_animations_enabled() -> bool {
    use std::{sync::Mutex, time::{Duration, Instant}};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match *cache {
        Some((at, on)) if at.elapsed() < Duration::from_secs(5) => on,
        _ => {
            let on = crate::linux::desktop_setting("org.gnome.desktop.interface", "enable-animations").trim() != "false";
            *cache = Some((Instant::now(), on));
            on
        }
    }
}
