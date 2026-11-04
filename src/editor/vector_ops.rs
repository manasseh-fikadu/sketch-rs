//! Vector editing: boolean operations, flatten, the pen tool and path point editing.

use super::{Editor, Tool};
use crate::geom::{Rect, V2, v2};
use crate::history::Tx;
use crate::model::{FillRule, Node, NodeId, NodeKind, PathPoint, SubPath};
use crate::render::draw;
use crate::text;
use crate::vector::{self, VPath};
use i_overlay::core::fill_rule::FillRule as OvFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl BoolOp {
    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Union => "Union",
            BoolOp::Subtract => "Subtract",
            BoolOp::Intersect => "Intersect",
            BoolOp::Exclude => "Exclude",
        }
    }
}

type Contours = Vec<Vec<[f64; 2]>>;

/// A node's filled area as world-space polygons.
pub fn node_contours(doc: &crate::model::Document, id: NodeId) -> Contours {
    let Some(n) = doc.get(id) else { return vec![] };
    let Some(wr) = doc.world_rect(id) else {
        return vec![];
    };
    let path = match &n.kind {
        NodeKind::Text { .. } => {
            let mut p = VPath::new();
            if let Some(l) = text::layout_node(n) {
                for g in &l.glyphs {
                    p.append(&text::glyph_path(
                        g.face,
                        g.gid,
                        wr.min() + v2(g.x, g.baseline),
                        g.size,
                    ));
                }
            }
            p
        }
        NodeKind::Group | NodeKind::Frame { .. } if !n.children.is_empty() => {
            // Containers contribute their children's union.
            let mut acc: Contours = Vec::new();
            for c in &n.children {
                let cc = node_contours(doc, *c);
                acc = if acc.is_empty() {
                    cc
                } else {
                    shapes_to_contours(acc.overlay(&cc, OverlayRule::Union, OvFill::NonZero))
                };
            }
            return acc;
        }
        _ => match draw::shape_path(n, wr) {
            Some(p) => p,
            None => return vec![],
        },
    };
    path.flatten(0.05)
        .into_iter()
        .filter(|p| p.pts.len() >= 3)
        .map(|p| p.pts.iter().map(|q| [q.x, q.y]).collect())
        .collect()
}

fn shapes_to_contours(s: Vec<Vec<Vec<[f64; 2]>>>) -> Contours {
    s.into_iter().flatten().collect()
}

pub fn boolean(op: BoolOp, operands: &[Contours]) -> Contours {
    let Some(first) = operands.first() else {
        return vec![];
    };
    let mut acc = first.clone();
    match op {
        BoolOp::Subtract => {
            for o in &operands[1..] {
                acc = shapes_to_contours(acc.overlay(o, OverlayRule::Difference, OvFill::NonZero));
            }
        }
        _ => {
            let rule = match op {
                BoolOp::Union => OverlayRule::Union,
                BoolOp::Intersect => OverlayRule::Intersect,
                _ => OverlayRule::Xor,
            };
            for o in &operands[1..] {
                acc = shapes_to_contours(acc.overlay(o, rule, OvFill::NonZero));
            }
        }
    }
    acc
}

/// Bounds of point lists.
fn contour_bounds(c: &Contours) -> Option<Rect> {
    let pts: Vec<V2> = c.iter().flatten().map(|p| v2(p[0], p[1])).collect();
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

/// Recompute a path node's box from its points (keeping world positions).
pub fn normalize_path(tx: &mut Tx, id: NodeId) {
    let Some(n) = tx.doc().get(id) else { return };
    let NodeKind::Path { subpaths, .. } = &n.kind else {
        return;
    };
    let p = vector::from_subpaths(V2::ZERO, subpaths);
    // Use the flattened curve so handles don't inflate the box.
    let pts: Vec<V2> = p.flatten(0.1).into_iter().flat_map(|q| q.pts).collect();
    let Some(f) = pts.first() else { return };
    let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x, f.y);
    for q in &pts {
        x0 = x0.min(q.x);
        y0 = y0.min(q.y);
        x1 = x1.max(q.x);
        y1 = y1.max(q.y);
    }
    let d = v2(x0, y0);
    if d == V2::ZERO && n.rect.w == x1 - x0 && n.rect.h == y1 - y0 {
        return;
    }
    let node = tx.node_mut(id).unwrap();
    if let NodeKind::Path { subpaths, .. } = &mut node.kind {
        for sp in subpaths.iter_mut() {
            for pt in sp.points.iter_mut() {
                pt.p = pt.p - d;
                if let Some(h) = &mut pt.h_in {
                    *h = *h - d;
                }
                if let Some(h) = &mut pt.h_out {
                    *h = *h - d;
                }
            }
        }
    }
    node.rect = Rect::new(node.rect.x + d.x, node.rect.y + d.y, x1 - x0, y1 - y0);
}

