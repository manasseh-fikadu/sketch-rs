//! Vector paths in world (or local) coordinates, shared by the GPU tessellator, hit testing,
//! SVG/PDF export and boolean operations.

use crate::geom::{Rect, V2, v2};
use crate::model::{PathPoint, SubPath};
use std::fmt::Write;

/// Cubic approximation constant for a quarter circle.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    M(V2),
    L(V2),
    Q(V2, V2),
    C(V2, V2, V2),
    Z,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VPath {
    pub segs: Vec<Seg>,
}

/// A flattened subpath.
#[derive(Debug, Clone, PartialEq)]
pub struct Poly {
    pub pts: Vec<V2>,
    pub closed: bool,
}

impl VPath {
    pub fn new() -> VPath {
        VPath::default()
    }
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }
    pub fn move_to(&mut self, p: V2) -> &mut Self {
        self.segs.push(Seg::M(p));
        self
    }
    pub fn line_to(&mut self, p: V2) -> &mut Self {
        self.segs.push(Seg::L(p));
        self
    }
    pub fn quad_to(&mut self, c: V2, p: V2) -> &mut Self {
        self.segs.push(Seg::Q(c, p));
        self
    }
    pub fn cubic_to(&mut self, c1: V2, c2: V2, p: V2) -> &mut Self {
        self.segs.push(Seg::C(c1, c2, p));
        self
    }
    pub fn close(&mut self) -> &mut Self {
        self.segs.push(Seg::Z);
        self
    }
    pub fn append(&mut self, o: &VPath) {
        self.segs.extend_from_slice(&o.segs);
    }

    /// Polyline from points.
    pub fn polyline(pts: &[V2], closed: bool) -> VPath {
        let mut p = VPath::new();
        if let Some(f) = pts.first() {
            p.move_to(*f);
            for q in &pts[1..] {
                p.line_to(*q);
            }
            if closed {
                p.close();
            }
        }
        p
    }

    pub fn map(&self, f: impl Fn(V2) -> V2) -> VPath {
        VPath {
            segs: self
                .segs
                .iter()
                .map(|s| match *s {
                    Seg::M(p) => Seg::M(f(p)),
                    Seg::L(p) => Seg::L(f(p)),
                    Seg::Q(c, p) => Seg::Q(f(c), f(p)),
                    Seg::C(a, b, p) => Seg::C(f(a), f(b), f(p)),
                    Seg::Z => Seg::Z,
                })
                .collect(),
        }
    }

    pub fn translate(&self, d: V2) -> VPath {
        self.map(|p| p + d)
    }

    /// Bounds of all points including control points (a cheap superset).
    pub fn bounds(&self) -> Option<Rect> {
        let mut pts = Vec::new();
        for s in &self.segs {
            match *s {
                Seg::M(p) | Seg::L(p) => pts.push(p),
                Seg::Q(c, p) => pts.extend([c, p]),
                Seg::C(a, b, p) => pts.extend([a, b, p]),
                Seg::Z => {}
            }
        }
        let f = pts.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x, f.y);
        for p in &pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Flatten curves into polylines with the given tolerance.
    pub fn flatten(&self, tol: f64) -> Vec<Poly> {
        let mut out: Vec<Poly> = Vec::new();
        let mut cur: Vec<V2> = Vec::new();
        let mut start = V2::ZERO;
        let tol = tol.max(1e-6);
        let flush = |cur: &mut Vec<V2>, out: &mut Vec<Poly>, closed: bool| {
            if cur.len() > 1 || (cur.len() == 1 && closed) {
                out.push(Poly {
                    pts: std::mem::take(cur),
                    closed,
                });
            } else {
                cur.clear();
            }
        };
        for s in &self.segs {
            match *s {
                Seg::M(p) => {
                    flush(&mut cur, &mut out, false);
                    cur.push(p);
                    start = p;
                }
                Seg::L(p) => {
                    if cur.is_empty() {
                        cur.push(start);
                    }
                    cur.push(p)
                }
                Seg::Q(c, p) => {
                    let a = *cur.last().unwrap_or(&start);
                    if cur.is_empty() {
                        cur.push(a);
                    }
                    let n = segments_for(a.dist(c) + c.dist(p), tol);
                    for i in 1..=n {
                        let t = i as f64 / n as f64;
                        let u = 1.0 - t;
                        cur.push(a * (u * u) + c * (2.0 * u * t) + p * (t * t));
                    }
                }
                Seg::C(c1, c2, p) => {
                    let a = *cur.last().unwrap_or(&start);
                    if cur.is_empty() {
                        cur.push(a);
                    }
                    let n = segments_for(a.dist(c1) + c1.dist(c2) + c2.dist(p), tol);
                    for i in 1..=n {
                        cur.push(cubic(a, c1, c2, p, i as f64 / n as f64));
                    }
                }
                Seg::Z => {
                    if cur.len() > 1 && cur.first().unwrap().dist(*cur.last().unwrap()) < 1e-9 {
                        cur.pop();
                    }
                    flush(&mut cur, &mut out, true);
                    cur.clear();
                }
            }
        }
        flush(&mut cur, &mut out, false);
        out
    }

    pub fn to_lyon(&self) -> lyon::path::Path {
        use lyon::math::point;
        let p = |v: V2| point(v.x as f32, v.y as f32);
        let mut b = lyon::path::Path::builder();
        let mut open = false;
        let mut last = V2::ZERO;
        for s in &self.segs {
            match *s {
                Seg::M(q) => {
                    if open {
                        b.end(false);
                    }
                    b.begin(p(q));
                    open = true;
                    last = q;
                }
                Seg::L(q) => {
                    if !open {
                        b.begin(p(last));
                        open = true;
                    }
                    b.line_to(p(q));
                    last = q;
                }
                Seg::Q(c, q) => {
                    if !open {
                        b.begin(p(last));
                        open = true;
                    }
                    b.quadratic_bezier_to(p(c), p(q));
                    last = q;
                }
                Seg::C(c1, c2, q) => {
                    if !open {
                        b.begin(p(last));
                        open = true;
                    }
                    b.cubic_bezier_to(p(c1), p(c2), p(q));
                    last = q;
                }
                Seg::Z => {
                    if open {
                        b.end(true);
                        open = false;
                    }
                }
            }
        }
        if open {
            b.end(false);
        }
        b.build()
    }

    /// SVG path data with 2-decimal precision.
    pub fn to_svg(&self) -> String {
        let mut d = String::new();
        for s in &self.segs {
            match *s {
                Seg::M(p) => {
                    let _ = write!(d, "M{} {}", n(p.x), n(p.y));
                }
                Seg::L(p) => {
                    let _ = write!(d, "L{} {}", n(p.x), n(p.y));
                }
                Seg::Q(c, p) => {
                    let _ = write!(d, "Q{} {} {} {}", n(c.x), n(c.y), n(p.x), n(p.y));
                }
                Seg::C(a, b, p) => {
                    let _ = write!(
                        d,
                        "C{} {} {} {} {} {}",
                        n(a.x),
                        n(a.y),
                        n(b.x),
                        n(b.y),
                        n(p.x),
                        n(p.y)
                    );
                }
                Seg::Z => d.push('Z'),
            }
        }
        d
    }

    /// Point-in-path test.
    pub fn contains(&self, q: V2, even_odd: bool, tol: f64) -> bool {
        let mut winding = 0i32;
        let mut crossings = 0u32;
        for poly in self.flatten(tol) {
            let pts = &poly.pts;
            let n = pts.len();
            if n < 3 {
                continue;
            }
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.y <= q.y) != (b.y <= q.y) {
                    let x = a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x);
                    if x > q.x {
                        crossings += 1;
                        winding += if b.y > a.y { 1 } else { -1 };
                    }
                }
            }
        }
        if even_odd {
            crossings % 2 == 1
        } else {
            winding != 0
        }
    }

    /// Shortest distance from `q` to the outline.
    pub fn distance(&self, q: V2, tol: f64) -> f64 {
        let mut best = f64::INFINITY;
        for poly in self.flatten(tol) {
            let pts = &poly.pts;
            let n = pts.len();
            let segs = if poly.closed { n } else { n.saturating_sub(1) };
            if n == 1 {
                best = best.min(pts[0].dist(q));
            }
            for i in 0..segs {
                best = best.min(seg_dist(pts[i], pts[(i + 1) % n], q));
            }
        }
        best
    }
}

