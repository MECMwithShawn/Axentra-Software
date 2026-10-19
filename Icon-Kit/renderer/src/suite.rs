// Axentra Glass + Precision. The same std-only renderer serves the exporter,
// Command, Ghost, and Windows resource build scripts.
use std::f32::consts::PI;
type C3 = [f32; 3];
const NAVY: C3 = [0.035, 0.055, 0.105];
#[allow(dead_code)]
pub const APPS: &[&str] = &[
    "command",
    "ghost",
    "vault",
    "crystalwire",
    "horizon",
    "pdf",
    "spaces",
    "mail",
    "outlook",
];
fn mix(a: C3, b: C3, t: f32) -> C3 {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}
fn rbox(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hw - r);
    let qy = (y - cy).abs() - (hh - r);
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}
fn seg(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    len(px - dx * t, py - dy * t)
}
fn poly(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    for i in 0..pts.len() {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % pts.len()];
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}
fn accent(app: &str) -> C3 {
    let c = match app {
        "vault" => [64, 139, 255],
        "crystalwire" => [31, 208, 234],
        "horizon" => [255, 180, 70],
        "ghost" => [170, 126, 255],
        "pdf" => [255, 105, 119],
        "spaces" => [26, 211, 192],
        "mail" => [239, 136, 199],
        "outlook" => [124, 140, 255], // #7C8CFF periwinkle, between Vault blue and Ghost violet
        _ => [98, 180, 255],
    };
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
    ]
}
fn node(i: usize) -> (f32, f32) {
    // Six nodes around a vertical hexagon, clockwise from the top.
    const NODES: [(f32, f32); 6] = [
        (0.0, -0.29),
        (0.25115, -0.145),
        (0.25115, 0.145),
        (0.0, 0.29),
        (-0.25115, 0.145),
        (-0.25115, -0.145),
    ];
    NODES[i]
}
fn hex(x: f32, y: f32, r: f32) -> bool {
    x.abs() <= r * 0.8660254 && y.abs() + x.abs() * 0.5773503 <= r
}
fn window(x: f32, y: f32, cx: f32, cy: f32, micro: bool) -> bool {
    let outer = rbox(x, y, cx, cy, 0.205, 0.16, 0.032) <= 0.0;
    let inner = rbox(x, y, cx, cy + 0.032, 0.174, 0.097, 0.008) < 0.0;
    let dots = !micro
        && [-0.14, -0.085]
            .iter()
            .any(|dx| len(x - cx - dx, y - cy + 0.112) < 0.016);
    outer && !inner && !dots
}
// Vault's circuit halo is a product-specific symbol inside the unchanged suite tile.
// Distances keep the luminous strokes narrow; micro icons lose the inner ring/dots.
fn vault_lock(x: f32, y: f32, micro: bool) -> (f32, bool) {
    let scale = if micro { 1.16 } else { 1.0 };
    let (x, y) = (x / scale, y / scale);
    let shackle = if y <= -0.075 {
        (len(x, y + 0.075) - 0.083).abs() - 0.020
    } else {
        seg(x.abs(), y, 0.083, -0.075, 0.083, -0.020) - 0.020
    };
    let body = rbox(x, y, 0.0, 0.069, 0.145, 0.108, 0.032);
    let eye = if micro { 0.032 } else { 0.027 };
    let hole = len(x, y - 0.034) <= eye
        || rbox(x, y, 0.0, 0.073, 0.013, 0.033, 0.012) <= 0.0;
    (shackle.min(body) * scale, hole)
}
fn vault_arc(x: f32, y: f32, angle: f32, radius: f32, start: f32, end: f32) -> f32 {
    if angle >= start && angle <= end {
        (len(x, y) - radius).abs()
    } else {
        [start, end].iter().map(|degrees| {
            let a = degrees * PI / 180.0;
            len(x - radius * a.cos(), y - radius * a.sin())
        }).fold(f32::INFINITY, f32::min)
    }
}
fn vault_finish(mut c: C3, micro: bool, x: f32, y: f32) -> C3 {
    let angle = y.atan2(x) * 180.0 / PI;
    let mut track = f32::INFINITY;
    // Reference-like broken outer halo, with open gaps at upper-left and bottom.
    for (start, end) in [(-110.0, 86.0), (104.0, 137.0), (151.0, 223.0)] {
        let a = if angle < -110.0 { angle + 360.0 } else { angle };
        track = track.min(vault_arc(x, y, a, 0.355, start, end));
    }
    if !micro {
        for (start, end) in [(-133.0, -27.0), (-10.0, 138.0), (160.0, 206.0)] {
            let a = if angle < -133.0 { angle + 360.0 } else { angle };
            track = track.min(vault_arc(x, y, a, 0.273, start, end) + 0.0015);
        }
    }
    let stroke = if micro { 0.009 } else { 0.0055 };
    let cyan = [0.24, 0.79, 1.0];
    let edge = (track - stroke).max(0.0);
    c = mix(c, cyan, (-(edge / 0.018).powi(2)).exp() * if micro { 0.08 } else { 0.23 });
    if track <= stroke {
        c = mix(cyan, [0.60, 0.94, 1.0], 0.30 + (-y).max(0.0) * 0.65);
    }
    if !micro {
        // A fine dotted middle ring; subordinate to both the lock and solid arcs.
        let step = 2.0 * PI / 48.0;
        let a = (y.atan2(x) / step).round() * step;
        let dot = len(x - 0.315 * a.cos(), y - 0.315 * a.sin());
        if dot < 0.0028 {
            c = mix(c, [0.43, 0.84, 1.0], 0.60);
        }
    }
    let mut node_distance = f32::INFINITY;
    for (radius, degrees) in [(0.355, 86.0), (0.355, 137.0), (0.355, 223.0), (0.273, 138.0)] {
        let a = degrees * PI / 180.0;
        node_distance = node_distance.min(len(x - radius * a.cos(), y - radius * a.sin()));
    }
    let node_radius = if micro { 0.012 } else { 0.013 };
    let node_edge = (node_distance - node_radius).max(0.0);
    c = mix(c, [0.59, 0.90, 1.0], (-(node_edge / 0.018).powi(2)).exp() * 0.30);
    if node_distance <= node_radius {
        c = [0.83, 0.97, 1.0];
    }
    let (lock, hole) = vault_lock(x, y, micro);
    let glow = (-(lock.max(0.0) / 0.021).powi(2)).exp();
    c = mix(c, [0.22, 0.72, 1.0], glow * 0.22);
    if lock <= 0.0 {
        c = if hole {
            [0.035, 0.16, 0.29]
        } else {
            let glass = mix([0.73, 0.96, 1.0], [0.32, 0.68, 0.98], (y + 0.17) / 0.36);
            mix(glass, [0.80, 0.98, 1.0], (-(lock / 0.004).powi(2)).exp() * 0.48)
        };
    }
    c
}
fn glyph(app: &str, micro: bool, x: f32, y: f32) -> bool {
    match app {
        "command" => {
            let core = hex(x, y, 0.16) && (micro || !hex(x, y, 0.072));
            core || (0..6).any(|i| {
                let (nx, ny) = node(i);
                rbox(x, y, nx, ny, 0.068, 0.068, 0.023) <= 0.0
                    || seg(x, y, nx * 0.49, ny * 0.49, nx, ny) <= if micro { 0.018 } else { 0.012 }
            })
        }
        "ghost" => {
            // Ghost retains its familiar head, eyes and scalloped hem.
            let body = if y <= -0.05 {
                len(x, y + 0.05) <= 0.25
            } else {
                x.abs() <= 0.25 && y <= 0.22 + 0.07 * (((x + 0.25) / 0.5) * 3.0 * PI).sin().abs()
            };
            let eyes = [-0.085, 0.085]
                .iter()
                .any(|ex| ((x - ex) / 0.045).powi(2) + ((y + 0.06) / 0.065).powi(2) <= 1.0);
            body && !eyes
        }
        "vault" => {
            let (lock, hole) = vault_lock(x, y, micro);
            lock <= 0.0 && !hole
        }
        "crystalwire" => {
            let body = poly(
                x,
                y,
                &[
                    (-0.18, -0.26),
                    (0.18, -0.26),
                    (0.34, -0.06),
                    (0.0, 0.31),
                    (-0.34, -0.06),
                ],
            );
            let w = if micro { 0.014 } else { 0.010 };
            let cuts = (y + 0.06).abs() <= w
                || seg(x, y, 0.0, -0.26, -0.13, -0.06) <= w
                || seg(x, y, 0.0, -0.26, 0.13, -0.06) <= w
                || (!micro
                    && (seg(x, y, -0.13, -0.06, 0.0, 0.31) <= w
                        || seg(x, y, 0.13, -0.06, 0.0, 0.31) <= w));
            body && !cuts
        }
        "horizon" => {
            let sun = y <= 0.06 && len(x, y - 0.06) <= 0.2;
            let rays = (0..5).any(|i| {
                let a = PI + i as f32 * PI / 4.0;
                seg(
                    x,
                    y,
                    0.265 * a.cos(),
                    0.06 + 0.265 * a.sin(),
                    0.345 * a.cos(),
                    0.06 + 0.345 * a.sin(),
                ) <= 0.019
            });
            sun || rays
                || rbox(x, y, 0.0, 0.135, 0.33, 0.021, 0.021) <= 0.0
                || rbox(x, y, 0.0, 0.225, 0.30, 0.021, 0.021) <= 0.0
        }
        "pdf" => {
            let body = rbox(x, y, 0.0, 0.0, 0.235, 0.31, 0.038) <= 0.0 && y >= x - 0.395;
            let fold =
                ((x - 0.075).abs() < 0.014 && y < -0.15) || ((y + 0.15).abs() < 0.014 && x > 0.075);
            let mark = poly(
                x,
                y,
                &[
                    (0.0, -0.125),
                    (0.16, 0.195),
                    (0.085, 0.195),
                    (0.0, 0.025),
                    (-0.085, 0.195),
                    (-0.16, 0.195),
                ],
            );
            body && !fold && !mark
        }
        "spaces" => {
            let mask = rbox(x, y, -0.085, 0.10, 0.22, 0.175, 0.04) <= 0.0;
            window(x, y, -0.085, 0.10, micro) || (!mask && window(x, y, 0.085, -0.10, micro))
        }
        "mail" => {
            // A solid, optically centered envelope with a crisp recessed flap.
            // The micro tier widens the flap and omits the lower fold seams.
            let body = rbox(x, y, 0.0, 0.0, 0.315, 0.215, 0.045) <= 0.0;
            let cut = if micro { 0.024 } else { 0.014 };
            let flap = seg(x.abs(), y, 0.300, -0.178, 0.0, 0.040) <= cut;
            let fold = !micro
                && seg(x.abs(), y, 0.306, 0.193, 0.125, -0.012) <= 0.009;
            body && !flap && !fold
        }
        "outlook" => {
            // A calendar page behind an envelope: mail plus calendar in one mark.
            // The envelope keeps a clear gap from the page; micro drops the date grid
            // and widens every cut.
            let cut = if micro { 0.024 } else { 0.014 };
            let (ex, ey) = (-0.075, 0.140);
            let envelope = rbox(x, y, ex, ey, 0.235, 0.160, 0.035);
            let flap = seg((x - ex).abs(), y - ey, 0.222, -0.128, 0.0, 0.040) <= cut;
            let in_envelope = envelope <= 0.0 && !flap;
            let (cx, cy) = (0.080, -0.045);
            let page = rbox(x, y, cx, cy, 0.230, 0.205, 0.040) <= 0.0;
            let rings = [-0.105, 0.105].iter().any(|dx| {
                rbox(x, y, cx + dx, cy - 0.218, 0.024, 0.050, 0.022) <= 0.0
            });
            let header = (y - (cy - 0.105)).abs() <= cut * 0.5 + 0.004 && (x - cx).abs() <= 0.205;
            // Date squares: one row under the header and a short column down the
            // right edge, all clear of the envelope's gap.
            let cells = [(-0.080, -0.055), (0.005, -0.055), (0.090, -0.055), (0.175, -0.055), (0.175, 0.025), (0.175, 0.105)];
            let grid = !micro
                && cells.iter().any(|&(dx, dy)| rbox(x, y, cx + dx, cy + dy, 0.024, 0.022, 0.006) <= 0.0);
            let gap = envelope <= if micro { 0.040 } else { 0.032 };
            let in_page = (page && !header && !grid || rings) && !gap;
            in_envelope || in_page
        }
        _ => false,
    }
}
fn sample(app: &str, micro: bool, u: f32, v: f32) -> [f32; 4] {
    let (x, y) = (u - 0.5, v - 0.5);
    let tile = rbox(x, y, 0.0, 0.0, 0.488, 0.488, 0.145);
    if tile > 0.0 {
        return [0.0; 4];
    }
    let a = accent(app);
    let top = mix(
        a,
        [0.72, 0.95, 1.0],
        if app == "horizon" { 0.15 } else { 0.25 },
    );
    let mut c = mix([0.055, 0.08, 0.15], NAVY, v);
    let halo = (-((len(x, y + 0.015) - 0.23) / 0.14).powi(2)).exp();
    c = mix(c, a, halo * if micro { 0.14 } else { 0.28 });
    let glint = (-((tile + 0.032) / 0.008).powi(2)
        - ((x + 0.30) / 0.20).powi(2)
        - ((y + 0.30) / 0.20).powi(2))
    .exp();
    c = mix(c, [0.70, 0.85, 1.0], glint * 0.70);
    if tile >= -0.018 {
        c = mix(top, a, v);
        if app == "command" {
            c = mix(c, [0.65, 0.50, 1.0], ((x - y + 0.2) * 0.5).clamp(0.0, 0.55));
        }
    } else if (-0.040..=-0.035).contains(&tile) && !micro {
        c = mix(c, top, 0.22);
    }
    if app == "vault" {
        let c = vault_finish(c, micro, x, y);
        return [c[0], c[1], c[2], 1.0];
    }
    let body = glyph(app, micro, x, y);
    if !body && !micro && len(x, y) > 0.12 {
        // A narrow colored rim and a broader low-opacity halo around the silhouette.
        for (spread, alpha) in [(0.006, 0.32)] {
            if [(spread, 0.0), (-spread, 0.0), (0.0, spread), (0.0, -spread)]
                .iter()
                .any(|(dx, dy)| glyph(app, micro, x + dx, y + dy))
            {
                c = mix(c, top, alpha);
            }
        }
    }
    if body {
        c = mix(mix(a, [1.0; 3], 0.36), a, (y + 0.34) / 0.62);
        if app == "ghost" {
            c = mix([0.68, 0.45, 1.0], [0.08, 0.87, 0.95], (y + 0.30) / 0.60);
        }
        if app == "command" {
            c = mix([0.63, 0.94, 1.0], [0.17, 0.67, 1.0], (y + 0.25) / 0.5);
            for (i, id) in ["vault", "crystalwire", "horizon", "spaces", "pdf", "ghost"]
                .iter()
                .enumerate()
            {
                let (nx, ny) = node(i);
                if rbox(x, y, nx, ny, 0.068, 0.068, 0.023) <= 0.0 {
                    c = mix(
                        mix(accent(id), [1.0; 3], 0.32),
                        accent(id),
                        (y - ny + 0.068) / 0.136,
                    );
                }
            }
        }
        if app == "crystalwire" && x < 0.0 {
            c = mix(c, [0.65, 1.0, 1.0], 0.12);
        }
        let shine = (-((x + 0.08) / 0.22).powi(2) - ((y + 0.19) / 0.09).powi(2)).exp();
        c = mix(c, [0.82, 0.98, 1.0], shine * 0.24);
        if !micro && !glyph(app, micro, x, y - 0.004) {
            c = mix(c, [0.88, 1.0, 1.0], 0.45);
        }
    }
    [c[0], c[1], c[2], 1.0]
}
pub fn rgba(app: &str, n: usize) -> Vec<u8> {
    let micro = n < 32;
    let mut out = vec![0u8; n * n * 4];
    let ss = if n <= 48 { 8 } else { 4 };
    for py in 0..n {
        for px in 0..n {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let u = (px as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32;
                    let v = (py as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32;
                    let c = sample(app, micro, u, v);
                    for k in 0..3 {
                        acc[k] += c[k] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let i = (py * n + px) * 4;
            if acc[3] > 0.0 {
                for k in 0..3 {
                    out[i + k] = (acc[k] / acc[3] * 255.0).round() as u8;
                }
                out[i + 3] = (acc[3] / (ss * ss) as f32 * 255.0).round() as u8;
            }
        }
    }
    out
}