/// Pen tool state between clicks.
#[derive(Debug, Clone)]
pub struct PenState {
    pub id: NodeId,
    /// The point being dragged out (its handles follow the pointer).
    pub dragging: bool,
}

/// A selected point while editing a path: (subpath, index).
#[derive(Debug, Clone, Default)]
pub struct PathEdit {
    pub id: NodeId,
    pub selected: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointPart {
    Anchor,
    In,
    Out,
}

const PEN_CLOSE_PX: f64 = 8.0;

impl Editor {
    /// Boolean-combine the selection into one vector layer.
    pub fn boolean(&mut self, op: BoolOp) {
        let mut ids = self.top_selection();
        if ids.len() < 2 {
            self.status = "Select two or more layers".into();
            return;
        }
        // Paint order, bottom first.
        let order = self.doc.paint_order();
        ids.sort_by_key(|id| order.iter().position(|x| x == id));
        let operands: Vec<Contours> = ids.iter().map(|id| node_contours(&self.doc, *id)).collect();
        let result = boolean(op, &operands);
        let style_from = if op == BoolOp::Subtract {
            ids[0]
        } else {
            *ids.last().unwrap()
        };
        let top = *ids.last().unwrap();
        let parent = self.doc.get(top).and_then(|n| n.parent);
        let index = self
            .doc
            .children(parent)
            .iter()
            .position(|x| *x == top)
            .map(|i| i + 1);
        let style = self.doc.get(style_from).unwrap().style.clone();
        let Some(b) = contour_bounds(&result) else {
            // Empty result (e.g. disjoint intersect): just remove the operands.
            self.edit(op.label(), |tx| {
                for id in &ids {
                    tx.remove(*id);
                }
            });
            self.selection.clear();
            return;
        };
        let id = self.edit(op.label(), |tx| {
            let o = tx.doc().parent_origin(parent);
            let nid = tx.alloc_id();
            let subpaths: Vec<SubPath> = result
                .iter()
                .map(|c| SubPath {
                    points: c
                        .iter()
                        .map(|p| PathPoint::corner(v2(p[0], p[1]) - b.min()))
                        .collect(),
                    closed: true,
                })
                .collect();
            let mut n = Node::new(
                nid,
                NodeKind::Path {
                    subpaths,
                    fill_rule: FillRule::EvenOdd,
                },
                b.translate(V2::ZERO - o),
            );
            n.style = style;
            n.style.rough = None;
            n.name = op.label().into();
            tx.insert(n, parent, index);
            for id in &ids {
                tx.remove(*id);
            }
            nid
        });
        self.selection = vec![id];
    }

