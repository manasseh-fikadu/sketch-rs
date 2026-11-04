//! Hand-drawn rendering: a port of rough.js 4.6.4 (renderer, generator, hachure/zigzag fills,
//! points-on-curve) and of Excalidraw's element → rough options mapping.
//!
//! Ported from roughjs (bin/renderer.js, bin/generator.js, bin/fillers/*, Copyright (c) 2019
//! Preet Shihn, MIT), hachure-fill, points-on-curve and points-on-path (Copyright (c) 2020/2023
//! Preet Shihn, MIT), and Excalidraw (packages/element/src/shape.ts, Copyright (c) 2020
//! Excalidraw, MIT). See THIRD_PARTY.md. The random-number call order matches rough.js exactly,
//! so the same seed draws the same wobble (`tests/fixtures/rough/` holds rough.js reference
//! output that the tests compare against).

use super::draw::{DrawOp, PaintSpec, rgba};
use crate::geom::{Rect, V2, v2};
use crate::model::rough::{FillStyle, Rough};
use crate::model::{Cap, Color, Join, Node, NodeKind};
use crate::vector::{self, VPath};
use std::cell::Cell;
use std::rc::Rc;

// ---------------------------------------------------------------------------------- rng + options

/// rough.js `Random`: a Park–Miller style LCG on i32 (`Math.imul(48271, seed)`).
#[derive(Debug, Clone)]
struct Rng(Rc<Cell<i32>>);

impl Rng {
    fn new(seed: i32) -> Rng {
        Rng(Rc::new(Cell::new(seed)))
    }
    fn next(&self) -> f64 {
        let s = self.0.get().wrapping_mul(48271);
        self.0.set(s);
        (s & 0x7fff_ffff) as f64 / 2147483648.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Hachure,
    Solid,
    Zigzag,
    CrossHatch,
    Dots,
}

/// rough.js options. Cloning shares the random generator (like `Object.assign({}, o)` after
/// the randomizer exists); `alter_seed` makes a fresh one (`cloneOptionsAlterSeed`).
#[derive(Debug, Clone)]
pub struct Opts {
    pub max_randomness_offset: f64,
    pub roughness: f64,
    pub bowing: f64,
    pub stroke_width: f64,
    pub curve_tightness: f64,
    pub curve_fitting: f64,
    pub curve_step_count: f64,
    pub fill_style: Style,
    pub fill_weight: f64,
    pub hachure_angle: f64,
    pub hachure_gap: f64,
    pub seed: i32,
    pub disable_multi_stroke: bool,
    pub disable_multi_stroke_fill: bool,
    pub preserve_vertices: bool,
    pub fill_shape_roughness_gain: f64,
    pub fill: bool,
    pub stroke: bool,
    rng: Rng,
}

impl Opts {
    pub fn new(seed: i32) -> Opts {
        let seed = if seed == 0 { 1 } else { seed };
        Opts {
            max_randomness_offset: 2.0,
            roughness: 1.0,
            bowing: 1.0,
            stroke_width: 1.0,
            curve_tightness: 0.0,
            curve_fitting: 0.95,
            curve_step_count: 9.0,
            fill_style: Style::Hachure,
            fill_weight: -1.0,
            hachure_angle: -41.0,
            hachure_gap: -1.0,
            seed,
            disable_multi_stroke: false,
            disable_multi_stroke_fill: false,
            preserve_vertices: false,
            fill_shape_roughness_gain: 0.8,
            fill: false,
            stroke: true,
            rng: Rng::new(seed),
        }
    }

    /// Options for a new generator call: same settings, fresh generator from `seed`.
    fn fresh(&self) -> Opts {
        Opts {
            rng: Rng::new(self.seed),
            ..self.clone()
        }
    }

