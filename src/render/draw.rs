//! Nodes → backend-neutral vector draw operations in world coordinates.
//!
//! The GPU tessellator, the SVG writer and the PDF writer all consume these ops, so every
//! backend draws the same geometry: shape outlines, gradients, images, strokes and glyphs.

use crate::assets;
use crate::geom::{Rect, V2, v2};
use crate::ink;
use crate::model::{
    BlendMode, BorderSide, Cap, Color, CssAngle, Document, FillRule, ImageMode, Join, Node, NodeId,
    NodeKind, Paint, Stop, StrokeAlign,
};
use crate::text::{self, FaceId};
use crate::vector::{self, VPath};

#[derive(Debug, Clone, PartialEq)]
pub enum PaintSpec {
    /// Straight-alpha RGBA, opacity already applied.
    Solid([f32; 4]),
    Linear {
        p0: V2,
        p1: V2,
        stops: Vec<Stop>,
        opacity: f32,
    },
    /// Ellipse with center and the two radius vectors.
    Radial {
        center: V2,
        rx: V2,
        ry: V2,
        stops: Vec<Stop>,
        opacity: f32,
    },
    Image {
        asset: String,
        /// World rect of one copy of the image.
        place: Rect,
        tile: bool,
        opacity: f32,
    },
}

/// A blurred box shadow, in world coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowOp {
    /// The box casting the shadow, after offset and spread.
    pub shape: Rect,
    pub shape_radii: [f64; 4],
    /// Gaussian standard deviation; CSS defines it as half the blur radius.
    pub sigma: f64,
    /// Straight alpha, opacity already applied.
    pub color: [f32; 4],
    /// The node's own box. An outer shadow paints only outside it, an inset one only inside.
    pub box_rect: Rect,
    pub box_radii: [f64; 4],
    pub inset: bool,
}

