//! Arrows and connectors: bound endpoints, straight / curved / elbow routing, arrowheads.
//!
//! The binding rules, constants and the elbow router (orthogonal grid + A* with a bend
//! penalty and Excalidraw's segment-count heuristic) are adapted from Excalidraw
//! (packages/element/src/binding.ts and elbowArrow.ts, Copyright (c) 2020 Excalidraw, MIT).
//! See THIRD_PARTY.md. The router is a simplified port: obstacles are the padded boxes of the
//! bound shapes, without Excalidraw's dynamic-AABB "side hack".

use crate::geom::{Rect, V2, v2};
use crate::model::{Document, Head, Node, NodeId, NodeKind, Route};
use crate::render::draw;
use crate::vector::{self, VPath};

/// Gap between an orbit-bound end and the target outline (`5 + strokeWidth / 2`).
pub fn binding_gap(target_stroke: f64) -> f64 {
    5.0 + target_stroke / 2.0
}

/// How close (screen px) the pointer must be to a shape to bind to it.
pub const BIND_DISTANCE_PX: f64 = 15.0;
pub const ELBOW_PADDING: f64 = 40.0;
pub const ELBOW_RADIUS: f64 = 16.0;

/// Can arrows attach to this node?
pub fn bindable(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Rect
        | NodeKind::Ellipse
        | NodeKind::Polygon { .. }
        | NodeKind::Star { .. }
        | NodeKind::Frame { .. }
        | NodeKind::Text { .. }
        | NodeKind::Sticky { .. } => true,
        NodeKind::Path { subpaths, .. } => subpaths.iter().any(|s| s.closed),
        _ => false,
    }
}

/// The topmost bindable node at `p` (or within `dist` of its outline), skipping `exclude`.
pub fn bind_target(doc: &Document, p: V2, dist: f64, exclude: &[NodeId]) -> Option<NodeId> {
    let order = doc.paint_order();
    let mut best: Option<(NodeId, f64)> = None;
    for id in order.into_iter().rev() {
        if exclude.contains(&id) || !doc.is_effectively_visible(id) || doc.is_effectively_locked(id)
        {
            continue;
        }
        let n = doc.get(id).unwrap();
        if !bindable(n) {
            continue;
        }
        let wr = doc.world_rect(id).unwrap();
        if !wr.inflate(dist).contains(p) {
            continue;
        }
        let path = draw::shape_path(n, wr).unwrap_or_else(|| vector::rounded_rect(wr, [0.0; 4]));
        let inside = path.contains(p, false, 0.5);
        // A label sitting on a shape: bind to the shape, not the text.
        if matches!(n.kind, NodeKind::Text { .. }) && inside {
            let under = doc.paint_order().into_iter().rev().find(|o| {
                *o != id
                    && !exclude.contains(o)
                    && doc.get(*o).is_some_and(|on| {
                        bindable(on) && !matches!(on.kind, NodeKind::Text { .. }) && !on.is_frame()
                    })
                    && doc.world_rect(*o).is_some_and(|r| r.contains(p))
            });
            if under.is_some() {
                continue;
            }
        }
        // Frames bind only from their edge, so arrows can be drawn inside them.
        if inside && n.is_frame() && path.distance(p, 0.5) > dist {
            continue;
        }
        if inside {
            return Some(id);
        }
        let d = path.distance(p, 0.5);
        if d <= dist && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((id, d));
        }
    }
    best.map(|b| b.0)
}

/// Normalized attachment point of `p` in a target box (Excalidraw nudges exact centers to
/// 0.5001 to avoid heading flips).
pub fn fixed_point(p: V2, wr: Rect) -> V2 {
    let f = |c: f64| {
        let c = c.clamp(-10.0, 10.0);
        if (c - 0.5).abs() < 1e-4 { 0.5001 } else { c }
    };
    v2(
        f((p.x - wr.x) / wr.w.max(1.0)),
        f((p.y - wr.y) / wr.h.max(1.0)),
    )
}

pub fn global_fixed(fixed: V2, wr: Rect) -> V2 {
    v2(wr.x + wr.w * fixed.x, wr.y + wr.h * fixed.y)
}