fn n(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r == 0.0 {
        "0".into()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn segments_for(len: f64, tol: f64) -> usize {
    ((len / (8.0 * tol).sqrt().max(1e-3)).sqrt() * 1.5)
        .ceil()
        .clamp(1.0, 200.0) as usize
}

pub fn cubic(a: V2, b: V2, c: V2, d: V2, t: f64) -> V2 {
    let u = 1.0 - t;
    a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t)
}

pub fn seg_dist(a: V2, b: V2, q: V2) -> f64 {
    let d = b - a;
    let l2 = d.dot(d);
    if l2 < 1e-18 {
        return a.dist(q);
    }
    let t = ((q - a).dot(d) / l2).clamp(0.0, 1.0);
    (a + d * t).dist(q)
}

// ---------------------------------------------------------------------------------- shapes

/// Rectangle with per-corner radii (tl, tr, br, bl), clamped to fit.
pub fn rounded_rect(r: Rect, radii: [f64; 4]) -> VPath {
    let max = (r.w.min(r.h) * 0.5).max(0.0);
    let [tl, tr, br, bl] = radii.map(|x| x.clamp(0.0, max));
    let mut p = VPath::new();
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let k = 1.0 - KAPPA;
    p.move_to(v2(x0 + tl, y0));
    p.line_to(v2(x1 - tr, y0));
    if tr > 0.0 {
        p.cubic_to(v2(x1 - tr * k, y0), v2(x1, y0 + tr * k), v2(x1, y0 + tr));
    }
    p.line_to(v2(x1, y1 - br));
    if br > 0.0 {
        p.cubic_to(v2(x1, y1 - br * k), v2(x1 - br * k, y1), v2(x1 - br, y1));
    }
    p.line_to(v2(x0 + bl, y1));
    if bl > 0.0 {
        p.cubic_to(v2(x0 + bl * k, y1), v2(x0, y1 - bl * k), v2(x0, y1 - bl));
    }
    p.line_to(v2(x0, y0 + tl));
    if tl > 0.0 {
        p.cubic_to(v2(x0, y0 + tl * k), v2(x0 + tl * k, y0), v2(x0 + tl, y0));
    }
    p.close();
    p
}

pub fn ellipse(r: Rect) -> VPath {
    let c = r.center();
    let (rx, ry) = (r.w * 0.5, r.h * 0.5);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let mut p = VPath::new();
    p.move_to(v2(c.x + rx, c.y));
    p.cubic_to(
        v2(c.x + rx, c.y + ky),
        v2(c.x + kx, c.y + ry),
        v2(c.x, c.y + ry),
    );
    p.cubic_to(
        v2(c.x - kx, c.y + ry),
        v2(c.x - rx, c.y + ky),
        v2(c.x - rx, c.y),
    );
    p.cubic_to(
        v2(c.x - rx, c.y - ky),
        v2(c.x - kx, c.y - ry),
        v2(c.x, c.y - ry),
    );
    p.cubic_to(
        v2(c.x + kx, c.y - ry),
        v2(c.x + rx, c.y - ky),
        v2(c.x + rx, c.y),
    );
    p.close();
    p
}

/// Vertices of a regular polygon inscribed in `r`, first vertex at the top center.
pub fn polygon_points(r: Rect, sides: u32) -> Vec<V2> {
    let n = sides.max(3) as usize;
    let c = r.center();
    (0..n)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / n as f64;
            v2(c.x + a.cos() * r.w * 0.5, c.y + a.sin() * r.h * 0.5)
        })
        .collect()
}