impl ShadowOp {
    /// Three standard deviations hold everything visible of a Gaussian.
    pub fn bounds(&self) -> Rect {
        if self.inset {
            self.box_rect
        } else {
            self.shape.inflate(3.0 * self.sigma + 1.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DrawOp {
    Fill {
        path: VPath,
        paint: PaintSpec,
        even_odd: bool,
    },
    Stroke {
        path: VPath,
        color: [f32; 4],
        width: f64,
        cap: Cap,
        join: Join,
        dash: Vec<f64>,
    },
    /// Glyphs sharing one face, size and color; positions are world baselines.
    Glyphs {
        face: FaceId,
        size: f64,
        color: [f32; 4],
        glyphs: Vec<(u16, V2)>,
    },
    Shadow(ShadowOp),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeDraw {
    pub ops: Vec<DrawOp>,
    pub blend: BlendMode,
}

pub fn rgba(c: Color, opacity: f64) -> [f32; 4] {
    c.to_f32(opacity)
}

/// The node's outline in world space (None for text and groups).
pub fn shape_path(n: &Node, wr: Rect) -> Option<VPath> {
    Some(match &n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => vector::rounded_rect(wr, n.style.corner_radii()),
        NodeKind::Ellipse => vector::ellipse(wr),
        NodeKind::Polygon { sides } => {
            vector::rounded_polygon(&vector::polygon_points(wr, *sides), n.style.corner_radius)
        }
        NodeKind::Star { points, inner } => vector::rounded_polygon(
            &vector::star_points(wr, *points, *inner),
            n.style.corner_radius,
        ),
        NodeKind::Path { subpaths, .. } => vector::from_subpaths(wr.min(), subpaths),
        NodeKind::Line { a, b } => VPath::polyline(&[wr.min() + *a, wr.min() + *b], false),
        NodeKind::Stroke {
            points,
            width,
            thinning,
        } => {
            let o = ink::outline(points, *width, *thinning);
            let pts: Vec<V2> = o.iter().map(|q| wr.min() + *q).collect();
            VPath::polyline(&pts, true)
        }
        NodeKind::Connector { points, route, .. } => {
            let pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
            crate::connector::line_path(&pts, *route)
        }
        NodeKind::Sticky { .. } => vector::rounded_rect(wr, [n.style.corner_radius.max(2.0); 4]),
        NodeKind::Text { .. } | NodeKind::Group => return None,
    })
}

/// Whether the outline is a closed area (fills apply).
pub fn is_area(n: &Node) -> bool {
    !matches!(
        n.kind,
        NodeKind::Line { .. }
            | NodeKind::Text { .. }
            | NodeKind::Group
            | NodeKind::Connector { .. }
    ) && !matches!(&n.kind, NodeKind::Path { subpaths, .. } if subpaths.iter().all(|s| !s.closed))
}

/// Stroke outline honoring inside/outside alignment for box shapes.
fn stroke_path(n: &Node, wr: Rect) -> Option<VPath> {
    let d = match n.style.stroke_align {
        StrokeAlign::Center => 0.0,
        StrokeAlign::Inside => -n.style.stroke_width * 0.5,
        StrokeAlign::Outside => n.style.stroke_width * 0.5,
    };
    if d == 0.0 {
        return shape_path(n, wr);
    }
    let r = wr.inflate(d);
    if r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    match &n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => Some(vector::rounded_rect(
            r,
            n.style
                .corner_radii()
                .map(|x| if x > 0.0 { (x + d).max(0.0) } else { 0.0 }),
        )),
        NodeKind::Ellipse => Some(vector::ellipse(r)),
        NodeKind::Polygon { sides } => Some(vector::rounded_polygon(
            &vector::polygon_points(r, *sides),
            n.style.corner_radius,
        )),
        NodeKind::Star { points, inner } => Some(vector::rounded_polygon(
            &vector::star_points(r, *points, *inner),
            n.style.corner_radius,
        )),
        _ => shape_path(n, wr),
    }
}

/// Where one copy of an image goes for a given mode.
pub fn image_place(mode: ImageMode, wr: Rect, iw: f64, ih: f64) -> Rect {
    let (iw, ih) = (iw.max(1.0), ih.max(1.0));
    match mode {
        ImageMode::Stretch => wr,
        ImageMode::Tile => Rect::new(wr.x, wr.y, iw, ih),
        ImageMode::Fill | ImageMode::Fit => {
            let s = if mode == ImageMode::Fill {
                (wr.w / iw).max(wr.h / ih)
            } else {
                (wr.w / iw).min(wr.h / ih)
            };
            let size = v2(iw * s, ih * s);
            let c = wr.center();
            Rect::new(c.x - size.x * 0.5, c.y - size.y * 0.5, size.x, size.y)
        }
    }
}

/// The CSS gradient line of a box: through its center, and long enough that the corners get
/// exactly the end colors. Unlike unit-box endpoints, the angle survives non-square boxes.
pub fn css_gradient_line(a: CssAngle, r: Rect) -> (V2, V2) {
    let dir = match a {
        CssAngle::Deg(d) => {
            let t = d.to_radians();
            v2(t.sin(), -t.cos())
        }
        // "Magic corners": the 50% line runs through the two other corners.
        CssAngle::To(x, y) if x != 0 && y != 0 => {
            let d = v2(x as f64 * r.h, y as f64 * r.w);
            let l = d.dot(d).sqrt().max(1e-9);
            v2(d.x / l, d.y / l)
        }
        CssAngle::To(x, y) => v2(x as f64, y as f64),
    };
    let half = ((r.w * dir.x).abs() + (r.h * dir.y).abs()) * 0.5;
    let c = r.center();
    (c - dir * half, c + dir * half)
}

pub fn paint_spec(p: &Paint, wr: Rect, opacity: f64) -> Option<PaintSpec> {
    let at = |u: V2| v2(wr.x + u.x * wr.w, wr.y + u.y * wr.h);
    Some(match p {
        Paint::Solid { color } => PaintSpec::Solid(rgba(*color, opacity)),
        Paint::Linear {
            stops,
            from,
            to,
            css,
        } => {
            let (p0, p1) = match css {
                Some(a) => css_gradient_line(*a, wr),
                None => (at(*from), at(*to)),
            };
            PaintSpec::Linear {
                p0,
                p1,
                stops: sorted(stops),
                opacity: opacity as f32,
            }
        }
        Paint::Radial {
            stops,
            center,
            radius,
        } => PaintSpec::Radial {
            center: at(*center),
            rx: v2(radius.x * wr.w, 0.0),
            ry: v2(0.0, radius.y * wr.h),
            stops: sorted(stops),
            opacity: opacity as f32,
        },
        Paint::Image {
            asset,
            mode,
            opacity: o,
        } => {
            let a = assets::get(asset);
            let (iw, ih) = a
                .as_ref()
                .map_or((wr.w, wr.h), |a| (a.width as f64, a.height as f64));
            if a.is_none() {
                // Missing asset: a neutral placeholder.
                return Some(PaintSpec::Solid(rgba(
                    Color::rgb(0xcc, 0xcc, 0xd2),
                    opacity * o,
                )));
            }
            PaintSpec::Image {
                asset: asset.clone(),
                place: image_place(*mode, wr, iw, ih),
                tile: *mode == ImageMode::Tile,
                opacity: (opacity * o) as f32,
            }
        }
    })
}

fn sorted(stops: &[Stop]) -> Vec<Stop> {
    let mut s = stops.to_vec();
    s.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    s
}

/// Corner radii of a shape that can cast a box shadow (None for everything else).
fn box_radii(n: &Node, wr: Rect) -> Option<[f64; 4]> {
    match n.kind {
        NodeKind::Frame { .. } | NodeKind::Rect => Some(n.style.corner_radii()),
        NodeKind::Ellipse => Some([wr.w.min(wr.h) * 0.5; 4]),
        _ => None,
    }
}

/// The node's box shadows of one kind. CSS paints the first one on top, so they go in reverse.
fn shadow_ops(n: &Node, wr: Rect, opacity: f64, inset: bool, ops: &mut Vec<DrawOp>) {
    let Some(radii) = box_radii(n, wr) else {
        return;
    };
    for s in n.style.shadows.iter().rev().filter(|s| s.inset == inset) {
        // A spread grows the box and its rounded corners; an inset spread shrinks them.
        let spread = if inset { -s.spread } else { s.spread };
        let shape = wr.translate(v2(s.x, s.y)).inflate(spread);
        if shape.w <= 0.0 || shape.h <= 0.0 {
            continue;
        }
        ops.push(DrawOp::Shadow(ShadowOp {
            shape,
            shape_radii: radii.map(|r| if r > 0.0 { (r + spread).max(0.0) } else { 0.0 }),
            sigma: (s.blur * 0.5).max(0.25),
            color: rgba(s.color, opacity),
            box_rect: wr,
            box_radii: radii,
            inset,
        }));
    }
}

/// A border whose sides differ. Square corners get a mitred trapezoid per side, so each side can
/// have its own color. Rounded corners get one ring between the outer and inner edges.
fn border_ops(n: &Node, sides: &[BorderSide; 4], wr: Rect, opacity: f64, ops: &mut Vec<DrawOp>) {
    let [t, r, b, l] = sides.map(|s| s.width.max(0.0));
    let color = |s: &BorderSide| s.color.or(n.style.stroke);
    let inner = Rect::new(
        wr.x + l,
        wr.y + t,
        (wr.w - l - r).max(0.0),
        (wr.h - t - b).max(0.0),
    );
    let radii = box_radii(n, wr).unwrap_or([0.0; 4]);
    if radii.iter().all(|x| *x <= 0.0) {
        let (x0, y0, x1, y1) = (wr.x, wr.y, wr.right(), wr.bottom());
        let (ix0, iy0, ix1, iy1) = (inner.x, inner.y, inner.right(), inner.bottom());
        let polys = [
            [v2(x0, y0), v2(x1, y0), v2(ix1, iy0), v2(ix0, iy0)],
            [v2(x1, y0), v2(x1, y1), v2(ix1, iy1), v2(ix1, iy0)],
            [v2(x1, y1), v2(x0, y1), v2(ix0, iy1), v2(ix1, iy1)],
            [v2(x0, y1), v2(x0, y0), v2(ix0, iy0), v2(ix0, iy1)],
        ];
        for (side, poly) in sides.iter().zip(polys) {
            if let (true, Some(c)) = (side.width > 0.0, color(side)) {
                ops.push(DrawOp::Fill {
                    path: VPath::polyline(poly.as_slice(), true),
                    paint: PaintSpec::Solid(rgba(c, opacity)),
                    even_odd: false,
                });
            }
        }
        return;
    }
    let Some(c) = sides.iter().filter(|s| s.width > 0.0).find_map(color) else {
        return;
    };
    let shrink = [t.max(l), t.max(r), b.max(r), b.max(l)];
    let inner_radii = [0, 1, 2, 3].map(|i| (radii[i] - shrink[i]).max(0.0));
    let mut path = vector::rounded_rect(wr, radii);
    path.append(&vector::rounded_rect(inner, inner_radii));
    ops.push(DrawOp::Fill {
        path,
        paint: PaintSpec::Solid(rgba(c, opacity)),
        even_odd: true,
    });
}

/// Draw ops for one node (not its children), in world space.
pub fn node_draw(n: &Node, wr: Rect, opacity: f64) -> NodeDraw {
    let mut ops = Vec::new();
    if let Some(r) = n.style.rough {
        ops = super::rough::node_ops(n, wr, opacity, &r);
        return NodeDraw {
            ops,
            blend: n.style.blend,
        };
    }
    match &n.kind {
        NodeKind::Group => {}
        NodeKind::Text { .. } => text_ops(n, wr, opacity, &mut ops),
        NodeKind::Connector { .. } => connector_ops(n, wr, opacity, &mut ops, false),
        NodeKind::Sticky { .. } => {
            let c = n
                .style
                .fills
                .last()
                .map_or(Color::rgb(0xff, 0xdf, 0x6b), |p| p.preview_color());
            let rad = [n.style.corner_radius.max(2.0); 4];
            // Soft drop shadow, then the note and its text.
            for (i, a) in [(3.0, 0.05), (1.5, 0.07)] {
                ops.push(DrawOp::Fill {
                    path: vector::rounded_rect(wr.translate(v2(0.0, i)).inflate(i * 0.5), rad),
                    paint: PaintSpec::Solid(rgba(Color::rgba(0, 0, 0, (a * 255.0) as u8), opacity)),
                    even_odd: false,
                });
            }
            ops.push(DrawOp::Fill {
                path: vector::rounded_rect(wr, rad),
                paint: PaintSpec::Solid(rgba(c, opacity)),
                even_odd: false,
            });
            if let Some(sc) = n.style.stroke
                && n.style.stroke_width > 0.0
            {
                ops.push(DrawOp::Stroke {
                    path: vector::rounded_rect(wr, rad),
                    color: rgba(sc, opacity),
                    width: n.style.stroke_width,
                    cap: n.style.cap,
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
            text_ops(n, wr, opacity, &mut ops);
        }
        NodeKind::Line { .. } => {
            if let (Some(c), Some(path)) = (n.style.stroke, shape_path(n, wr)) {
                ops.push(DrawOp::Stroke {
                    path,
                    color: rgba(c, opacity),
                    width: n.style.stroke_width.max(0.0),
                    cap: if n.style.cap == Cap::Butt {
                        Cap::Round
                    } else {
                        n.style.cap
                    },
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
        }
        NodeKind::Stroke { .. } => {
            let c = n
                .style
                .fills
                .last()
                .map_or(Color::INK, |p| p.preview_color());
            if let Some(path) = shape_path(n, wr) {
                ops.push(DrawOp::Fill {
                    path,
                    paint: PaintSpec::Solid(rgba(c, opacity)),
                    even_odd: false,
                });
            }
        }
        _ => {
            let Some(path) = shape_path(n, wr) else {
                return NodeDraw::default();
            };
            let even_odd = matches!(
                n.kind,
                NodeKind::Path {
                    fill_rule: FillRule::EvenOdd,
                    ..
                }
            );
            shadow_ops(n, wr, opacity, false, &mut ops);
            if is_area(n) {
                for p in &n.style.fills {
                    if let Some(paint) = paint_spec(p, wr, opacity) {
                        ops.push(DrawOp::Fill {
                            path: path.clone(),
                            paint,
                            even_odd,
                        });
                    }
                }
            }
            shadow_ops(n, wr, opacity, true, &mut ops);
            if let Some(sides) = &n.style.borders {
                border_ops(n, sides, wr, opacity, &mut ops);
            } else if let Some(c) = n.style.stroke
                && n.style.stroke_width > 0.0
                && let Some(sp) = stroke_path(n, wr)
            {
                ops.push(DrawOp::Stroke {
                    path: sp,
                    color: rgba(c, opacity),
                    width: n.style.stroke_width,
                    cap: n.style.cap,
                    join: n.style.join,
                    dash: n.style.dash.clone(),
                });
            }
        }
    }
    NodeDraw {
        ops,
        blend: n.style.blend,
    }
}

/// A connector's line and heads. With `rough`, only the heads are produced (the line comes
/// from the rough renderer).
pub fn connector_ops(n: &Node, wr: Rect, opacity: f64, ops: &mut Vec<DrawOp>, heads_only: bool) {
    let NodeKind::Connector {
        points,
        route,
        start_head,
        end_head,
        ..
    } = &n.kind
    else {
        return;
    };
    let Some(c) = n.style.stroke else { return };
    let color = rgba(c, opacity);
    let sw = n.style.stroke_width.max(0.5);
    let pts: Vec<V2> = points.iter().map(|p| wr.min() + *p).collect();
    if pts.len() < 2 {
        return;
    }
    if !heads_only {
        ops.push(DrawOp::Stroke {
            path: crate::connector::line_path(&pts, *route),
            color,
            width: sw,
            cap: Cap::Round,
            join: Join::Round,
            dash: n.style.dash.clone(),
        });
    }
    let last = pts.len() - 1;
    for (head, tip, from) in [
        (*start_head, pts[0], pts[1]),
        (*end_head, pts[last], pts[last - 1]),
    ] {
        let (lines, polys) = crate::connector::head_shapes(head, tip, from, sw, tip.dist(from));
        for l in lines {
            ops.push(DrawOp::Stroke {
                path: VPath::polyline(&l, false),
                color,
                width: sw,
                cap: Cap::Round,
                join: Join::Round,
                dash: Vec::new(),
            });
        }
        for p in polys {
            let path = VPath::polyline(&p, true);
            ops.push(DrawOp::Fill {
                path: path.clone(),
                paint: PaintSpec::Solid(color),
                even_odd: false,
            });
            ops.push(DrawOp::Stroke {
                path,
                color,
                width: sw,
                cap: Cap::Round,
                join: Join::Round,
                dash: Vec::new(),
            });
        }
    }
}

fn text_ops(n: &Node, wr: Rect, opacity: f64, ops: &mut Vec<DrawOp>) {
    let Some(l) = text::layout_node(n) else {
        return;
    };
    let o = wr.min() + text::text_offset(n);
    let mut i = 0;
    while i < l.glyphs.len() {
        let g0 = l.glyphs[i];
        let mut glyphs = Vec::new();
        while i < l.glyphs.len() {
            let g = &l.glyphs[i];
            if g.face != g0.face || g.size != g0.size || g.color != g0.color {
                break;
            }
            glyphs.push((g.gid, o + v2(g.x, g.baseline)));
            i += 1;
        }
        ops.push(DrawOp::Glyphs {
            face: g0.face,
            size: g0.size,
            color: rgba(g0.color, opacity),
            glyphs,
        });
    }
    // Underline / strikethrough as thin rectangles.
    for g in l.glyphs.iter().filter(|g| g.underline || g.strike) {
        let th = (g.size / 14.0).max(0.5);
        for (on, dy) in [(g.underline, g.size * 0.12), (g.strike, -g.size * 0.28)] {
            if on {
                let r = Rect::new(o.x + g.x, o.y + g.baseline + dy, g.advance, th);
                ops.push(DrawOp::Fill {
                    path: vector::rounded_rect(r, [0.0; 4]),
                    paint: PaintSpec::Solid(rgba(g.color, opacity)),
                    even_odd: false,
                });
            }
        }
    }
}

/// Product of ancestor opacities.
pub fn inherited_opacity(doc: &Document, id: NodeId) -> f64 {
    doc.ancestors(id)
        .iter()
        .filter_map(|a| doc.get(*a))
        .map(|n| n.style.opacity)
        .product()
}

/// World bounds a node draws into (rect plus stroke overhang).
pub fn draw_bounds(n: &Node, wr: Rect) -> Rect {
    let mut pad = match n.style.stroke_align {
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Center => n.style.stroke_width * 0.5,
        StrokeAlign::Outside => n.style.stroke_width,
    };
    if n.style.stroke.is_none() && !matches!(n.kind, NodeKind::Line { .. }) {
        pad = 0.0;
    }
    if n.style.rough.is_some() {
        pad += n.style.stroke_width * 2.0 + 4.0;
    }
    if matches!(n.kind, NodeKind::Line { .. }) {
        pad = pad.max(n.style.stroke_width * 0.5);
    }
    if matches!(n.kind, NodeKind::Connector { .. }) {
        // Arrowheads reach past the end points.
        pad = pad.max(n.style.stroke_width + 20.0);
    }
    if matches!(n.kind, NodeKind::Sticky { .. }) {
        pad = pad.max(4.0);
    }
    let mut out = wr.inflate(pad);
    for s in n.style.shadows.iter().filter(|s| !s.inset) {
        let reach = s.spread + 1.5 * s.blur + 1.0;
        out = out.union(&wr.translate(v2(s.x, s.y)).inflate(reach));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V2, x: f64, y: f64) -> bool {
        (a.x - x).abs() < 1e-6 && (a.y - y).abs() < 1e-6
    }

    #[test]
    fn css_gradient_lines_use_the_real_box() {
        let r = Rect::new(0.0, 0.0, 400.0, 100.0);
        // Straight down: top edge to bottom edge, whatever the width.
        let (a, b) = css_gradient_line(CssAngle::Deg(180.0), r);
        assert!(
            close(a, 200.0, 0.0) && close(b, 200.0, 100.0),
            "{a:?} {b:?}"
        );
        // 45deg keeps its true angle on a wide box, and reaches past it to the corners.
        let (a, b) = css_gradient_line(CssAngle::Deg(45.0), r);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        assert!((dx + dy).abs() < 1e-6, "45° up and to the right: {dx} {dy}");
        let half = (400.0 + 100.0) * 45f64.to_radians().sin() * 0.5;
        assert!((dx.hypot(dy) * 0.5 - half).abs() < 1e-6);
        // Corner keywords: the 50% line runs through the other two corners.
        let (a, b) = css_gradient_line(CssAngle::To(1, 1), r);
        let d = v2(b.x - a.x, b.y - a.y);
        let across = v2(400.0, -100.0);
        assert!(d.dot(across).abs() < 1e-6, "perpendicular to that diagonal");
    }
}