/// The target's outline grown by `gap`, as polygons.
fn outline_polys(doc: &Document, id: NodeId, gap: f64) -> Vec<Vec<V2>> {
    let (Some(n), Some(wr)) = (doc.get(id), doc.world_rect(id)) else {
        return vec![];
    };
    let grown = Node {
        rect: wr.inflate(gap),
        style: crate::model::Style {
            corner_radius: if n.style.corner_radius > 0.0 {
                n.style.corner_radius + gap
            } else {
                0.0
            },
            radii: n
                .style
                .radii
                .map(|r| r.map(|x| if x > 0.0 { x + gap } else { 0.0 })),
            ..n.style.clone()
        },
        ..n.clone()
    };
    let path = match &n.kind {
        NodeKind::Path { .. } => draw::shape_path(n, wr),
        _ => draw::shape_path(&grown, grown.rect),
    }
    .unwrap_or_else(|| vector::rounded_rect(wr.inflate(gap), [0.0; 4]));
    path.flatten(0.25)
        .into_iter()
        .filter(|p| p.pts.len() > 2)
        .map(|p| p.pts)
        .collect()
}

fn seg_intersect(a: V2, b: V2, c: V2, d: V2) -> Option<V2> {
    let r = b - a;
    let s = d - c;
    let den = r.x * s.y - r.y * s.x;
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((c.x - a.x) * s.y - (c.y - a.y) * s.x) / den;
    let u = ((c.x - a.x) * r.y - (c.y - a.y) * r.x) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| a + r * t)
}

/// Where segment `a→b` crosses the target outline (grown by `gap`), nearest to `b`.
pub fn outline_hit(doc: &Document, id: NodeId, a: V2, b: V2, gap: f64) -> Option<V2> {
    let mut best: Option<(V2, f64)> = None;
    for poly in outline_polys(doc, id, gap) {
        let n = poly.len();
        for i in 0..n {
            if let Some(p) = seg_intersect(a, b, poly[i], poly[(i + 1) % n]) {
                let d = p.dist(b);
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((p, d));
                }
            }
        }
    }
    best.map(|b| b.0)
}

/// Excalidraw's `headingForPointFromElement` for boxes: which side of `r` the point faces.
pub fn heading_from_box(r: Rect, p: V2) -> V2 {
    let c = r.center();
    let d = p - c;
    // Compare against the box's diagonals (aspect-aware).
    let nx = d.x / r.w.max(1e-9);
    let ny = d.y / r.h.max(1e-9);
    if ny.abs() >= nx.abs() {
        if ny < 0.0 {
            v2(0.0, -1.0)
        } else {
            v2(0.0, 1.0)
        }
    } else if nx > 0.0 {
        v2(1.0, 0.0)
    } else {
        v2(-1.0, 0.0)
    }
}

pub fn vector_heading(v: V2) -> V2 {
    if v.x > v.y.abs() {
        v2(1.0, 0.0)
    } else if v.x <= -v.y.abs() {
        v2(-1.0, 0.0)
    } else if v.y > v.x.abs() {
        v2(0.0, 1.0)
    } else {
        v2(0.0, -1.0)
    }
}

/// Recompute the world points of a connector from its bindings. Returns None if unchanged
/// inputs are missing.
pub fn resolve(doc: &Document, id: NodeId) -> Option<Vec<V2>> {
    let n = doc.get(id)?;
    let wr = doc.world_rect(id)?;
    let NodeKind::Connector {
        points,
        start,
        end,
        route,
        start_head,
        end_head,
    } = &n.kind
    else {
        return None;
    };
    let mut pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
    if pts.len() < 2 {
        return Some(pts);
    }
    let focus = |b: &crate::model::Binding| -> Option<(V2, Rect, f64)> {
        let t = doc.get(b.node)?;
        let tr = doc.world_rect(b.node)?;
        Some((global_fixed(b.fixed, tr), tr, t.style.stroke_width))
    };
    let sf = start.as_ref().and_then(|b| focus(b).map(|f| (b, f)));
    let ef = end.as_ref().and_then(|b| focus(b).map(|f| (b, f)));
    if *route == Route::Elbow {
        let last = pts.len() - 1;
        let s = sf.map_or(pts[0], |(_, (f, _, _))| f);
        let e = ef.map_or(pts[last], |(_, (f, _, _))| f);
        return Some(route_elbow(
            doc,
            s,
            sf.map(|(b, (_, r, sw))| (b.node, r, sw)),
            e,
            ef.map(|(b, (_, r, sw))| (b.node, r, sw)),
            *start_head != Head::None,
            *end_head != Head::None,
        ));
    }
    let n_pts = pts.len();
    // Anchor each end toward the other end's focus (2 points) or its neighbor point.
    let s_anchor = if n_pts == 2 {
        ef.map_or(pts[1], |(_, (f, _, _))| f)
    } else {
        pts[1]
    };
    let e_anchor = if n_pts == 2 {
        sf.map_or(pts[0], |(_, (f, _, _))| f)
    } else {
        pts[n_pts - 2]
    };
    if let Some((b, (f, _, sw))) = sf {
        pts[0] = if b.inside {
            f
        } else {
            outline_hit(doc, b.node, f, s_anchor, binding_gap(sw)).unwrap_or(f)
        };
    }
    if let Some((b, (f, _, sw))) = ef {
        pts[n_pts - 1] = if b.inside {
            f
        } else {
            outline_hit(doc, b.node, f, e_anchor, binding_gap(sw)).unwrap_or(f)
        };
    }
    Some(pts)
}

