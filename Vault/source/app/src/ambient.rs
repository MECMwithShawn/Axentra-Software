//! Ambient backdrop for the unlock screen: a deep-navy radial glow with a slow "constellation"
//! of glowing particles that drift upward, join with faint lines when near each other, and ease
//! away from the mouse pointer. Ported from the Rooted in the Word site's canvas background
//! (same counts, speeds, link distance and pointer radius), recoloured for Vault.

use std::time::Instant;

use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Stroke, pos2, vec2};

const LINK_DISTANCE: f32 = 128.0;
const POINTER_RADIUS: f32 = 165.0;
const POINTER_PUSH: f32 = 2.8;
const LINK_ALPHA: f32 = 0.20;
const TONES: [Color32; 4] = [
    Color32::from_rgb(0x22, 0xd3, 0xee),
    Color32::from_rgb(0x5f, 0xe3, 0xff),
    Color32::from_rgb(0x2f, 0x8b, 0xd3),
    Color32::from_rgb(0xa6, 0xe9, 0xff),
];
const LINK: Color32 = Color32::from_rgb(0x5f, 0xe3, 0xff);
const BASE: Color32 = Color32::from_rgb(0x05, 0x09, 0x11);
const GLOW: Color32 = Color32::from_rgb(0x12, 0x34, 0x57);

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    size: f32,
    alpha: f32,
    tone: usize,
}

pub struct Ambient {
    particles: Vec<Particle>,
    size: egui::Vec2,
    last: Option<Instant>,
    rng: u64,
    animate: bool,
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Radial gradient disc: `inner` at the centre fading to `outer` at `radius`.
pub fn radial(painter: &egui::Painter, center: Pos2, radius: f32, inner: Color32, outer: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, inner);
    let n = 72;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + vec2(a.cos(), a.sin()) * radius, outer);
    }
    for i in 1..=n as u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

impl Ambient {
    /// With `animate` false (Windows animation effects off) the field is drawn once, still.
    pub fn new(animate: bool) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        Ambient {
            particles: Vec::new(),
            size: egui::Vec2::ZERO,
            last: None,
            rng: seed | 1,
            animate,
        }
    }

    fn random(&mut self) -> f32 {
        // xorshift64*: plenty for decoration, never used for secrets.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn spawn(&mut self, anywhere: bool) -> Particle {
        let (w, h) = (self.size.x, self.size.y);
        Particle {
            x: self.random() * w,
            y: if anywhere {
                self.random() * h
            } else {
                h + 24.0
            },
            vx: (self.random() - 0.5) * 0.25,
            vy: -(0.34 * self.random() + 0.12),
            size: 1.45 * self.random() + 0.75,
            alpha: 0.3 * self.random() + 0.32,
            tone: (self.random() * TONES.len() as f32) as usize % TONES.len(),
        }
    }

    /// Paints the backdrop over `rect` and advances the animation one frame.
    pub fn paint(&mut self, ui: &egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, BASE);
        let glow_center = pos2(rect.center().x, rect.top() + rect.height() * 0.34);
        radial(
            &painter,
            glow_center,
            rect.width().max(rect.height()) * 0.62,
            GLOW,
            with_alpha(GLOW, 0.0),
        );

        if (self.size - rect.size()).length() > 1.0 {
            self.size = rect.size();
            let count = ((self.size.x * self.size.y) / 19_000.0)
                .round()
                .clamp(34.0, 72.0) as usize;
            self.particles = (0..count).map(|_| self.spawn(true)).collect();
        }

        // Frame-rate independent: speeds above are per 60 Hz frame, as on the site.
        let now = Instant::now();
        let step = self
            .last
            .map(|t| (now - t).as_secs_f32().min(0.05) * 60.0)
            .unwrap_or(1.0);
        self.last = Some(now);

        if self.animate {
            let pointer = ui.ctx().pointer_hover_pos().map(|p| p - rect.min.to_vec2());
            for i in 0..self.particles.len() {
                let p = &mut self.particles[i];
                p.x += p.vx * step;
                p.y += p.vy * step;
                if let Some(m) = pointer {
                    let (dx, dy) = (p.x - m.x, p.y - m.y);
                    let d = (dx * dx + dy * dy).sqrt();
                    if d > 0.0 && d < POINTER_RADIUS {
                        let push = (1.0 - d / POINTER_RADIUS) * POINTER_PUSH * step;
                        p.x += dx / d * push;
                        p.y += dy / d * push;
                    }
                }
                if p.y < -30.0 || p.x < -30.0 || p.x > self.size.x + 30.0 {
                    self.particles[i] = self.spawn(false);
                }
            }
            ui.ctx().request_repaint();
        }

        let at = |p: &Particle| rect.min + vec2(p.x, p.y);
        for i in 0..self.particles.len() {
            for j in i + 1..self.particles.len() {
                let (a, b) = (&self.particles[i], &self.particles[j]);
                let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                if d < LINK_DISTANCE {
                    painter.line_segment(
                        [at(a), at(b)],
                        Stroke::new(
                            0.7,
                            with_alpha(LINK, (1.0 - d / LINK_DISTANCE) * LINK_ALPHA),
                        ),
                    );
                }
            }
        }
        for p in &self.particles {
            let c = TONES[p.tone];
            painter.circle_filled(at(p), p.size * 3.4, with_alpha(c, p.alpha * 0.14));
            painter.circle_filled(at(p), p.size * 1.8, with_alpha(c, p.alpha * 0.28));
            painter.circle_filled(at(p), p.size, with_alpha(c, p.alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_stays_in_unit_range() {
        let mut a = Ambient::new(false);
        for _ in 0..10_000 {
            let r = a.random();
            assert!((0.0..1.0).contains(&r), "{r}");
        }
    }

    #[test]
    fn particles_rise_and_respawn_below() {
        let mut a = Ambient::new(true);
        a.size = vec2(800.0, 600.0);
        let p = a.spawn(false);
        assert!(
            p.y > 600.0 && p.vy < 0.0,
            "new particles enter from below and rise"
        );
        assert!(p.size >= 0.75 && p.size <= 2.2);
    }
}
