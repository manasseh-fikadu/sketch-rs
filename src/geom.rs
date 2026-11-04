//! Geometry primitives and the canvas camera.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

pub const fn v2(x: f64, y: f64) -> V2 {
    V2 { x, y }
}

impl V2 {
    pub const ZERO: V2 = v2(0.0, 0.0);
    pub fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn dist(self, o: V2) -> f64 {
        (self - o).len()
    }
    pub fn dot(self, o: V2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn normalized(self) -> V2 {
        let l = self.len();
        if l > 1e-12 { self / l } else { V2::ZERO }
    }
    /// Left-hand perpendicular (rotated +90° in a y-down coordinate system).
    pub fn perp(self) -> V2 {
        v2(-self.y, self.x)
    }
    pub fn lerp(self, o: V2, t: f64) -> V2 {
        self + (o - self) * t
    }
    pub fn round(self) -> V2 {
        v2(self.x.round(), self.y.round())
    }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        v2(self.x + o.x, self.y + o.y)
    }
}
impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        v2(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for V2 {
    type Output = V2;
    fn mul(self, s: f64) -> V2 {
        v2(self.x * s, self.y * s)
    }
}
impl Div<f64> for V2 {
    type Output = V2;
    fn div(self, s: f64) -> V2 {
        v2(self.x / s, self.y / s)
    }
}
impl Neg for V2 {
    type Output = V2;
    fn neg(self) -> V2 {
        v2(-self.x, -self.y)
    }
}

/// Axis-aligned rectangle with non-negative size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    /// Rectangle spanning two arbitrary corner points.
    pub fn from_points(a: V2, b: V2) -> Rect {
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }
    pub fn min(&self) -> V2 {
        v2(self.x, self.y)
    }
    pub fn max(&self) -> V2 {
        v2(self.x + self.w, self.y + self.h)
    }
    pub fn center(&self) -> V2 {
        v2(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn contains(&self, p: V2) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x <= o.right() && o.x <= self.right() && self.y <= o.bottom() && o.y <= self.bottom()
    }
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }
    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r >= x && b >= y).then(|| Rect::new(x, y, r - x, b - y))
    }
    pub fn translate(&self, d: V2) -> Rect {
        Rect::new(self.x + d.x, self.y + d.y, self.w, self.h)
    }
    pub fn inflate(&self, m: f64) -> Rect {
        Rect::new(self.x - m, self.y - m, self.w + 2.0 * m, self.h + 2.0 * m)
    }
    pub fn union_all<'a>(rects: impl IntoIterator<Item = &'a Rect>) -> Option<Rect> {
        rects
            .into_iter()
            .fold(None, |acc, r| Some(acc.map_or(*r, |a: Rect| a.union(r))))
    }
}

pub const MIN_ZOOM: f64 = 0.01;
pub const MAX_ZOOM: f64 = 256.0;

/// Maps world (document) coordinates to screen (logical pixel) coordinates:
/// `screen = (world - pan) * zoom`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// World coordinate shown at the top-left corner of the viewport.
    pub pan: V2,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            pan: V2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, p: V2) -> V2 {
        (p - self.pan) * self.zoom
    }
    pub fn screen_to_world(&self, p: V2) -> V2 {
        p / self.zoom + self.pan
    }
    pub fn world_rect_to_screen(&self, r: &Rect) -> Rect {
        let a = self.world_to_screen(r.min());
        Rect::new(a.x, a.y, r.w * self.zoom, r.h * self.zoom)
    }
    /// Pan by a delta expressed in screen pixels.
    pub fn pan_by_screen(&mut self, d: V2) {
        self.pan = self.pan - d / self.zoom;
    }
    /// Zoom so that the world point under `anchor` (screen) stays fixed.
    pub fn zoom_about(&mut self, anchor: V2, new_zoom: f64) {
        let world = self.screen_to_world(anchor);
        self.zoom = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = world - anchor / self.zoom;
    }
    /// Visible world rect for a viewport of the given screen size.
    pub fn visible_world(&self, viewport: V2) -> Rect {
        Rect::new(
            self.pan.x,
            self.pan.y,
            viewport.x / self.zoom,
            viewport.y / self.zoom,
        )
    }
    /// Fit `r` into the viewport with a screen-space margin, capping zoom at `max_zoom`.
    pub fn fit(&mut self, r: &Rect, viewport: V2, margin: f64, max_zoom: f64) {
        let avail = v2(
            (viewport.x - 2.0 * margin).max(1.0),
            (viewport.y - 2.0 * margin).max(1.0),
        );
        let z = (avail.x / r.w.max(1e-6))
            .min(avail.y / r.h.max(1e-6))
            .clamp(MIN_ZOOM, max_zoom);
        self.zoom = z;
        self.pan = r.center() - viewport / (2.0 * z);
    }
}

/// Zoom steps used by Cmd +/- (powers-of-two-ish ladder, like most design tools).
pub const ZOOM_STEPS: [f64; 17] = [
    0.02, 0.05, 0.1, 0.125, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0,
];

pub fn next_zoom_step(z: f64, up: bool) -> f64 {
    if up {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|s| *s > z * 1.001)
            .unwrap_or(MAX_ZOOM)
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < z * 0.999)
            .unwrap_or(MIN_ZOOM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_roundtrip() {
        let c = Camera {
            pan: v2(-120.5, 33.0),
            zoom: 2.75,
        };
        let p = v2(17.25, -401.0);
        let back = c.screen_to_world(c.world_to_screen(p));
        assert!(back.dist(p) < 1e-9);
    }

    #[test]
    fn zoom_about_keeps_anchor_fixed() {
        let mut c = Camera {
            pan: v2(10.0, 20.0),
            zoom: 1.0,
        };
        let anchor = v2(400.0, 300.0);
        let before = c.screen_to_world(anchor);
        for z in [2.0, 0.37, 13.0, 1000.0, 0.0001] {
            c.zoom_about(anchor, z);
            assert!(c.screen_to_world(anchor).dist(before) < 1e-6);
        }
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn fit_centers_rect() {
        let mut c = Camera::default();
        let r = Rect::new(1000.0, 1000.0, 200.0, 100.0);
        c.fit(&r, v2(800.0, 600.0), 50.0, 10.0);
        let sc = c.world_to_screen(r.center());
        assert!(sc.dist(v2(400.0, 300.0)) < 1e-6);
        assert!((c.zoom - 3.5).abs() < 1e-9);
    }

    #[test]
    fn rect_ops() {
        let a = Rect::from_points(v2(10.0, 10.0), v2(0.0, 0.0));
        assert_eq!(a, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert!(a.intersects(&b));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(a.intersection(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert!(Rect::new(20.0, 0.0, 1.0, 1.0).intersection(&a).is_none());
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(next_zoom_step(1.0, true), 1.5);
        assert_eq!(next_zoom_step(1.0, false), 0.75);
        assert_eq!(next_zoom_step(1.2, true), 1.5);
    }
}