// ---------------------------------------------------------------------------------- elbow

#[derive(Clone, Copy, PartialEq)]
struct Dir(i8, i8);

const UP: Dir = Dir(0, -1);
const RIGHT: Dir = Dir(1, 0);
const DOWN: Dir = Dir(0, 1);
const LEFT: Dir = Dir(-1, 0);

fn dir_of(v: V2) -> Dir {
    let h = vector_heading(v);
    Dir(h.x as i8, h.y as i8)
}

fn flip(d: Dir) -> Dir {
    Dir(-d.0, -d.1)
}

/// Excalidraw's `estimateSegmentCount` lookup (start node `s`, end node `e`).
fn estimate_segments(s: V2, e: V2, sh: Dir, eh: Dir) -> f64 {
    let r = if eh == RIGHT {
        if sh == RIGHT {
            if s.x >= e.x {
                4
            } else if s.y == e.y {
                0
            } else {
                2
            }
        } else if sh == UP {
            if s.y > e.y && s.x < e.x { 1 } else { 3 }
        } else if sh == DOWN {
            if s.y < e.y && s.x < e.x { 1 } else { 3 }
        } else if s.y == e.y {
            4
        } else {
            2
        }
    } else if eh == LEFT {
        if sh == RIGHT {
            if s.y == e.y { 4 } else { 2 }
        } else if sh == UP {
            if s.y > e.y && s.x > e.x { 1 } else { 3 }
        } else if sh == DOWN {
            if s.y < e.y && s.x > e.x { 1 } else { 3 }
        } else if s.x <= e.x {
            4
        } else if s.y == e.y {
            0
        } else {
            2
        }
    } else if eh == UP {
        if sh == RIGHT {
            if s.y > e.y && s.x < e.x { 1 } else { 3 }
        } else if sh == UP {
            if s.y >= e.y {
                4
            } else if s.x == e.x {
                0
            } else {
                2
            }
        } else if sh == DOWN {
            if s.x == e.x { 4 } else { 2 }
        } else if s.y > e.y && s.x > e.x {
            1
        } else {
            3
        }
    } else if sh == RIGHT {
        if s.y < e.y && s.x < e.x { 1 } else { 3 }
    } else if sh == UP {
        if s.x == e.x { 4 } else { 2 }
    } else if sh == DOWN {
        if s.y <= e.y {
            4
        } else if s.x == e.x {
            0
        } else {
            2
        }
    } else if s.y < e.y && s.x > e.x {
        1
    } else {
        3
    };
    r as f64
}

fn manhattan(a: V2, b: V2) -> f64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn strictly_inside(r: &Rect, p: V2) -> bool {
    p.x > r.x && p.x < r.right() && p.y > r.y && p.y < r.bottom()
}

