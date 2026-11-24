// Copied from Axentra Command (tools/suite-icons/src/suite.rs), the suite icon source of truth.
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
    "excel",
    "mail", "zip", "usb", "minitube", "onedrive", "outlook",
    "notepad",
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
        "excel" => [105, 199, 160],
        "mail" => [206, 143, 187],
        "zip" => [172, 195, 124],
        "usb" => [128, 193, 166],
        "minitube" => [143, 204, 164],
        "onedrive" => [184, 222, 255],
        "onedrive-periwinkle" => [139, 151, 255],
        "onedrive-azure" => [72, 170, 255],
        "outlook" => [149, 157, 228],
        "notepad" => [200, 212, 232],
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
        "excel" => {
            // Folded sheet and solid green table header, with the suite's fine outlines.
            // Micro sizes simplify the grid so the fold and header remain legible.
            let w = if micro { 0.018 } else { 0.010 };
            let paper = rbox(x, y, 0.0, 0.0, 0.245, 0.305, 0.028).abs() <= w
                && y >= x - 0.385;
            let fold = [(0.075, -0.305, 0.245, -0.135),
                        (0.075, -0.305, 0.075, -0.135),
                        (0.075, -0.135, 0.245, -0.135)]
                .iter().any(|&(ax, ay, bx, by)| seg(x, y, ax, ay, bx, by) <= w);
            let grid = rbox(x, y, 0.0, 0.095, 0.180, 0.135, 0.009).abs() <= w;
            let header = x.abs() <= 0.180 && (-0.040..=0.030).contains(&y);
            let rows: &[f32] = if micro { &[0.130] } else { &[0.100, 0.165] };
            let cols: &[f32] = if micro { &[0.0] } else { &[-0.060, 0.060] };
            paper || fold || grid || header
                || rows.iter().any(|ry| seg(x, y, -0.180, *ry, 0.180, *ry) <= w)
                || cols.iter().any(|cx| seg(x, y, *cx, 0.030, *cx, 0.230) <= w)
        }

        "spaces" => {
            let mask = rbox(x, y, -0.085, 0.10, 0.22, 0.175, 0.04) <= 0.0;
            window(x, y, -0.085, 0.10, micro) || (!mask && window(x, y, 0.085, -0.10, micro))
        }
        _ => false,
    }
}
// Excel follows the current outlined Horizon/CrystalWire artwork. Keep this
// product finish separate so regenerating the suite does not change other apps.
fn excel_sample(micro: bool, u: f32, v: f32) -> [f32; 4] {
    let (x, y) = (u - 0.5, v - 0.5);
    let d = rbox(x, y, 0.0, 0.0, 0.453125, 0.453125, 0.15625);
    let rim_width = if micro { 0.015 } else { 0.01094 };
    let t = ((u + v) * 0.5).clamp(0.0, 1.0);
    let stops: &[(f32, C3)] = &[
        (0.0, [0.855, 0.941, 1.0]), (0.16, [0.66, 0.79, 0.74]),
        (0.42, [0.20, 0.30, 0.29]), (0.70, [0.33, 0.55, 0.44]),
        (1.0, [0.73, 0.90, 0.79]),
    ];
    let mut rim = stops[stops.len()-1].1;
    for pair in stops.windows(2) {
        if t >= pair[0].0 && t <= pair[1].0 {
            rim = mix(pair[0].1, pair[1].1, (t-pair[0].0)/(pair[1].0-pair[0].0));
            break;
        }
    }
    if d > rim_width {
        let alpha = (-((d-rim_width)/0.018).powi(2)).exp() * 0.24;
        return [rim[0], rim[1], rim[2], alpha];
    }
    let mut c = if t < 0.5 {
        mix([0.106,0.110,0.188], [0.063,0.071,0.118], t*2.0)
    } else {
        mix([0.063,0.071,0.118], [0.031,0.043,0.071], (t-0.5)*2.0)
    };
    let glow = (1.0-len(x,y+0.025)/0.41).max(0.0) * 0.14;
    c = mix(c, accent("excel"), glow);
    if d >= -rim_width { c = rim; }
    let inner = rbox(x,y,0.0,0.0,0.421875,0.421875,0.1328);
    if !micro && inner.abs() < 0.006 { c = mix(c,[0.78,0.91,0.84],0.07); }
    if poly(x, y, &[(0.075,-0.305), (0.245,-0.135), (0.075,-0.135)]) {
        c = mix(c, accent("excel"), 0.32);
    }
    if glyph("excel",micro,x,y) {
        c = mix([0.82,0.97,0.88], [0.34,0.67,0.50], (y+0.28)/0.60);
        if x.abs() < 0.170 && (-0.030..=0.030).contains(&y) {
            c = mix([0.38,0.80,0.57], [0.20,0.57,0.38], (y+0.030)/0.060);
        }
    }
    [c[0],c[1],c[2],1.0]
}
// Notepad takes Excel's product finish (the Horizon/CrystalWire glass rim) in silver: a
// spiral-bound pad in fine outlines with one solid element, the binding strip, which the
// rings rise out of. Micro sizes keep two rings and two lines. The rim and glyph tones are
// derived from the accent with the same mix Excel's hand-tuned greens follow.
fn notepad_glyph(micro: bool, x: f32, y: f32) -> u8 {
    // 0 background, 1 outline, 2 solid binding, 3 hole in the binding
    let w = if micro { 0.018 } else { 0.010 };
    let line = |ax: f32, ay: f32, bx: f32, by: f32| seg(x, y, ax, ay, bx, by) <= w;
    let page = rbox(x, y, 0.0, 0.035, 0.235, 0.285, 0.028);
    let rings: &[f32] = if micro { &[-0.1, 0.1] } else { &[-0.12, 0.0, 0.12] };
    let band = if micro { -0.135 } else { -0.155 };
    let hole = |cx: f32| rbox(x, y, cx, band - 0.045, if micro { 0.024 } else { 0.017 }, 0.02, 0.012) <= 0.0;
    if page <= 0.0 && y <= band {
        return if rings.iter().any(|cx| hole(*cx)) { 3 } else { 2 };
    }
    let rw = if micro { 0.03 } else { 0.024 };
    if y < band - 0.045 && rings.iter().any(|cx| rbox(x, y, *cx, -0.255, rw, 0.06, rw).abs() <= w) {
        return 1;
    }
    let notes = if micro {
        line(-0.13, -0.05, 0.13, -0.05) || line(-0.13, 0.07, 0.07, 0.07)
    } else {
        line(-0.15, -0.085, 0.15, -0.085) || line(-0.15, -0.015, 0.06, -0.015)
    };
    let code = !micro && (line(-0.075, 0.085, -0.15, 0.145) || line(-0.15, 0.145, -0.075, 0.205)
        || line(0.075, 0.085, 0.15, 0.145) || line(0.15, 0.145, 0.075, 0.205) || line(0.03, 0.075, -0.03, 0.215));
    if page.abs() <= w || notes || code { 1 } else { 0 }
}
fn notepad_sample(micro: bool, u: f32, v: f32) -> [f32; 4] {
    let (x, y) = (u - 0.5, v - 0.5);
    let d = rbox(x, y, 0.0, 0.0, 0.453125, 0.453125, 0.15625);
    let rim_width = if micro { 0.015 } else { 0.01094 };
    let t = ((u + v) * 0.5).clamp(0.0, 1.0);
    let a = accent("notepad");
    let stops: &[(f32, C3)] = &[
        (0.0, [0.855, 0.941, 1.0]), (0.16, mix(a, [0.80, 0.80, 0.80], 0.55)),
        (0.42, mix(a, [0.08, 0.08, 0.10], 0.70)), (0.70, mix(a, [0.18, 0.18, 0.20], 0.38)),
        (1.0, mix(a, [1.0; 3], 0.45)),
    ];
    let mut rim = stops[stops.len()-1].1;
    for pair in stops.windows(2) {
        if t >= pair[0].0 && t <= pair[1].0 {
            rim = mix(pair[0].1, pair[1].1, (t-pair[0].0)/(pair[1].0-pair[0].0));
            break;
        }
    }
    if d > rim_width {
        let alpha = (-((d-rim_width)/0.018).powi(2)).exp() * 0.24;
        return [rim[0], rim[1], rim[2], alpha];
    }
    let mut c = if t < 0.5 {
        mix([0.106,0.110,0.188], [0.063,0.071,0.118], t*2.0)
    } else {
        mix([0.063,0.071,0.118], [0.031,0.043,0.071], (t-0.5)*2.0)
    };
    let glow = (1.0-len(x,y+0.025)/0.41).max(0.0) * 0.14;
    c = mix(c, a, glow);
    if d >= -rim_width { c = rim; }
    let inner = rbox(x,y,0.0,0.0,0.421875,0.421875,0.1328);
    if !micro && inner.abs() < 0.006 { c = mix(c,mix(a,[1.0;3],0.7),0.07); }
    match notepad_glyph(micro, x, y) {
        1 => c = mix(mix(a,[1.0;3],0.62), mix(a,[0.15,0.15,0.2],0.2), (y+0.30)/0.62),
        2 => c = mix(mix(a,[1.0;3],0.22), mix(a,[0.08,0.08,0.1],0.22), (y+0.25)/0.10),
        _ => {}
    }
    [c[0],c[1],c[2],1.0]
}
// Shared outlined artwork for the rest of the installed Axentra suite.
fn premium_glyph(app: &str, micro: bool, x: f32, y: f32) -> bool {
    let w = if micro { 0.018 } else { 0.011 };
    let line = |ax, ay, bx, by| seg(x,y,ax,ay,bx,by) <= w;
    let outline = |cx,cy,hw,hh,r| rbox(x,y,cx,cy,hw,hh,r).abs() <= w;
    match app {
        "command" => {
            let core = (0..6).any(|i| { let a = i as f32*PI/3.0-PI/2.0; let b=a+PI/3.0;
                line(0.123*a.cos(),0.123*a.sin(),0.123*b.cos(),0.123*b.sin()) });
            core || (0..6).any(|i| { let (nx,ny)=node(i);
                outline(nx,ny,0.056,0.056,0.017) || line(nx*0.48,ny*0.48,nx*0.78,ny*0.78) })
        }
        "ghost" => {
            let head = y<=-0.025 && (len(x,y+0.025)-0.225).abs()<=w;
            let sides = line(-0.225,-0.025,-0.225,0.255) || line(0.225,-0.025,0.225,0.255);
            let hem = [(-0.225,0.255,-0.15,0.195),(-0.15,0.195,-0.075,0.255),(-0.075,0.255,0.0,0.195),
                (0.0,0.195,0.075,0.255),(0.075,0.255,0.15,0.195),(0.15,0.195,0.225,0.255)]
                .iter().any(|&(a,b,c,d)|line(a,b,c,d));
            head || sides || hem || [-0.082,0.082].iter().any(|ex|rbox(x,y,*ex,-0.02,0.022,0.040,0.022)<=0.0)
        }
        "pdf" => {
            let paper = outline(0.0,0.0,0.23,0.30,0.027) && y>=x-0.37;
            paper || line(0.07,-0.30,0.23,-0.14) || line(0.07,-0.30,0.07,-0.14) || line(0.07,-0.14,0.23,-0.14)
                || line(-0.13,0.20,0.0,-0.07) || line(0.0,-0.07,0.13,0.20) || line(-0.075,0.09,0.075,0.09)
        }
        "spaces" => {
            let front = outline(-0.07,0.08,0.24,0.18,0.028) || line(-0.30,-0.015,0.16,-0.015);
            let mask = rbox(x,y,-0.07,0.08,0.25,0.19,0.03)<0.0;
            front || (!mask && (outline(0.09,-0.105,0.24,0.18,0.028)||line(-0.14,-0.20,0.32,-0.20)))
        }
        "mail" => outline(0.0,0.0,0.29,0.21,0.035) || line(-0.27,-0.18,0.0,0.045) || line(0.0,0.045,0.27,-0.18)
            || (!micro && (line(-0.27,0.19,-0.105,0.015)||line(0.27,0.19,0.105,0.015))),
        "outlook" => {
            let env = outline(-0.045,0.105,0.26,0.16,0.025) || line(-0.29,-0.035,-0.045,0.13) || line(-0.045,0.13,0.20,-0.035);
            let behind = y < -0.065 && (outline(0.08,-0.105,0.235,0.18,0.025) || line(-0.14,-0.18,0.30,-0.18));
            env || behind || line(-0.045,-0.32,-0.045,-0.24) || line(0.185,-0.32,0.185,-0.24)
        }
        "zip" => {
            let doc=outline(0.0,0.0,0.225,0.30,0.03);
            let teeth=(0..if micro {4} else {6}).any(|i| {let step=if micro {0.09}else{0.06};let cx=if i%2==0{-0.025}else{0.025};
                rbox(x,y,cx,-0.245+i as f32*step,0.025,step*0.23,0.003)<=0.0 });
            doc || teeth || outline(0.0,0.17,0.047,0.060,0.012)
        }
        "usb" => outline(0.0,0.08,0.145,0.225,0.035) || outline(0.0,-0.225,0.105,0.08,0.012)
            || (!micro && (line(-0.040,-0.25,-0.040,-0.205)||line(0.040,-0.25,0.040,-0.205)))
            || line(0.0,-0.055,0.0,0.16) || line(0.0,0.16,-0.07,0.085) || line(0.0,0.16,0.07,0.085),
        "minitube" => outline(0.0,0.0,0.30,0.23,0.037) || line(-0.07,-0.12,-0.07,0.12)
            || line(-0.07,-0.12,0.13,0.0) || line(0.13,0.0,-0.07,0.12),
        "onedrive" | "onedrive-azure" | "onedrive-periwinkle" => {
            let curve = |points: [(f32,f32);4]| {
                let mut previous=points[0];let mut hit=false;
                for i in 1..=24 {let t=i as f32/24.0;let u=1.0-t;
                    let px=u*u*u*points[0].0+3.0*u*u*t*points[1].0+3.0*u*t*t*points[2].0+t*t*t*points[3].0;
                    let py=u*u*u*points[0].1+3.0*u*u*t*points[1].1+3.0*u*t*t*points[2].1+t*t*t*points[3].1;
                    hit |= line(previous.0,previous.1,px,py);previous=(px,py);
                }hit
            };
            curve([(-0.23,0.15),(-0.36,0.15),(-0.36,-0.055),(-0.20,-0.055)])
                || curve([(-0.20,-0.055),(-0.18,-0.29),(0.12,-0.29),(0.145,-0.055)])
                || curve([(0.145,-0.055),(0.35,-0.08),(0.37,0.15),(0.22,0.15)])
                || line(-0.23,0.15,0.22,0.15)
        }
        _ => false,
    }
}
fn premium_sample(app: &str, micro: bool, u:f32,v:f32)->[f32;4] {
    let (x,y)=(u-0.5,v-0.5);
    let d=rbox(x,y,0.0,0.0,0.453125,0.453125,0.15625);
    let rw=if micro {0.015}else{0.01094};let t=((u+v)*0.5).clamp(0.0,1.0);let a=accent(app);
    let stops=[(0.0,[0.855,0.941,1.0]),(0.16,mix(a,[0.82,0.84,0.86],0.60)),
        (0.42,mix(a,[0.10,0.11,0.15],0.72)),(0.70,mix(a,[0.20,0.21,0.25],0.35)),(1.0,mix(a,[0.94,0.95,1.0],0.48))];
    let mut rim=stops[4].1;
    for p in stops.windows(2) {if t>=p[0].0&&t<=p[1].0 {rim=mix(p[0].1,p[1].1,(t-p[0].0)/(p[1].0-p[0].0));break;}}
    if d>rw {return [rim[0],rim[1],rim[2],(-((d-rw)/0.018).powi(2)).exp()*0.24];}
    let mut c=if t<0.5 {mix([0.106,0.110,0.188],[0.063,0.071,0.118],t*2.0)}else{mix([0.063,0.071,0.118],[0.031,0.043,0.071],(t-0.5)*2.0)};
    c=mix(c,a,(1.0-len(x,y+0.025)/0.41).max(0.0)*0.14);
    if d>=-rw {c=rim;}
    if !micro&&rbox(x,y,0.0,0.0,0.421875,0.421875,0.1328).abs()<0.006 {c=mix(c,[0.80,0.86,0.93],0.07);}
    if app=="vault" {c=vault_finish(c,micro,x,y);}
    else if premium_glyph(app,micro,x,y) {
        c=mix(mix(a,[1.0;3],0.62),mix(a,[0.20,0.23,0.26],0.20),(y+0.32)/0.64);
        if app=="command" {for (i,id) in ["vault","crystalwire","horizon","spaces","pdf","ghost"].iter().enumerate(){let(nx,ny)=node(i);if rbox(x,y,nx,ny,0.062,0.062,0.02)<=0.0{c=mix(accent(id),[0.90,0.95,1.0],0.35);}}}
    }
    [c[0],c[1],c[2],1.0]
}
fn sample(app: &str, micro: bool, u: f32, v: f32) -> [f32; 4] {
    if app == "excel" { return excel_sample(micro, u, v); }
    if app == "notepad" { return notepad_sample(micro, u, v); }
    if matches!(app,"command"|"ghost"|"vault"|"pdf"|"spaces"|"mail"|"zip"|"usb"|"minitube"|"onedrive"|"outlook"|"onedrive-azure"|"onedrive-periwinkle") { return premium_sample(app,micro,u,v); }
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