    /// Convert selected shapes (and text) to editable vector paths (Cmd+E).
    pub fn flatten_selection(&mut self) {
        let ids: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| {
                self.doc
                    .get(*id)
                    .is_some_and(|n| !n.is_container() && !matches!(n.kind, NodeKind::Path { .. }))
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        self.edit("Flatten", |tx| {
            for id in ids {
                let n = tx.doc().get(id).unwrap().clone();
                let wr = tx.doc().world_rect(id).unwrap();
                let path = match &n.kind {
                    NodeKind::Text { .. } => {
                        let mut p = VPath::new();
                        if let Some(l) = text::layout_node(&n) {
                            for g in &l.glyphs {
                                p.append(&text::glyph_path(
                                    g.face,
                                    g.gid,
                                    wr.min() + v2(g.x, g.baseline),
                                    g.size,
                                ));
                            }
                        }
                        p
                    }
                    _ => draw::shape_path(&n, wr).unwrap_or_default(),
                };
                let sub = vector::to_subpaths(&path, wr.min());
                let m = tx.node_mut(id).unwrap();
                if matches!(n.kind, NodeKind::Line { .. }) {
                    // A line keeps its stroke and has no fill.
                    m.style.fills.clear();
                }
                m.kind = NodeKind::Path {
                    subpaths: sub,
                    fill_rule: FillRule::NonZero,
                };
                if n.name == n.kind.type_name() {
                    m.name = "Vector".into();
                }
                normalize_path(tx, id);
            }
        });
    }

    // ---------------------------------------------------------------------- pen

    pub(super) fn pen_down(&mut self, world: V2, pos: V2) {
        let p = if self.snapping { world.round() } else { world };
        match self.pen.clone() {
            None => {
                self.end_text_edit();
                self.begin("Draw path");
                let parent = self.frame_at(p, &[]);
                let o = self.doc.parent_origin(parent);
                let id = {
                    let mut tx = self.tx();
                    let id = tx.alloc_id();
                    let mut n = Node::new(
                        id,
                        NodeKind::Path {
                            subpaths: vec![SubPath {
                                points: vec![PathPoint::corner(V2::ZERO)],
                                closed: false,
                            }],
                            fill_rule: FillRule::NonZero,
                        },
                        Rect::new(p.x - o.x, p.y - o.y, 0.0, 0.0),
                    );
                    n.style.fills.clear();
                    n.style.stroke = Some(crate::model::Color::INK);
                    n.style.stroke_width = 2.0;
                    tx.insert(n, parent, None)
                };
                self.selection = vec![id];
                self.pen = Some(PenState { id, dragging: true });
            }
            Some(st) => {
                let Some(wr) = self.doc.world_rect(st.id) else {
                    self.pen = None;
                    return;
                };
                let first = match &self.doc.get(st.id).unwrap().kind {
                    NodeKind::Path { subpaths, .. } => subpaths[0].points.first().map(|q| q.p),
                    _ => None,
                };
                let n_pts = self.point_count(st.id);
                if let Some(f) = first
                    && n_pts >= 2
                    && self.camera.world_to_screen(wr.min() + f).dist(pos) <= PEN_CLOSE_PX
                {
                    self.with_path(st.id, |sp| sp[0].closed = true);
                    self.finish_pen();
                    return;
                }
                let local = p - wr.min();
                self.with_path(st.id, |sp| sp[0].points.push(PathPoint::corner(local)));
                self.pen = Some(PenState {
                    id: st.id,
                    dragging: true,
                });
            }
        }
    }

    /// Dragging after a pen click pulls out symmetric handles (a smooth point).
    pub(super) fn pen_drag(&mut self, world: V2) {
        let Some(st) = self.pen.clone().filter(|s| s.dragging) else {
            return;
        };
        let Some(wr) = self.doc.world_rect(st.id) else {
            return;
        };
        let local = world - wr.min();
        let alt = self.mods.alt;
        self.with_path(st.id, |sp| {
            if let Some(pt) = sp[0].points.last_mut() {
                if pt.p.dist(local) < 0.5 {
                    pt.h_out = None;
                    pt.h_in = None;
                    return;
                }
                pt.h_out = Some(local);
                if !alt {
                    pt.h_in = Some(pt.p - (local - pt.p));
                }
            }
        });
    }

    pub(super) fn pen_up(&mut self) {
        if let Some(st) = &mut self.pen {
            st.dragging = false;
        }
    }

    fn point_count(&self, id: NodeId) -> usize {
        match self.doc.get(id).map(|n| &n.kind) {
            Some(NodeKind::Path { subpaths, .. }) => subpaths.iter().map(|s| s.points.len()).sum(),
            _ => 0,
        }
    }

    /// Edit a path's subpaths (node-local coordinates), then re-fit its box.
    pub fn with_path(&mut self, id: NodeId, f: impl FnOnce(&mut Vec<SubPath>)) {
        let mut tx = self.tx();
        if let Some(n) = tx.node_mut(id)
            && let NodeKind::Path { subpaths, .. } = &mut n.kind
        {
            f(subpaths);
        }
        normalize_path(&mut tx, id);
    }

    /// Finish the path being drawn (Enter / Esc / closing click).
    pub fn finish_pen(&mut self) {
        let Some(st) = self.pen.take() else { return };
        if self.point_count(st.id) < 2 {
            self.tx().remove(st.id);
            self.selection.clear();
        } else {
            let closed = matches!(
                self.doc.get(st.id).map(|n| &n.kind),
                Some(NodeKind::Path { subpaths, .. }) if subpaths[0].closed
            );
            if closed {
                // Closed shapes get a fill, like Figma's default.
                let mut tx = self.tx();
                let n = tx.node_mut(st.id).unwrap();
                n.style
                    .set_fill(Some(crate::model::Color::rgb(0xd9, 0xd9, 0xd9)));
            }
            self.selection = vec![st.id];
        }
        self.commit();
        self.tool = Tool::Select;
    }

    // ---------------------------------------------------------------------- point editing

    pub fn start_path_edit(&mut self, id: NodeId) {
        if matches!(
            self.doc.get(id).map(|n| &n.kind),
            Some(NodeKind::Path { .. })
        ) {
            self.path_edit = Some(PathEdit { id, selected: None });
            self.selection = vec![id];
        }
    }

    /// The point part under a screen position while editing a path.
    pub fn path_part_at(&self, pos: V2) -> Option<(usize, usize, PointPart)> {
        let pe = self.path_edit.as_ref()?;
        let n = self.doc.get(pe.id)?;
        let wr = self.doc.world_rect(pe.id)?;
        let NodeKind::Path { subpaths, .. } = &n.kind else {
            return None;
        };
        let scr = |p: V2| self.camera.world_to_screen(wr.min() + p);
        let mut best: Option<((usize, usize, PointPart), f64)> = None;
        for (si, sp) in subpaths.iter().enumerate() {
            for (pi, pt) in sp.points.iter().enumerate() {
                let mut cands = vec![(PointPart::Anchor, pt.p)];
                if pe.selected == Some((si, pi)) {
                    cands.extend(pt.h_in.map(|h| (PointPart::In, h)));
                    cands.extend(pt.h_out.map(|h| (PointPart::Out, h)));
                }
                for (part, p) in cands {
                    let d = scr(p).dist(pos);
                    if d <= 7.0 && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some(((si, pi, part), d));
                    }
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Move a point part to a world position. Anchors carry their handles; handles mirror
    /// the opposite handle unless Alt is held.
    pub fn drag_path_part(&mut self, si: usize, pi: usize, part: PointPart, world: V2) {
        let Some(pe) = self.path_edit.clone() else {
            return;
        };
        let Some(wr) = self.doc.world_rect(pe.id) else {
            return;
        };
        let local = world - wr.min();
        let alt = self.mods.alt;
        self.with_path(pe.id, |sp| {
            let Some(pt) = sp.get_mut(si).and_then(|s| s.points.get_mut(pi)) else {
                return;
            };
            match part {
                PointPart::Anchor => {
                    let d = local - pt.p;
                    pt.p = local;
                    if let Some(h) = &mut pt.h_in {
                        *h = *h + d;
                    }
                    if let Some(h) = &mut pt.h_out {
                        *h = *h + d;
                    }
                }
                PointPart::In | PointPart::Out => {
                    let (this, other) = if part == PointPart::In {
                        (&mut pt.h_in, &mut pt.h_out)
                    } else {
                        (&mut pt.h_out, &mut pt.h_in)
                    };
                    *this = Some(local);
                    if !alt && let Some(o) = other {
                        let len = o.dist(pt.p);
                        let dir = (pt.p - local).normalized();
                        *o = pt.p + dir * len;
                    }
                }
            }
        });
    }

    /// Delete the selected path point (the whole layer when too few remain).
    pub fn delete_path_point(&mut self) -> bool {
        let Some(pe) = self.path_edit.clone() else {
            return false;
        };
        let Some((si, pi)) = pe.selected else {
            return false;
        };
        self.begin("Delete point");
        self.with_path(pe.id, |sp| {
            if let Some(s) = sp.get_mut(si) {
                if pi < s.points.len() {
                    s.points.remove(pi);
                }
                if s.points.len() < 2 {
                    sp.remove(si);
                }
            }
        });
        let empty = matches!(
            self.doc.get(pe.id).map(|n| &n.kind),
            Some(NodeKind::Path { subpaths, .. }) if subpaths.is_empty()
        );
        if empty {
            self.tx().remove(pe.id);
            self.path_edit = None;
            self.selection.clear();
        } else if let Some(p) = &mut self.path_edit {
            p.selected = None;
        }
        self.commit();
        true
    }

    /// Toggle a point between corner and smooth (double-click a point).
    pub fn toggle_point_smooth(&mut self, si: usize, pi: usize) {
        let Some(pe) = self.path_edit.clone() else {
            return;
        };
        self.begin("Toggle point");
        self.with_path(pe.id, |sp| {
            let Some(s) = sp.get_mut(si) else { return };
            let n = s.points.len();
            let prev = s.points[(pi + n - 1) % n].p;
            let next = s.points[(pi + 1) % n].p;
            let pt = &mut s.points[pi];
            if pt.h_in.is_some() || pt.h_out.is_some() {
                pt.h_in = None;
                pt.h_out = None;
            } else {
                let dir = (next - prev).normalized();
                let l = (prev.dist(next) / 6.0).max(4.0);
                pt.h_in = Some(pt.p - dir * l);
                pt.h_out = Some(pt.p + dir * l);
            }
        });
        self.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Contours {
        vec![vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]]
    }

    fn area(c: &Contours) -> f64 {
        c.iter()
            .map(|p| {
                let n = p.len();
                (0..n)
                    .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
                    .sum::<f64>()
                    * 0.5
            })
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn boolean_areas() {
        let a = square(0.0, 0.0, 10.0);
        let b = square(5.0, 5.0, 10.0);
        assert!((area(&boolean(BoolOp::Union, &[a.clone(), b.clone()])) - 175.0).abs() < 1e-6);
        assert!((area(&boolean(BoolOp::Intersect, &[a.clone(), b.clone()])) - 25.0).abs() < 1e-6);
        assert!((area(&boolean(BoolOp::Subtract, &[a.clone(), b.clone()])) - 75.0).abs() < 1e-6);
        let x = boolean(BoolOp::Exclude, &[a, b]);
        let total: f64 = x.iter().map(|c| area(&vec![c.clone()])).sum();
        assert!((total - 150.0).abs() < 1e-6, "{total}");
    }
}