/// Orthogonal route from `s` to `e`. Bound ends leave their shape perpendicular to the
/// side they're on; the path avoids the shapes' padded boxes.
pub fn route_elbow(
    _doc: &Document,
    s: V2,
    s_bound: Option<(NodeId, Rect, f64)>,
    e: V2,
    e_bound: Option<(NodeId, Rect, f64)>,
    s_head: bool,
    e_head: bool,
) -> Vec<V2> {
    // Attached near the center: leave toward the other end; otherwise from the nearest side.
    let centered = |r: Rect, p: V2| {
        let d = p - r.center();
        (d.x / r.w.max(1.0)).abs() < 0.15 && (d.y / r.h.max(1.0)).abs() < 0.15
    };
    let sh = match s_bound {
        Some((_, r, _)) if !centered(r, s) => dir_of(heading_from_box(r, s)),
        _ => dir_of(e - s),
    };
    let eh = match e_bound {
        Some((_, r, _)) if !centered(r, e) => dir_of(heading_from_box(r, e)),
        _ => dir_of(s - e),
    };
    // Endpoints on the outline + gap, then dongles out by the padding.
    let snap = |p: V2, b: Option<(NodeId, Rect, f64)>, h: Dir, head: bool| -> V2 {
        match b {
            Some((_, r, sw)) => {
                // Keep the attachment inside the side's span.
                let p = v2(p.x.clamp(r.x, r.right()), p.y.clamp(r.y, r.bottom()));
                let gap =
                    binding_gap(sw) * if head { 1.0 } else { 0.5 } + if head { 2.0 } else { 0.0 };
                match h {
                    UP => v2(p.x, r.y - gap),
                    DOWN => v2(p.x, r.bottom() + gap),
                    LEFT => v2(r.x - gap, p.y),
                    _ => v2(r.right() + gap, p.y),
                }
            }
            None => p,
        }
    };
    let sp = snap(s, s_bound, sh, s_head);
    let ep = snap(e, e_bound, eh, e_head);
    let pad = |b: Option<(NodeId, Rect, f64)>, p: V2| {
        b.map(|(_, r, _)| r.inflate(ELBOW_PADDING))
            .unwrap_or_else(|| Rect::new(p.x - 2.0, p.y - 2.0, 4.0, 4.0))
    };
    let sa = pad(s_bound, sp);
    let ea = pad(e_bound, ep);
    let dongle = |p: V2, h: Dir, r: Rect, bound: bool| -> V2 {
        if !bound {
            return p;
        }
        match h {
            UP => v2(p.x, r.y),
            DOWN => v2(p.x, r.bottom()),
            LEFT => v2(r.x, p.y),
            _ => v2(r.right(), p.y),
        }
    };
    let sd = dongle(sp, sh, sa, s_bound.is_some());
    let ed = dongle(ep, eh, ea, e_bound.is_some());
    let overlap = strictly_inside(&ea, sd) || strictly_inside(&sa, ed);
    let obstacles: Vec<Rect> = if overlap {
        vec![]
    } else {
        [s_bound.map(|_| sa), e_bound.map(|_| ea)]
            .into_iter()
            .flatten()
            .collect()
    };
    let common = sa.union(&ea).inflate(ELBOW_PADDING);
    // Grid lines.
    let mut xs: Vec<f64> = vec![
        sd.x,
        ed.x,
        sa.x,
        sa.right(),
        ea.x,
        ea.right(),
        common.x,
        common.right(),
    ];
    let mut ys: Vec<f64> = vec![
        sd.y,
        ed.y,
        sa.y,
        sa.bottom(),
        ea.y,
        ea.bottom(),
        common.y,
        common.bottom(),
    ];
    // Midlines between the shapes give the classic Z routes.
    xs.push((sa.right() + ea.x) * 0.5);
    xs.push((ea.right() + sa.x) * 0.5);
    ys.push((sa.bottom() + ea.y) * 0.5);
    ys.push((ea.bottom() + sa.y) * 0.5);
    let dedup = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    };
    dedup(&mut xs);
    dedup(&mut ys);
    let cols = xs.len();
    let idx = |x: f64, y: f64| -> Option<usize> {
        let c = xs.iter().position(|v| (v - x).abs() < 1e-9)?;
        let r = ys.iter().position(|v| (v - y).abs() < 1e-9)?;
        Some(r * cols + c)
    };
    let (Some(start), Some(goal)) = (idx(sd.x, sd.y), idx(ed.x, ed.y)) else {
        return vec![sp, ep];
    };
    let n = xs.len() * ys.len();
    let pos = |i: usize| v2(xs[i % cols], ys[i / cols]);
    let bend = manhattan(sd, ed).max(1.0);
    let mut g = vec![f64::INFINITY; n];
    let mut f = vec![f64::INFINITY; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut closed = vec![false; n];
    let mut open: Vec<usize> = vec![start];
    g[start] = 0.0;
    f[start] = 0.0;
    let mut found = false;
    while !open.is_empty() {
        // Lowest f (first on ties, like the binary heap's strict comparisons).
        let (k, _) = open
            .iter()
            .enumerate()
            .fold((0, f64::INFINITY), |acc, (k, i)| {
                if f[*i] < acc.1 { (k, f[*i]) } else { acc }
            });
        let cur = open.remove(k);
        if closed[cur] {
            continue;
        }
        if cur == goal {
            found = true;
            break;
        }
        closed[cur] = true;
        let (c, r) = (cur % cols, cur / cols);
        let prev = match parent[cur] {
            Some(p) => dir_of(pos(cur) - pos(p)),
            None => sh,
        };
        let nbrs = [
            (r > 0).then(|| cur - cols),
            (c + 1 < cols).then(|| cur + 1),
            (r + 1 < ys.len()).then(|| cur + cols),
            (c > 0).then(|| cur - 1),
        ];
        for (i, nb) in nbrs.into_iter().enumerate() {
            let Some(nb) = nb else { continue };
            if closed[nb] {
                continue;
            }
            let mid = (pos(nb) + pos(cur)) * 0.5;
            if obstacles.iter().any(|o| strictly_inside(o, mid)) {
                continue;
            }
            let nh = [UP, RIGHT, DOWN, LEFT][i];
            if nh == flip(prev) || (nb == start && nh == sh) || (nb == goal && nh == eh) {
                continue;
            }
            let ng =
                g[cur] + manhattan(pos(nb), pos(cur)) + if prev != nh { bend.powi(3) } else { 0.0 };
            if ng < g[nb] {
                parent[nb] = Some(cur);
                g[nb] = ng;
                f[nb] = ng
                    + manhattan(pos(goal), pos(nb))
                    + estimate_segments(pos(nb), pos(goal), nh, eh) * bend * bend;
                open.push(nb);
            }
        }
    }
    let mut path = Vec::new();
    if found {
        let mut cur = Some(goal);
        while let Some(c) = cur {
            path.push(pos(c));
            cur = parent[c];
        }
        path.reverse();
    } else {
        path = vec![sd, v2(ed.x, sd.y), ed];
    }
    let mut pts = vec![sp];
    pts.extend(path);
    pts.push(ep);
    simplify_orthogonal(pts)
}