pub fn star_points(r: Rect, points: u32, inner: f64) -> Vec<V2> {
    let n = points.max(3) as usize * 2;
    let c = r.center();
    (0..n)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / n as f64;
            let k = if i % 2 == 0 {
                1.0
            } else {
                inner.clamp(0.01, 1.0)
            };
            v2(c.x + a.cos() * r.w * 0.5 * k, c.y + a.sin() * r.h * 0.5 * k)
        })
        .collect()
}

/// Polygon with rounded corners (radius clamped per corner).
pub fn rounded_polygon(pts: &[V2], radius: f64) -> VPath {
    if radius <= 0.0 || pts.len() < 3 {
        return VPath::polyline(pts, true);
    }
    let n = pts.len();
    let mut p = VPath::new();
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        let (d0, d1) = ((prev - cur).normalized(), (next - cur).normalized());
        let r = radius.min(prev.dist(cur) * 0.5).min(next.dist(cur) * 0.5);
        let a = cur + d0 * r;
        let b = cur + d1 * r;
        if i == 0 {
            p.move_to(a);
        } else {
            p.line_to(a);
        }
        p.cubic_to(a + (cur - a) * KAPPA, b + (cur - b) * KAPPA, b);
    }
    p.close();
    p
}

/// A node's vector subpaths, offset by `origin`.
pub fn from_subpaths(origin: V2, subpaths: &[SubPath]) -> VPath {
    let mut p = VPath::new();
    for sp in subpaths {
        let Some(first) = sp.points.first() else {
            continue;
        };
        p.move_to(origin + first.p);
        let n = sp.points.len();
        let count = if sp.closed { n } else { n - 1 };
        for i in 0..count {
            let a = &sp.points[i];
            let b = &sp.points[(i + 1) % n];
            seg_between(&mut p, origin, a, b);
        }
        if sp.closed {
            p.close();
        }
    }
    p
}