    fn alter_seed(&self) -> Opts {
        let seed = self.seed.wrapping_add(1);
        Opts {
            seed,
            rng: Rng::new(seed),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Move(V2),
    Bez(V2, V2, V2),
    Line(V2),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetKind {
    Path,
    FillPath,
    FillSketch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpSet {
    pub kind: SetKind,
    pub ops: Vec<Op>,
}

fn offset(min: f64, max: f64, o: &Opts, gain: f64) -> f64 {
    o.roughness * gain * (o.rng.next() * (max - min) + min)
}

fn offset_opt(x: f64, o: &Opts, gain: f64) -> f64 {
    offset(-x, x, o, gain)
}

fn rnd(x: f64, o: &Opts) -> f64 {
    offset_opt(x, o, 1.0)
}

// ---------------------------------------------------------------------------------- renderer

fn line_ops(a: V2, b: V2, o: &Opts, mv: bool, overlay: bool) -> Vec<Op> {
    let len_sq = (a.x - b.x).powi(2) + (a.y - b.y).powi(2);
    let len = len_sq.sqrt();
    let gain = if len < 200.0 {
        1.0
    } else if len > 500.0 {
        0.4
    } else {
        -0.0016668 * len + 1.233334
    };
    let mut off = o.max_randomness_offset;
    if off * off * 100.0 > len_sq {
        off = len / 10.0;
    }
    let half = off / 2.0;
    let diverge = 0.2 + o.rng.next() * 0.2;
    let mdx = offset_opt(
        o.bowing * o.max_randomness_offset * (b.y - a.y) / 200.0,
        o,
        gain,
    );
    let mdy = offset_opt(
        o.bowing * o.max_randomness_offset * (a.x - b.x) / 200.0,
        o,
        gain,
    );
    let r = if overlay { half } else { off };
    let rr = || offset_opt(r, o, gain);
    let pv = o.preserve_vertices;
    let mut ops = Vec::with_capacity(2);
    if mv {
        let x = a.x + if pv { 0.0 } else { rr() };
        let y = a.y + if pv { 0.0 } else { rr() };
        ops.push(Op::Move(v2(x, y)));
    }
    let c1x = mdx + a.x + (b.x - a.x) * diverge + rr();
    let c1y = mdy + a.y + (b.y - a.y) * diverge + rr();
    let c2x = mdx + a.x + 2.0 * (b.x - a.x) * diverge + rr();
    let c2y = mdy + a.y + 2.0 * (b.y - a.y) * diverge + rr();
    let ex = b.x + if pv { 0.0 } else { rr() };
    let ey = b.y + if pv { 0.0 } else { rr() };
    ops.push(Op::Bez(v2(c1x, c1y), v2(c2x, c2y), v2(ex, ey)));
    ops
}

fn double_line(a: V2, b: V2, o: &Opts, filling: bool) -> Vec<Op> {
    let single = if filling {
        o.disable_multi_stroke_fill
    } else {
        o.disable_multi_stroke
    };
    let mut ops = line_ops(a, b, o, true, false);
    if !single {
        ops.extend(line_ops(a, b, o, true, true));
    }
    ops
}

pub fn linear_path(pts: &[V2], close: bool, o: &Opts) -> OpSet {
    let mut ops = Vec::new();
    if pts.len() > 2 {
        for w in pts.windows(2) {
            ops.extend(double_line(w[0], w[1], o, false));
        }
        if close {
            ops.extend(double_line(*pts.last().unwrap(), pts[0], o, false));
        }
    } else if pts.len() == 2 {
        ops = double_line(pts[0], pts[1], o, false);
    }
    OpSet {
        kind: SetKind::Path,
        ops,
    }
}

fn curve_ops(points: &[V2], o: &Opts) -> Vec<Op> {
    let len = points.len();
    let mut ops = Vec::new();
    if len > 3 {
        let s = 1.0 - o.curve_tightness;
        ops.push(Op::Move(points[1]));
        let mut i = 1;
        while i + 2 < len {
            let p = points[i];
            let b1 = v2(
                p.x + (s * points[i + 1].x - s * points[i - 1].x) / 6.0,
                p.y + (s * points[i + 1].y - s * points[i - 1].y) / 6.0,
            );
            let b2 = v2(
                points[i + 1].x + (s * points[i].x - s * points[i + 2].x) / 6.0,
                points[i + 1].y + (s * points[i].y - s * points[i + 2].y) / 6.0,
            );
            ops.push(Op::Bez(b1, b2, points[i + 1]));
            i += 1;
        }
    } else if len == 3 {
        ops.push(Op::Move(points[1]));
        ops.push(Op::Bez(points[1], points[2], points[2]));
    } else if len == 2 {
        ops.extend(double_line(points[0], points[1], o, false));
    }
    ops
}

fn curve_with_offset(points: &[V2], off: f64, o: &Opts) -> Vec<Op> {
    let j = |p: V2| {
        let x = p.x + rnd(off, o);
        let y = p.y + rnd(off, o);
        v2(x, y)
    };
    let mut ps = vec![j(points[0]), j(points[0])];
    for i in 1..points.len() {
        ps.push(j(points[i]));
        if i == points.len() - 1 {
            ps.push(j(points[i]));
        }
    }
    curve_ops(&ps, o)
}

pub fn curve(points: &[V2], o: &Opts) -> OpSet {
    let mut ops = curve_with_offset(points, 1.0 * (1.0 + o.roughness * 0.2), o);
    if !o.disable_multi_stroke {
        let o2 = o.alter_seed();
        ops.extend(curve_with_offset(
            points,
            1.5 * (1.0 + o.roughness * 0.22),
            &o2,
        ));
    }
    OpSet {
        kind: SetKind::Path,
        ops,
    }
}

struct EllipseParams {
    increment: f64,
    rx: f64,
    ry: f64,
}

fn ellipse_params(w: f64, h: f64, o: &Opts) -> EllipseParams {
    use std::f64::consts::PI;
    let psq = (PI * 2.0 * (((w / 2.0).powi(2) + (h / 2.0).powi(2)) / 2.0).sqrt()).sqrt();
    let steps = o
        .curve_step_count
        .max(o.curve_step_count / 200f64.sqrt() * psq)
        .ceil();
    let increment = PI * 2.0 / steps;
    let mut rx = (w / 2.0).abs();
    let mut ry = (h / 2.0).abs();
    let fit = 1.0 - o.curve_fitting;
    rx += rnd(rx * fit, o);
    ry += rnd(ry * fit, o);
    EllipseParams { increment, rx, ry }
}

fn ellipse_points(
    inc: f64,
    c: V2,
    rx: f64,
    ry: f64,
    off: f64,
    overlap: f64,
    o: &Opts,
) -> (Vec<V2>, Vec<V2>) {
    use std::f64::consts::PI;
    let mut core = Vec::new();
    let mut all = Vec::new();
    if o.roughness == 0.0 {
        let inc = inc / 4.0;
        all.push(v2(c.x + rx * (-inc).cos(), c.y + ry * (-inc).sin()));
        let mut a = 0.0;
        while a <= PI * 2.0 {
            let p = v2(c.x + rx * a.cos(), c.y + ry * a.sin());
            core.push(p);
            all.push(p);
            a += inc;
        }
        all.push(v2(c.x + rx, c.y));
        all.push(v2(c.x + rx * inc.cos(), c.y + ry * inc.sin()));
    } else {
        let rad = rnd(0.5, o) - PI / 2.0;
        let jx = rnd(off, o);
        let jy = rnd(off, o);
        all.push(v2(
            jx + c.x + 0.9 * rx * (rad - inc).cos(),
            jy + c.y + 0.9 * ry * (rad - inc).sin(),
        ));
        let end = PI * 2.0 + rad - 0.01;
        let mut a = rad;
        while a < end {
            let jx = rnd(off, o);
            let jy = rnd(off, o);
            let p = v2(jx + c.x + rx * a.cos(), jy + c.y + ry * a.sin());
            core.push(p);
            all.push(p);
            a += inc;
        }
        for (k, ang) in [
            (1.0, rad + PI * 2.0 + overlap * 0.5),
            (0.98, rad + overlap),
            (0.9, rad + overlap * 0.5),
        ] {
            let jx = rnd(off, o);
            let jy = rnd(off, o);
            all.push(v2(
                jx + c.x + k * rx * ang.cos(),
                jy + c.y + k * ry * ang.sin(),
            ));
        }
    }
    (all, core)
}

fn ellipse_with_params(c: V2, o: &Opts, p: &EllipseParams) -> (Vec<V2>, OpSet) {
    let inner = offset(0.4, 1.0, o, 1.0);
    let overlap = p.increment * offset(0.1, inner, o, 1.0);
    let (ap1, cp1) = ellipse_points(p.increment, c, p.rx, p.ry, 1.0, overlap, o);
    let mut ops = curve_ops(&ap1, o);
    if !o.disable_multi_stroke && o.roughness != 0.0 {
        let (ap2, _) = ellipse_points(p.increment, c, p.rx, p.ry, 1.5, 0.0, o);
        ops.extend(curve_ops(&ap2, o));
    }
    (
        cp1,
        OpSet {
            kind: SetKind::Path,
            ops,
        },
    )
}

fn bezier_to(c1: V2, c2: V2, p: V2, cur: V2, o: &Opts) -> Vec<Op> {
    let mro = if o.max_randomness_offset != 0.0 {
        o.max_randomness_offset
    } else {
        1.0
    };
    let ros = [mro, mro + 0.3];
    let iterations = if o.disable_multi_stroke { 1 } else { 2 };
    let pv = o.preserve_vertices;
    let mut ops = Vec::new();
    for (i, r) in ros.iter().take(iterations).enumerate() {
        if i == 0 {
            ops.push(Op::Move(cur));
        } else {
            let x = cur.x + if pv { 0.0 } else { rnd(ros[0], o) };
            let y = cur.y + if pv { 0.0 } else { rnd(ros[0], o) };
            ops.push(Op::Move(v2(x, y)));
        }
        let f = if pv {
            p
        } else {
            let x = p.x + rnd(*r, o);
            let y = p.y + rnd(*r, o);
            v2(x, y)
        };
        let a = v2(c1.x + rnd(*r, o), c1.y + rnd(*r, o));
        let a = v2(a.x, a.y);
        let bx = c2.x + rnd(*r, o);
        let by = c2.y + rnd(*r, o);
        ops.push(Op::Bez(a, v2(bx, by), f));
    }
    ops
}

/// Absolute, normalized path segments (M/L/C/Z), as path-data-parser produces them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    M(V2),
    L(V2),
    C(V2, V2, V2),
    Z,
}

/// Convert one of our paths into normalized segments (quadratics elevated to cubics).
pub fn segs_of(p: &VPath) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut cur = V2::ZERO;
    let mut start = V2::ZERO;
    for s in &p.segs {
        match *s {
            vector::Seg::M(q) => {
                out.push(Seg::M(q));
                cur = q;
                start = q;
            }
            vector::Seg::L(q) => {
                out.push(Seg::L(q));
                cur = q;
            }
            vector::Seg::Q(c, q) => {
                let c1 = cur + (c - cur) * (2.0 / 3.0);
                let c2 = q + (c - q) * (2.0 / 3.0);
                out.push(Seg::C(c1, c2, q));
                cur = q;
            }
            vector::Seg::C(a, b, q) => {
                out.push(Seg::C(a, b, q));
                cur = q;
            }
            vector::Seg::Z => {
                out.push(Seg::Z);
                cur = start;
            }
        }
    }
    out
}

pub fn svg_path(segs: &[Seg], o: &Opts) -> OpSet {
    let mut ops = Vec::new();
    let mut first = V2::ZERO;
    let mut cur = V2::ZERO;
    for s in segs {
        match *s {
            Seg::M(p) => {
                cur = p;
                first = p;
            }
            Seg::L(p) => {
                ops.extend(double_line(cur, p, o, false));
                cur = p;
            }
            Seg::C(a, b, p) => {
                ops.extend(bezier_to(a, b, p, cur, o));
                cur = p;
            }
            Seg::Z => {
                ops.extend(double_line(cur, first, o, false));
                cur = first;
            }
        }
    }
    OpSet {
        kind: SetKind::Path,
        ops,
    }
}

pub fn solid_fill_polygon(polys: &[Vec<V2>], o: &Opts) -> OpSet {
    let mut ops = Vec::new();
    for pts in polys {
        if pts.len() > 2 {
            let off = o.max_randomness_offset;
            let x = pts[0].x + rnd(off, o);
            let y = pts[0].y + rnd(off, o);
            ops.push(Op::Move(v2(x, y)));
            for p in &pts[1..] {
                let x = p.x + rnd(off, o);
                let y = p.y + rnd(off, o);
                ops.push(Op::Line(v2(x, y)));
            }
        }
    }
    OpSet {
        kind: SetKind::FillPath,
        ops,
    }
}

// ---------------------------------------------------------------------------------- fills

/// JS `Math.round` (halves round up).
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

fn rotate(p: V2, deg: f64) -> V2 {
    let a = std::f64::consts::PI / 180.0 * deg;
    let (s, c) = a.sin_cos();
    v2(p.x * c - p.y * s, p.x * s + p.y * c)
}

/// hachure-fill `hachureLines`: scan-line fill of rotated polygons.
fn hachure_lines(polys: &[Vec<V2>], gap: f64, angle: f64, step: f64) -> Vec<(V2, V2)> {
    let gap = gap.max(0.1);
    let rotated: Vec<Vec<V2>> = polys
        .iter()
        .map(|p| {
            p.iter()
                .map(|q| if angle != 0.0 { rotate(*q, angle) } else { *q })
                .collect()
        })
        .collect();
    let mut lines = straight_hachure(&rotated, gap, step);
    if angle != 0.0 {
        for l in &mut lines {
            l.0 = rotate(l.0, -angle);
            l.1 = rotate(l.1, -angle);
        }
    }
    lines
}

struct Edge {
    ymin: f64,
    ymax: f64,
    x: f64,
    islope: f64,
}

fn straight_hachure(polys: &[Vec<V2>], gap: f64, step: f64) -> Vec<(V2, V2)> {
    let mut verts: Vec<Vec<V2>> = Vec::new();
    for p in polys {
        if p.is_empty() {
            continue;
        }
        let mut v = p.clone();
        if v[0] != *v.last().unwrap() {
            v.push(v[0]);
        }
        if v.len() > 2 {
            verts.push(v);
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    for v in &verts {
        for w in v.windows(2) {
            let (p1, p2) = (w[0], w[1]);
            if p1.y != p2.y {
                let ymin = p1.y.min(p2.y);
                edges.push(Edge {
                    ymin,
                    ymax: p1.y.max(p2.y),
                    x: if ymin == p1.y { p1.x } else { p2.x },
                    islope: (p2.x - p1.x) / (p2.y - p1.y),
                });
            }
        }
    }
    edges.sort_by(|a, b| {
        a.ymin
            .partial_cmp(&b.ymin)
            .unwrap()
            .then(a.x.partial_cmp(&b.x).unwrap())
            .then(a.ymax.partial_cmp(&b.ymax).unwrap())
    });
    let mut lines = Vec::new();
    if edges.is_empty() {
        return lines;
    }
    let mut pending: std::collections::VecDeque<Edge> = edges.into();
    let mut active: Vec<Edge> = Vec::new();
    let mut y = pending[0].ymin;
    let mut iteration = 0.0f64;
    while !active.is_empty() || !pending.is_empty() {
        while pending.front().is_some_and(|e| e.ymin <= y) {
            active.push(pending.pop_front().unwrap());
        }
        active.retain(|e| e.ymax > y);
        // Stable sort by x (ties keep their order, like the JS comparator returning 0).
        active.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        if (step != 1.0 || iteration % gap == 0.0) && active.len() > 1 {
            let mut i = 0;
            while i + 1 < active.len() {
                let (ce, ne) = (&active[i], &active[i + 1]);
                lines.push((v2(js_round(ce.x), y), v2(js_round(ne.x), y)));
                i += 2;
            }
        }
        y += step;
        for e in &mut active {
            e.x += step * e.islope;
        }
        iteration += 1.0;
    }
    lines
}

fn polygon_hachure_lines(polys: &[Vec<V2>], o: &Opts) -> Vec<(V2, V2)> {
    let angle = o.hachure_angle + 90.0;
    let mut gap = if o.hachure_gap < 0.0 {
        o.stroke_width * 4.0
    } else {
        o.hachure_gap
    };
    gap = gap.max(0.1);
    let mut skip = 1.0;
    if o.roughness >= 1.0 && o.rng.next() > 0.7 {
        skip = gap;
    }
    hachure_lines(polys, gap, angle, skip)
}

fn render_lines(lines: &[(V2, V2)], o: &Opts) -> Vec<Op> {
    let mut ops = Vec::new();
    for (a, b) in lines {
        ops.extend(double_line(*a, *b, o, true));
    }
    ops
}

pub fn pattern_fill(polys: &[Vec<V2>], o: &Opts) -> OpSet {
    let ops = match o.fill_style {
        Style::CrossHatch => {
            let mut ops = render_lines(&polygon_hachure_lines(polys, o), o);
            let o2 = Opts {
                hachure_angle: o.hachure_angle + 90.0,
                ..o.clone()
            };
            ops.extend(render_lines(&polygon_hachure_lines(polys, &o2), &o2));
            ops
        }
        Style::Zigzag => {
            let gap = if o.hachure_gap < 0.0 {
                o.stroke_width * 4.0
            } else {
                o.hachure_gap
            }
            .max(0.1);
            let o2 = Opts {
                hachure_gap: gap,
                ..o.clone()
            };
            let lines = polygon_hachure_lines(polys, &o2);
            let a = o.hachure_angle.to_radians();
            let (dgx, dgy) = (gap * 0.5 * a.cos(), gap * 0.5 * a.sin());
            let mut zz = Vec::new();
            for (p1, p2) in lines {
                if p1.dist(p2) > 0.0 {
                    zz.push((v2(p1.x - dgx, p1.y + dgy), p2));
                    zz.push((v2(p1.x + dgx, p1.y - dgy), p2));
                }
            }
            render_lines(&zz, o)
        }
        Style::Dots => {
            // rough.js jitters dots with Math.random; we use the seeded generator instead.
            let o2 = Opts {
                hachure_angle: 0.0,
                ..o.clone()
            };
            let lines = polygon_hachure_lines(polys, &o2);
            let gap = if o.hachure_gap < 0.0 {
                o.stroke_width * 4.0
            } else {
                o.hachure_gap
            }
            .max(0.1);
            let fw = if o.fill_weight < 0.0 {
                o.stroke_width / 2.0
            } else {
                o.fill_weight
            };
            let ro = gap / 4.0;
            let mut ops = Vec::new();
            for (a, b) in lines {
                let length = a.dist(b);
                let count = (length / gap).ceil() as i64 - 1;
                let off = length - count as f64 * gap;
                let x = (a.x + b.x) / 2.0 - gap / 4.0;
                let min_y = a.y.min(b.y);
                for i in 0..count.max(0) {
                    let y = min_y + off + i as f64 * gap;
                    let cx = x - ro + o.rng.next() * 2.0 * ro;
                    let cy = y - ro + o.rng.next() * 2.0 * ro;
                    let p = ellipse_params(fw, fw, o);
                    ops.extend(ellipse_with_params(v2(cx, cy), o, &p).1.ops);
                }
            }
            ops
        }
        _ => render_lines(&polygon_hachure_lines(polys, o), o),
    };
    OpSet {
        kind: SetKind::FillSketch,
        ops,
    }
}

// ---------------------------------------------------------------------------------- points-on-curve

fn flatness(p: &[V2]) -> f64 {
    let mut ux = 3.0 * p[1].x - 2.0 * p[0].x - p[3].x;
    ux *= ux;
    let mut uy = 3.0 * p[1].y - 2.0 * p[0].y - p[3].y;
    uy *= uy;
    let mut vx = 3.0 * p[2].x - 2.0 * p[3].x - p[0].x;
    vx *= vx;
    let mut vy = 3.0 * p[2].y - 2.0 * p[3].y - p[0].y;
    vy *= vy;
    ux.max(vx) + uy.max(vy)
}

fn split_bezier(p: [V2; 4], tol: f64, out: &mut Vec<V2>) {
    if flatness(&p) < tol {
        if let Some(l) = out.last() {
            if l.dist(p[0]) > 1.0 {
                out.push(p[0]);
            }
        } else {
            out.push(p[0]);
        }
        out.push(p[3]);
    } else {
        let q1 = p[0].lerp(p[1], 0.5);
        let q2 = p[1].lerp(p[2], 0.5);
        let q3 = p[2].lerp(p[3], 0.5);
        let r1 = q1.lerp(q2, 0.5);
        let r2 = q2.lerp(q3, 0.5);
        let red = r1.lerp(r2, 0.5);
        split_bezier([p[0], q1, r1, red], tol, out);
        split_bezier([red, r2, q3, p[3]], tol, out);
    }
}

fn seg_dist_sq(p: V2, v: V2, w: V2) -> f64 {
    let l2 = (v.x - w.x).powi(2) + (v.y - w.y).powi(2);
    if l2 == 0.0 {
        return (p.x - v.x).powi(2) + (p.y - v.y).powi(2);
    }
    let t = (((p.x - v.x) * (w.x - v.x) + (p.y - v.y) * (w.y - v.y)) / l2).clamp(0.0, 1.0);
    let q = v.lerp(w, t);
    (p.x - q.x).powi(2) + (p.y - q.y).powi(2)
}

fn rdp(points: &[V2], start: usize, end: usize, eps: f64, out: &mut Vec<V2>) {
    let s = points[start];
    let e = points[end - 1];
    let mut max_d = 0.0;
    let mut max_i = 1;
    for (i, p) in points.iter().enumerate().take(end - 1).skip(start + 1) {
        let d = seg_dist_sq(*p, s, e);
        if d > max_d {
            max_d = d;
            max_i = i;
        }
    }
    if max_d.sqrt() > eps {
        rdp(points, start, max_i + 1, eps, out);
        rdp(points, max_i, end, eps, out);
    } else {
        if out.is_empty() {
            out.push(s);
        }
        out.push(e);
    }
}

fn simplify(points: &[V2], d: f64) -> Vec<V2> {
    let mut out = Vec::new();
    if !points.is_empty() {
        rdp(points, 0, points.len(), d, &mut out);
    }
    out
}

fn points_on_bezier_curves(pts: &[V2], tol: f64, dist: Option<f64>) -> Vec<V2> {
    let mut out = Vec::new();
    let n = (pts.len().saturating_sub(1)) / 3;
    for i in 0..n {
        let o = i * 3;
        split_bezier([pts[o], pts[o + 1], pts[o + 2], pts[o + 3]], tol, &mut out);
    }
    match dist {
        Some(d) if d > 0.0 && !out.is_empty() => simplify(&out, d),
        _ => out,
    }
}

#[allow(clippy::needless_range_loop)]
fn curve_to_bezier(pin: &[V2], tightness: f64) -> Vec<V2> {
    let len = pin.len();
    if len == 3 {
        return vec![pin[0], pin[1], pin[2], pin[2]];
    }
    let mut pts = vec![pin[0], pin[0]];
    for i in 1..len {
        pts.push(pin[i]);
        if i == len - 1 {
            pts.push(pin[i]);
        }
    }
    let s = 1.0 - tightness;
    let mut out = vec![pts[0]];
    let mut i = 1;
    while i + 2 < pts.len() {
        let p = pts[i];
        out.push(v2(
            p.x + (s * pts[i + 1].x - s * pts[i - 1].x) / 6.0,
            p.y + (s * pts[i + 1].y - s * pts[i - 1].y) / 6.0,
        ));
        out.push(v2(
            pts[i + 1].x + (s * pts[i].x - s * pts[i + 2].x) / 6.0,
            pts[i + 1].y + (s * pts[i].y - s * pts[i + 2].y) / 6.0,
        ));
        out.push(pts[i + 1]);
        i += 1;
    }
    out
}

fn points_on_path(segs: &[Seg], tol: f64, dist: f64) -> Vec<Vec<V2>> {
    let mut sets: Vec<Vec<V2>> = Vec::new();
    let mut cur: Vec<V2> = Vec::new();
    let mut start = V2::ZERO;
    let mut pending: Vec<V2> = Vec::new();
    let flush_curve = |cur: &mut Vec<V2>, pending: &mut Vec<V2>| {
        if pending.len() >= 4 {
            cur.extend(points_on_bezier_curves(pending, tol, None));
        }
        pending.clear();
    };
    for s in segs {
        match *s {
            Seg::M(p) => {
                flush_curve(&mut cur, &mut pending);
                if !cur.is_empty() {
                    sets.push(std::mem::take(&mut cur));
                }
                start = p;
                cur.push(p);
            }
            Seg::L(p) => {
                flush_curve(&mut cur, &mut pending);
                cur.push(p);
            }
            Seg::C(a, b, p) => {
                if pending.is_empty() {
                    pending.push(*cur.last().unwrap_or(&start));
                }
                pending.extend([a, b, p]);
            }
            Seg::Z => {
                flush_curve(&mut cur, &mut pending);
                cur.push(start);
            }
        }
    }
    flush_curve(&mut cur, &mut pending);
    if !cur.is_empty() {
        sets.push(cur);
    }
    sets.into_iter()
        .map(|s| simplify(&s, dist))
        .filter(|s| !s.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------------- generator

/// Drop every `move` but the first, giving one closed fill outline.
fn merged_shape(ops: Vec<Op>) -> Vec<Op> {
    ops.into_iter()
        .enumerate()
        .filter(|(i, op)| *i == 0 || !matches!(op, Op::Move(_)))
        .map(|(_, op)| op)
        .collect()
}

pub fn g_line(a: V2, b: V2, o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    vec![OpSet {
        kind: SetKind::Path,
        ops: double_line(a, b, &o, false),
    }]
}

pub fn g_rectangle(r: Rect, o: &Opts) -> Vec<OpSet> {
    let pts = vec![r.min(), v2(r.right(), r.y), r.max(), v2(r.x, r.bottom())];
    g_polygon(&pts, o)
}

pub fn g_polygon(pts: &[V2], o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    let outline = linear_path(pts, true, &o);
    let mut sets = Vec::new();
    if o.fill {
        sets.push(if o.fill_style == Style::Solid {
            solid_fill_polygon(&[pts.to_vec()], &o)
        } else {
            pattern_fill(&[pts.to_vec()], &o)
        });
    }
    if o.stroke {
        sets.push(outline);
    }
    sets
}

pub fn g_linear_path(pts: &[V2], o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    vec![linear_path(pts, false, &o)]
}

pub fn g_ellipse(c: V2, w: f64, h: f64, o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    let params = ellipse_params(w, h, &o);
    let (est, outline) = ellipse_with_params(c, &o, &params);
    let mut sets = Vec::new();
    if o.fill {
        if o.fill_style == Style::Solid {
            let (_, mut shape) = ellipse_with_params(c, &o, &params);
            shape.kind = SetKind::FillPath;
            sets.push(shape);
        } else {
            sets.push(pattern_fill(&[est], &o));
        }
    }
    if o.stroke {
        sets.push(outline);
    }
    sets
}

pub fn g_curve(pts: &[V2], o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    let outline = curve(pts, &o);
    let mut sets = Vec::new();
    if o.fill && pts.len() >= 3 {
        if o.fill_style == Style::Solid {
            let fo = Opts {
                disable_multi_stroke: true,
                roughness: if o.roughness != 0.0 {
                    o.roughness + o.fill_shape_roughness_gain
                } else {
                    0.0
                },
                ..o.clone()
            };
            let shape = curve(pts, &fo);
            sets.push(OpSet {
                kind: SetKind::FillPath,
                ops: merged_shape(shape.ops),
            });
        } else {
            let poly = points_on_bezier_curves(
                &curve_to_bezier(pts, 0.0),
                10.0,
                Some((1.0 + o.roughness) / 2.0),
            );
            sets.push(pattern_fill(&[poly], &o));
        }
    }
    if o.stroke {
        sets.push(outline);
    }
    sets
}

pub fn g_path(segs: &[Seg], o: &Opts) -> Vec<OpSet> {
    let o = o.fresh();
    let mut sets = Vec::new();
    if segs.is_empty() {
        return sets;
    }
    let point_sets = points_on_path(segs, 1.0, (1.0 + o.roughness) / 2.0);
    let shape = svg_path(segs, &o);
    if o.fill {
        if o.fill_style == Style::Solid {
            if point_sets.len() == 1 {
                let fo = Opts {
                    disable_multi_stroke: true,
                    roughness: if o.roughness != 0.0 {
                        o.roughness + o.fill_shape_roughness_gain
                    } else {
                        0.0
                    },
                    ..o.clone()
                };
                let fs = svg_path(segs, &fo);
                sets.push(OpSet {
                    kind: SetKind::FillPath,
                    ops: merged_shape(fs.ops),
                });
            } else {
                sets.push(solid_fill_polygon(&point_sets, &o));
            }
        } else {
            sets.push(pattern_fill(&point_sets, &o));
        }
    }
    if o.stroke {
        sets.push(shape);
    }
    sets
}

// ---------------------------------------------------------------------------------- Excalidraw mapping

/// Excalidraw's `adjustRoughness`: small shapes get less wobble.
pub fn adjust_roughness(r: f64, w: f64, h: f64, rounded: bool, linear: bool) -> f64 {
    let (mx, mn) = (w.max(h), w.min(h));
    if (mn >= 20.0 && mx >= 50.0) || (mn >= 15.0 && rounded) || (linear && mx >= 50.0) {
        return r;
    }
    (r / if mx < 10.0 { 3.0 } else { 2.0 }).min(2.5)
}

fn ops_to_path(ops: &[Op]) -> VPath {
    let mut p = VPath::new();
    for op in ops {
        match *op {
            Op::Move(q) => {
                p.move_to(q);
            }
            Op::Bez(a, b, q) => {
                p.cubic_to(a, b, q);
            }
            Op::Line(q) => {
                p.line_to(q);
            }
        }
    }
    p
}

/// Hand-drawn draw ops for a node at world rect `wr`.
pub fn node_ops(n: &Node, wr: Rect, opacity: f64, r: &Rough) -> Vec<DrawOp> {
    let sw = n.style.stroke_width.max(0.0);
    let fill_color: Option<Color> = n.style.fills.last().map(|p| p.preview_color());
    let stroke = n.style.stroke;
    let dashed = !n.style.dash.is_empty();
    let dotted = dashed && n.style.dash[0] <= 2.0;
    let rounded = n.style.corner_radius > 0.0;
    let linear = matches!(n.kind, NodeKind::Line { .. })
        || matches!(&n.kind, NodeKind::Path { subpaths, .. } if subpaths.iter().all(|s| !s.closed));
    let mut o = Opts::new(r.seed as i32);
    o.roughness = adjust_roughness(r.roughness, wr.w, wr.h, rounded, linear);
    o.bowing = r.bowing;
    o.stroke_width = if dashed { sw + 0.5 } else { sw };
    o.disable_multi_stroke = dashed;
    o.fill_weight = sw / 2.0;
    o.hachure_gap = r.hachure_gap.unwrap_or(sw * 4.0);
    o.hachure_angle = r.hachure_angle;
    o.preserve_vertices = r.roughness < 2.0;
    o.fill_style = match r.fill_style {
        FillStyle::Hachure => Style::Hachure,
        FillStyle::CrossHatch => Style::CrossHatch,
        FillStyle::Solid => Style::Solid,
        FillStyle::Zigzag => Style::Zigzag,
        FillStyle::Dots => Style::Dots,
    };
    o.fill = fill_color.is_some() && super::draw::is_area(n);
    o.stroke = stroke.is_some() && sw > 0.0;
    let sets: Vec<OpSet> = match &n.kind {
        NodeKind::Rect | NodeKind::Frame { .. } if rounded => {
            let rad = n.style.corner_radius.min(wr.w.min(wr.h) * 0.5);
            let (x0, y0, x1, y1) = (wr.x, wr.y, wr.right(), wr.bottom());
            // Excalidraw's rounded rectangle path (quadratic corners).
            let mut p = VPath::new();
            p.move_to(v2(x0 + rad, y0))
                .line_to(v2(x1 - rad, y0))
                .quad_to(v2(x1, y0), v2(x1, y0 + rad))
                .line_to(v2(x1, y1 - rad))
                .quad_to(v2(x1, y1), v2(x1 - rad, y1))
                .line_to(v2(x0 + rad, y1))
                .quad_to(v2(x0, y1), v2(x0, y1 - rad))
                .line_to(v2(x0, y0 + rad))
                .quad_to(v2(x0, y0), v2(x0 + rad, y0));
            o.preserve_vertices = true;
            g_path(&segs_of(&p), &o)
        }
        NodeKind::Rect | NodeKind::Frame { .. } => g_rectangle(wr, &o),
        NodeKind::Ellipse => {
            o.curve_fitting = 1.0;
            g_ellipse(wr.center(), wr.w, wr.h, &o)
        }
        NodeKind::Polygon { sides } => g_polygon(&vector::polygon_points(wr, *sides), &o),
        NodeKind::Star { points, inner } => {
            g_polygon(&vector::star_points(wr, *points, *inner), &o)
        }
        NodeKind::Line { a, b } => g_linear_path(&[wr.min() + *a, wr.min() + *b], &o),
        NodeKind::Path { subpaths, .. } => {
            let path = vector::from_subpaths(wr.min(), subpaths);
            let simple = subpaths.len() == 1
                && subpaths[0]
                    .points
                    .iter()
                    .all(|p| p.h_in.is_none() && p.h_out.is_none());
            if simple && !subpaths[0].closed {
                let pts: Vec<V2> = subpaths[0].points.iter().map(|p| wr.min() + p.p).collect();
                g_linear_path(&pts, &o)
            } else if simple {
                let pts: Vec<V2> = subpaths[0].points.iter().map(|p| wr.min() + p.p).collect();
                g_polygon(&pts, &o)
            } else {
                g_path(&segs_of(&path), &o)
            }
        }
        NodeKind::Connector { points, route, .. } => {
            let pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
            o.fill = false;
            match route {
                crate::model::Route::Curved if pts.len() >= 3 => g_curve(&pts, &o),
                crate::model::Route::Elbow => {
                    o.preserve_vertices = true;
                    g_path(&segs_of(&crate::connector::line_path(&pts, *route)), &o)
                }
                _ => g_linear_path(&pts, &o),
            }
        }
        // Text, ink, groups, sticky notes: no hand-drawn form.
        _ => {
            let mut plain = n.clone();
            plain.style.rough = None;
            return super::draw::node_draw(&plain, wr, opacity).ops;
        }
    };
    let even_odd = !matches!(n.kind, NodeKind::Ellipse);
    let dash = if dotted {
        vec![1.5, 6.0 + sw]
    } else if dashed {
        vec![8.0, 8.0 + sw]
    } else {
        Vec::new()
    };
    let mut out = Vec::new();
    if matches!(n.kind, NodeKind::Connector { .. }) {
        // Heads stay crisp; the line wobbles.
        super::draw::connector_ops(n, wr, opacity, &mut out, true);
    }
    for s in sets {
        let path = ops_to_path(&s.ops);
        match s.kind {
            SetKind::Path => {
                if let Some(c) = stroke {
                    out.push(DrawOp::Stroke {
                        path,
                        color: rgba(c, opacity),
                        width: o.stroke_width,
                        cap: Cap::Round,
                        join: Join::Round,
                        dash: dash.clone(),
                    });
                }
            }
            SetKind::FillPath => {
                if let Some(c) = fill_color {
                    out.push(DrawOp::Fill {
                        path,
                        paint: PaintSpec::Solid(rgba(c, opacity)),
                        even_odd,
                    });
                }
            }
            SetKind::FillSketch => {
                if let Some(c) = fill_color {
                    let w = if o.fill_weight < 0.0 {
                        sw / 2.0
                    } else {
                        o.fill_weight
                    };
                    out.push(DrawOp::Stroke {
                        path,
                        color: rgba(c, opacity),
                        width: w.max(0.5),
                        cap: Cap::Round,
                        join: Join::Round,
                        dash: Vec::new(),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> serde_json::Value {
        let s = include_str!("../../tests/fixtures/rough/roughjs-4.6.4-reference.json");
        serde_json::from_str(s).unwrap()
    }

    fn compare(name: &str, sets: &[OpSet]) {
        let r = reference();
        let want = r[name].as_array().unwrap();
        assert_eq!(want.len(), sets.len(), "{name}: set count");
        for (w, s) in want.iter().zip(sets) {
            let kind = match s.kind {
                SetKind::Path => "path",
                SetKind::FillPath => "fillPath",
                SetKind::FillSketch => "fillSketch",
            };
            assert_eq!(w["type"], kind, "{name}");
            let ops = w["ops"].as_array().unwrap();
            assert_eq!(ops.len(), s.ops.len(), "{name}: op count ({kind})");
            for (i, (wo, so)) in ops.iter().zip(&s.ops).enumerate() {
                let wo = wo.as_array().unwrap();
                let nums: Vec<f64> = wo[1..].iter().map(|v| v.as_f64().unwrap()).collect();
                let (op, got): (&str, Vec<f64>) = match so {
                    Op::Move(p) => ("move", vec![p.x, p.y]),
                    Op::Line(p) => ("lineTo", vec![p.x, p.y]),
                    Op::Bez(a, b, p) => ("bcurveTo", vec![a.x, a.y, b.x, b.y, p.x, p.y]),
                };
                assert_eq!(wo[0], op, "{name} op {i}");
                for (a, b) in nums.iter().zip(&got) {
                    assert!((a - b).abs() < 1e-6, "{name} op {i}: {nums:?} vs {got:?}");
                }
            }
        }
    }

    fn base(seed: i32) -> Opts {
        let mut o = Opts::new(seed);
        o.stroke_width = 2.0;
        o
    }

    #[test]
    fn rng_matches_rough_js() {
        let r = Rng::new(7);
        // Math.imul(48271, 7) = 337897 → 337897 / 2^31
        assert!((r.next() - 337897.0 / 2147483648.0).abs() < 1e-15);
    }

    #[test]
    fn matches_rough_js_reference_output() {
        let mut o = base(7);
        o.fill = true;
        o.hachure_gap = 8.0;
        o.fill_weight = 1.0;
        o.preserve_vertices = true;
        compare(
            "rect_hachure",
            &g_rectangle(Rect::new(10.0, 20.0, 120.0, 80.0), &o),
        );

        let mut o = base(3);
        o.fill = true;
        o.fill_style = Style::Solid;
        compare(
            "rect_solid",
            &g_rectangle(Rect::new(0.0, 0.0, 50.0, 40.0), &o),
        );

        let mut o = base(42);
        o.fill = true;
        o.fill_style = Style::CrossHatch;
        o.hachure_gap = 8.0;
        o.fill_weight = 1.0;
        o.curve_fitting = 1.0;
        compare("ellipse_cross", &g_ellipse(v2(60.0, 40.0), 120.0, 80.0, &o));

        let mut o = base(5);
        o.fill = true;
        o.fill_style = Style::Solid;
        o.curve_fitting = 1.0;
        o.roughness = 2.0;
        compare("ellipse_solid", &g_ellipse(v2(30.0, 30.0), 60.0, 60.0, &o));

        let mut o = base(11);
        o.fill = true;
        o.fill_style = Style::Zigzag;
        o.hachure_gap = 8.0;
        o.fill_weight = 1.0;
        let diamond = [
            v2(50.0, 0.0),
            v2(100.0, 50.0),
            v2(50.0, 100.0),
            v2(0.0, 50.0),
        ];
        compare("polygon_zigzag", &g_polygon(&diamond, &o));

        let o = base(9);
        let pts = [
            v2(0.0, 0.0),
            v2(40.0, 30.0),
            v2(80.0, -10.0),
            v2(120.0, 40.0),
        ];
        compare("curve", &g_curve(&pts, &o));

        let mut o = base(13);
        o.roughness = 2.0;
        compare("line", &g_line(v2(0.0, 0.0), v2(300.0, 120.0), &o));

        let mut o = base(21);
        o.disable_multi_stroke = true;
        o.stroke_width = 2.5;
        let pts = [v2(0.0, 0.0), v2(60.0, 10.0), v2(90.0, 70.0)];
        compare("linear_dashed", &g_linear_path(&pts, &o));

        let mut o = base(17);
        o.fill = true;
        o.hachure_gap = 8.0;
        o.fill_weight = 1.0;
        o.preserve_vertices = true;
        let mut p = VPath::new();
        p.move_to(v2(10.0, 0.0))
            .line_to(v2(90.0, 0.0))
            .quad_to(v2(100.0, 0.0), v2(100.0, 10.0))
            .line_to(v2(100.0, 50.0))
            .quad_to(v2(100.0, 60.0), v2(90.0, 60.0))
            .line_to(v2(10.0, 60.0))
            .quad_to(v2(0.0, 60.0), v2(0.0, 50.0))
            .line_to(v2(0.0, 10.0))
            .quad_to(v2(0.0, 0.0), v2(10.0, 0.0));
        compare("path_rounded", &g_path(&segs_of(&p), &o));

        let mut o = base(19);
        o.fill = true;
        o.fill_style = Style::Solid;
        let mut p = VPath::new();
        p.move_to(v2(0.0, 0.0))
            .line_to(v2(80.0, 0.0))
            .cubic_to(v2(100.0, 0.0), v2(100.0, 40.0), v2(80.0, 40.0))
            .line_to(v2(0.0, 40.0))
            .close();
        compare("path_solid", &g_path(&segs_of(&p), &o));
    }

    #[test]
    fn adjust_roughness_matches_excalidraw() {
        assert_eq!(adjust_roughness(1.0, 100.0, 100.0, false, false), 1.0);
        assert_eq!(adjust_roughness(2.0, 30.0, 8.0, false, false), 1.0);
        assert!((adjust_roughness(2.0, 8.0, 8.0, false, false) - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(adjust_roughness(2.0, 60.0, 2.0, false, true), 2.0);
    }
}