/// Drop duplicate and collinear points.
fn simplify_orthogonal(pts: Vec<V2>) -> Vec<V2> {
    let mut out: Vec<V2> = Vec::new();
    for p in pts {
        if out.last().is_some_and(|l| l.dist(p) <= 1.0) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            let h1 = (b.y - a.y).abs() < (b.x - a.x).abs();
            let h2 = (p.y - b.y).abs() < (p.x - b.x).abs();
            if h1 == h2 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

// ---------------------------------------------------------------------------------- drawing

/// The connector's line through world points.
pub fn line_path(pts: &[V2], route: Route) -> VPath {
    match route {
        Route::Straight => VPath::polyline(pts, false),
        Route::Curved if pts.len() >= 3 => {
            // Catmull-Rom through the points (rough.js `curveToBezier`).
            let mut p = VPath::new();
            let mut q = vec![pts[0]];
            q.extend_from_slice(pts);
            q.push(*pts.last().unwrap());
            p.move_to(pts[0]);
            for i in 1..q.len() - 2 {
                let b1 = q[i] + (q[i + 1] - q[i - 1]) / 6.0;
                let b2 = q[i + 1] - (q[i + 2] - q[i]) / 6.0;
                p.cubic_to(b1, b2, q[i + 1]);
            }
            p
        }
        Route::Curved => VPath::polyline(pts, false),
        Route::Elbow => {
            // Rounded corners (Excalidraw `generateElbowArrowShape`, radius 16).
            let mut p = VPath::new();
            p.move_to(pts[0]);
            for i in 1..pts.len().saturating_sub(1) {
                let (a, c, b) = (pts[i - 1], pts[i], pts[i + 1]);
                let r = ELBOW_RADIUS.min(c.dist(a) / 2.0).min(c.dist(b) / 2.0);
                p.line_to(c + (a - c).normalized() * r);
                p.quad_to(c, c + (b - c).normalized() * r);
            }
            p.line_to(*pts.last().unwrap());
            p
        }
    }
}

/// Head geometry at `tip`, arriving from `from`: (stroked lines, filled polygons).
pub fn head_shapes(
    head: Head,
    tip: V2,
    from: V2,
    sw: f64,
    seg_len: f64,
) -> (Vec<Vec<V2>>, Vec<Vec<V2>>) {
    let size: f64 = match head {
        Head::Arrow => 25.0,
        Head::Diamond => 12.0,
        _ => 15.0,
    };
    let size = size.min(seg_len * if head == Head::Diamond { 0.25 } else { 0.5 }) + sw * 0.5;
    let dir = (tip - from).normalized();
    if dir == V2::ZERO {
        return (vec![], vec![]);
    }
    let back = tip - dir * size;
    let rot = |p: V2, c: V2, deg: f64| {
        let a = deg.to_radians();
        let (s, co) = a.sin_cos();
        let d = p - c;
        c + v2(d.x * co - d.y * s, d.x * s + d.y * co)
    };
    match head {
        Head::None => (vec![], vec![]),
        Head::Arrow => {
            let a = rot(back, tip, -20.0);
            let b = rot(back, tip, 20.0);
            (vec![vec![a, tip, b]], vec![])
        }
        Head::Bar => {
            let a = rot(back, tip, -90.0);
            let b = rot(back, tip, 90.0);
            let half = (a - b) * 0.5;
            (vec![vec![tip + half * 0.6, tip - half * 0.6]], vec![])
        }
        Head::Triangle => {
            let a = rot(back, tip, -25.0);
            let b = rot(back, tip, 25.0);
            (vec![], vec![vec![tip, a, b]])
        }
        Head::Dot => {
            let r = (size * 0.35).max(sw);
            let c = tip - dir * r;
            let pts = (0..24)
                .map(|i| {
                    let a = i as f64 / 24.0 * std::f64::consts::TAU;
                    c + v2(a.cos(), a.sin()) * r
                })
                .collect();
            (vec![], vec![pts])
        }
        Head::Diamond => {
            let a = rot(back, tip, -25.0);
            let b = rot(back, tip, 25.0);
            let far = tip - dir * size * 2.0;
            (vec![], vec![vec![tip, a, far, b]])
        }
    }
}

/// Where the line must stop so a filled head doesn't show the line through it.
pub fn line_inset(head: Head) -> f64 {
    match head {
        Head::Triangle | Head::Diamond | Head::Dot => 1.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Recording, Tx};

    #[test]
    fn elbow_routes_are_orthogonal_and_leave_perpendicular() {
        let doc = Document::default();
        let a = Rect::new(0.0, 0.0, 100.0, 60.0);
        let b = Rect::new(300.0, 200.0, 100.0, 60.0);
        let pts = route_elbow(
            &doc,
            v2(100.0, 30.0),
            Some((NodeId(1), a, 2.0)),
            v2(300.0, 230.0),
            Some((NodeId(2), b, 2.0)),
            false,
            true,
        );
        assert!(pts.len() >= 3, "{pts:?}");
        for w in pts.windows(2) {
            assert!(
                (w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6,
                "not orthogonal: {pts:?}"
            );
        }
        assert!(
            pts[0].x > 100.0 && (pts[0].y - 30.0).abs() < 1e-9,
            "leaves the right side: {pts:?}"
        );
        assert!(
            pts.last().unwrap().x < 300.0,
            "enters the left side: {pts:?}"
        );
        // The route never crosses either box.
        for w in pts.windows(2) {
            let m = (w[0] + w[1]) * 0.5;
            assert!(!strictly_inside(&a, m) && !strictly_inside(&b, m));
        }
    }

    #[test]
    fn orbit_binding_stops_at_outline_plus_gap() {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let r = tx.alloc_id();
        tx.insert(
            Node::new(r, NodeKind::Rect, Rect::new(200.0, 0.0, 100.0, 100.0)),
            None,
            None,
        );
        let hit = outline_hit(
            tx.doc(),
            r,
            v2(250.0, 50.0),
            v2(0.0, 50.0),
            binding_gap(1.0),
        )
        .unwrap();
        assert!((hit.x - (200.0 - 5.5)).abs() < 1e-9, "{hit:?}");
        assert_eq!(
            fixed_point(v2(250.0, 50.0), Rect::new(200.0, 0.0, 100.0, 100.0)),
            v2(0.5001, 0.5001)
        );
    }
}