fn seg_between(p: &mut VPath, o: V2, a: &PathPoint, b: &PathPoint) {
    match (a.h_out, b.h_in) {
        (None, None) => {
            p.line_to(o + b.p);
        }
        (c1, c2) => {
            p.cubic_to(o + c1.unwrap_or(a.p), o + c2.unwrap_or(b.p), o + b.p);
        }
    }
}

/// Polylines (from booleans or imports) into subpaths with corner points.
pub fn polys_to_subpaths(polys: &[Poly], origin: V2) -> Vec<SubPath> {
    polys
        .iter()
        .map(|p| SubPath {
            points: p
                .pts
                .iter()
                .map(|q| PathPoint::corner(*q - origin))
                .collect(),
            closed: p.closed,
        })
        .collect()
}

/// Convert any path into subpaths (keeping cubic segments as handles).
pub fn to_subpaths(path: &VPath, origin: V2) -> Vec<SubPath> {
    let mut out: Vec<SubPath> = Vec::new();
    let mut cur: Option<SubPath> = None;
    let mut last = V2::ZERO;
    for s in &path.segs {
        match *s {
            Seg::M(p) => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                cur = Some(SubPath {
                    points: vec![PathPoint::corner(p - origin)],
                    closed: false,
                });
                last = p;
            }
            Seg::L(p) => {
                let c = cur.get_or_insert_with(|| SubPath {
                    points: vec![PathPoint::corner(last - origin)],
                    closed: false,
                });
                c.points.push(PathPoint::corner(p - origin));
                last = p;
            }
            Seg::Q(q, p) => {
                // Elevate to cubic.
                let c1 = last + (q - last) * (2.0 / 3.0);
                let c2 = p + (q - p) * (2.0 / 3.0);
                let c = cur.get_or_insert_with(|| SubPath {
                    points: vec![PathPoint::corner(last - origin)],
                    closed: false,
                });
                c.points.last_mut().unwrap().h_out = Some(c1 - origin);
                c.points.push(PathPoint {
                    p: p - origin,
                    h_in: Some(c2 - origin),
                    h_out: None,
                });
                last = p;
            }
            Seg::C(c1, c2, p) => {
                let c = cur.get_or_insert_with(|| SubPath {
                    points: vec![PathPoint::corner(last - origin)],
                    closed: false,
                });
                c.points.last_mut().unwrap().h_out = Some(c1 - origin);
                c.points.push(PathPoint {
                    p: p - origin,
                    h_in: Some(c2 - origin),
                    h_out: None,
                });
                last = p;
            }
            Seg::Z => {
                if let Some(mut c) = cur.take() {
                    // Merge a closing point that duplicates the first one.
                    if c.points.len() > 1
                        && c.points.first().unwrap().p.dist(c.points.last().unwrap().p) < 1e-9
                    {
                        let l = c.points.pop().unwrap();
                        c.points[0].h_in = l.h_in;
                    }
                    c.closed = true;
                    last = origin + c.points[0].p;
                    out.push(c);
                }
            }
        }
    }
    if let Some(c) = cur {
        out.push(c);
    }
    out
}

/// Split a polyline into dashes (pattern lengths alternate on/off).
pub fn dash_polyline(pts: &[V2], closed: bool, pattern: &[f64]) -> Vec<Vec<V2>> {
    let pat: Vec<f64> = pattern.iter().map(|x| x.max(0.01)).collect();
    if pat.is_empty() || pts.len() < 2 {
        return vec![pts.to_vec()];
    }
    let mut seq: Vec<V2> = pts.to_vec();
    if closed {
        seq.push(pts[0]);
    }
    let mut out = Vec::new();
    let mut idx = 0;
    let mut left = pat[0];
    let mut on = true;
    let mut cur: Vec<V2> = vec![seq[0]];
    for w in seq.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = a.dist(b);
        while seg > left {
            let p = a + (b - a) * (left / seg);
            if on {
                cur.push(p);
                out.push(std::mem::take(&mut cur));
            } else {
                cur = vec![p];
            }
            seg -= left;
            a = p;
            on = !on;
            idx = (idx + 1) % pat.len();
            left = pat[idx];
        }
        left -= seg;
        if on {
            cur.push(b);
        }
    }
    if on && cur.len() > 1 {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_and_distance() {
        let p = rounded_rect(Rect::new(0.0, 0.0, 100.0, 50.0), [10.0; 4]);
        assert!(p.contains(v2(50.0, 25.0), false, 0.1));
        assert!(
            !p.contains(v2(1.0, 1.0), false, 0.1),
            "rounded corner excluded"
        );
        assert!(!p.contains(v2(150.0, 25.0), false, 0.1));
        assert!((p.distance(v2(50.0, -5.0), 0.1) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn ellipse_flattens_close_to_circle() {
        let p = ellipse(Rect::new(-10.0, -10.0, 20.0, 20.0));
        for poly in p.flatten(0.01) {
            assert!(poly.closed);
            for q in poly.pts {
                assert!((q.len() - 10.0).abs() < 0.03, "{}", q.len());
            }
        }
    }

    #[test]
    fn subpaths_roundtrip() {
        let p = ellipse(Rect::new(0.0, 0.0, 40.0, 20.0));
        let sp = to_subpaths(&p, v2(5.0, 5.0));
        assert_eq!(sp.len(), 1);
        assert!(sp[0].closed);
        assert_eq!(sp[0].points.len(), 4);
        let back = from_subpaths(v2(5.0, 5.0), &sp);
        let a: Vec<V2> = p.flatten(0.05).concat_pts();
        let b: Vec<V2> = back.flatten(0.05).concat_pts();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!(x.dist(*y) < 1e-9);
        }
    }

    trait Concat {
        fn concat_pts(self) -> Vec<V2>;
    }
    impl Concat for Vec<Poly> {
        fn concat_pts(self) -> Vec<V2> {
            self.into_iter().flat_map(|p| p.pts).collect()
        }
    }

    #[test]
    fn dashes() {
        let d = dash_polyline(&[v2(0.0, 0.0), v2(10.0, 0.0)], false, &[2.0, 3.0]);
        assert_eq!(d.len(), 2);
        assert!((d[0][1].x - 2.0).abs() < 1e-9);
        assert!((d[1][0].x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn star_and_polygon() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let tri = polygon_points(r, 3);
        assert!((tri[0].x - 50.0).abs() < 1e-9 && tri[0].y.abs() < 1e-9);
        assert_eq!(star_points(r, 5, 0.4).len(), 10);
    }
}
