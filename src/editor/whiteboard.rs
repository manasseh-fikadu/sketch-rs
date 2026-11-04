//! Whiteboard tools: arrows that bind to shapes, quick-connect, sticky notes, the laser
//! pointer and the hand-drawn style toggle.
//!
//! Laser decay constants and the sticky-note behavior follow Excalidraw
//! (packages/excalidraw/laserTrails.ts, packages/laser-pointer; Copyright (c) 2020/2023
//! Excalidraw, MIT). See THIRD_PARTY.md.

use super::{Editor, Gesture, Tool};
use crate::connector::{self, BIND_DISTANCE_PX};
use crate::geom::{Rect, V2, v2};
use crate::history::Tx;
use crate::model::rough::Rough;
use crate::model::{Binding, Head, Node, NodeId, NodeKind, Route};

/// Laser trail points fade over this long (ms), as in Excalidraw.
pub const LASER_DECAY_MS: f64 = 1000.0;
/// Only the newest points are drawn, tapering to nothing.
pub const LASER_MAX_POINTS: usize = 50;
/// Distance between a shape and the copy quick-connect creates.
pub const QUICK_GAP: f64 = 80.0;

/// Directions of the quick-connect knobs: right, down, left, up.
pub const QUICK_DIRS: [V2; 4] = [v2(1.0, 0.0), v2(0.0, 1.0), v2(-1.0, 0.0), v2(0.0, -1.0)];

/// Recompute a connector's points from its bindings and store them (node-local).
pub fn update_connector(tx: &mut Tx, id: NodeId) {
    // Drop bindings to layers that no longer exist.
    let stale = match tx.doc().get(id).map(|n| &n.kind) {
        Some(NodeKind::Connector { start, end, .. }) => (
            start.is_some_and(|b| !tx.doc().nodes.contains_key(&b.node)),
            end.is_some_and(|b| !tx.doc().nodes.contains_key(&b.node)),
        ),
        _ => return,
    };
    if stale.0 || stale.1 {
        let n = tx.node_mut(id).unwrap();
        if let NodeKind::Connector { start, end, .. } = &mut n.kind {
            if stale.0 {
                *start = None;
            }
            if stale.1 {
                *end = None;
            }
        }
    }
    let Some(pts) = connector::resolve(tx.doc(), id) else {
        return;
    };
    set_world_points(tx, id, &pts);
}

/// Store world points into a connector, refitting its box.
pub fn set_world_points(tx: &mut Tx, id: NodeId, pts: &[V2]) {
    let Some(n) = tx.doc().get(id) else { return };
    let o = tx.doc().parent_origin(n.parent);
    let b = Rect::union_all(
        &pts.iter()
            .map(|p| Rect::new(p.x, p.y, 0.0, 0.0))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let local: Vec<V2> = pts.iter().map(|p| *p - b.min()).collect();
    let rect = b.translate(V2::ZERO - o);
    let same =
        matches!(&n.kind, NodeKind::Connector { points, .. } if *points == local) && n.rect == rect;
    if same {
        return;
    }
    let n = tx.node_mut(id).unwrap();
    n.rect = rect;
    if let NodeKind::Connector { points, .. } = &mut n.kind {
        *points = local;
    }
}

impl Editor {
    fn bind_dist(&self) -> f64 {
        BIND_DISTANCE_PX / self.camera.zoom
    }

    /// World points of a connector.
    pub fn connector_points(&self, id: NodeId) -> Option<Vec<V2>> {
        let n = self.doc.get(id)?;
        let wr = self.doc.world_rect(id)?;
        match &n.kind {
            NodeKind::Connector { points, .. } => {
                Some(points.iter().map(|p| wr.min() + *p).collect())
            }
            _ => None,
        }
    }

    /// Connectors that must be re-routed because something in `changed` moved or resized.
    pub(super) fn reroute_connectors(&mut self, changed: &std::collections::HashSet<NodeId>) {
        let affected: Vec<NodeId> = self
            .doc
            .nodes
            .values()
            .filter_map(|n| match &n.kind {
                NodeKind::Connector { start, end, .. } => {
                    let hit = |b: &Option<Binding>| {
                        b.is_some_and(|b| {
                            changed.contains(&b.node)
                                || !self.doc.nodes.contains_key(&b.node)
                                || self
                                    .doc
                                    .ancestors(b.node)
                                    .iter()
                                    .any(|a| changed.contains(a))
                        })
                    };
                    (hit(start)
                        || hit(end)
                        || (changed.contains(&n.id) && (start.is_some() || end.is_some())))
                    .then_some(n.id)
                }
                _ => None,
            })
            .collect();
        if affected.is_empty() {
            return;
        }
        let mut tx = self.tx();
        for id in affected {
            update_connector(&mut tx, id);
        }
    }

    pub(super) fn arrow_down(&mut self, world: V2) {
        self.end_text_edit();
        self.begin("Add arrow");
        let start_target = connector::bind_target(&self.doc, world, self.bind_dist(), &[]);
        let start_binding = start_target.map(|t| Binding {
            node: t,
            fixed: connector::fixed_point(world, self.doc.world_rect(t).unwrap()),
            inside: false,
        });
        let route = self.arrow_route;
        let rough = self.sketch;
        let id = {
            let mut tx = self.tx();
            let id = tx.alloc_id();
            let mut n = Node::new(
                id,
                NodeKind::Connector {
                    points: vec![V2::ZERO, V2::ZERO],
                    start: start_binding,
                    end: None,
                    route,
                    start_head: Head::None,
                    end_head: Head::Arrow,
                },
                Rect::new(world.x, world.y, 0.0, 0.0),
            );
            n.style.rough = rough.map(|r| Rough {
                seed: (id.0 as u32).wrapping_mul(2654435761) | 1,
                ..r
            });
            tx.insert(n, None, None)
        };
        self.selection = vec![id];
        self.gesture = Gesture::ConnectorEnd {
            id,
            end: 1,
            start: world,
        };
    }

    /// Drag a connector end to `world`, binding to whatever shape is under it.
    pub(super) fn drag_connector_end(&mut self, id: NodeId, end: usize, world: V2) {
        let Some(mut pts) = self.connector_points(id) else {
            return;
        };
        let target = connector::bind_target(&self.doc, world, self.bind_dist(), &[id])
            .filter(|t| !self.doc.is_ancestor(id, *t));
        self.bind_hover = target;
        let idx = if end == 0 { 0 } else { pts.len() - 1 };
        let mut p = world;
        if self.mods.shift && target.is_none() && pts.len() == 2 {
            // 45° steps from the other end.
            let other = pts[1 - idx];
            let d = p - other;
            let a = (d.y.atan2(d.x) / std::f64::consts::FRAC_PI_4).round()
                * std::f64::consts::FRAC_PI_4;
            p = other + v2(a.cos(), a.sin()) * d.len();
        }
        pts[idx] = p;
        let binding = target.map(|t| Binding {
            node: t,
            fixed: connector::fixed_point(world, self.doc.world_rect(t).unwrap()),
            inside: false,
        });
        let mut tx = self.tx();
        if let Some(n) = tx.node_mut(id)
            && let NodeKind::Connector { start, end: e, .. } = &mut n.kind
        {
            if end == 0 {
                *start = binding;
            } else {
                *e = binding;
            }
        }
        set_world_points(&mut tx, id, &pts);
        update_connector(&mut tx, id);
    }

    pub(super) fn connector_end_up(&mut self, id: NodeId, start: V2, world: V2) {
        self.bind_hover = None;
        let created = self
            .rec
            .as_ref()
            .is_some_and(|(r, _)| r.before(id) == Some(&None));
        // A click without a drag makes a default-length arrow.
        if created && (world - start).len() * self.camera.zoom < 8.0 {
            let pts = vec![start, start + v2(120.0, 0.0)];
            let mut tx = self.tx();
            set_world_points(&mut tx, id, &pts);
            update_connector(&mut tx, id);
        }
        self.commit();
        if created {
            self.tool = Tool::Select;
        }
    }

    /// Screen-space handles of the selected connector: (end index, position). Index 2 is the
    /// midpoint "bend" handle of a two-point connector.
    pub fn connector_handles(&self) -> Vec<(usize, V2)> {
        let [id] = self.selection[..] else {
            return vec![];
        };
        let Some(pts) = self.connector_points(id) else {
            return vec![];
        };
        let s = |p: V2| self.camera.world_to_screen(p);
        let mut out = vec![(0, s(pts[0])), (1, s(*pts.last().unwrap()))];
        let elbow = matches!(
            self.doc.get(id).map(|n| &n.kind),
            Some(NodeKind::Connector {
                route: Route::Elbow,
                ..
            })
        );
        if pts.len() == 2 && !elbow {
            out.push((2, s((pts[0] + pts[1]) * 0.5)));
        }
        out
    }

    /// Start dragging a connector handle under `pos`, if any.
    pub(super) fn connector_handle_down(&mut self, pos: V2, world: V2) -> bool {
        let Some(&(end, _)) = self
            .connector_handles()
            .iter()
            .find(|(_, p)| p.dist(pos) <= 7.0)
        else {
            return false;
        };
        let id = self.selection[0];
        if end == 2 {
            // Bend: insert a middle point and make the arrow curved.
            self.begin("Bend arrow");
            let mut pts = self.connector_points(id).unwrap();
            pts.insert(1, world);
            let mut tx = self.tx();
            if let Some(n) = tx.node_mut(id)
                && let NodeKind::Connector { route, .. } = &mut n.kind
            {
                *route = Route::Curved;
            }
            set_world_points(&mut tx, id, &pts);
            self.gesture = Gesture::ConnectorMid { id };
            return true;
        }
        self.begin("Move arrow end");
        self.gesture = Gesture::ConnectorEnd {
            id,
            end,
            start: world,
        };
        true
    }

    pub(super) fn drag_connector_mid(&mut self, id: NodeId, world: V2) {
        let Some(mut pts) = self.connector_points(id) else {
            return;
        };
        if pts.len() >= 3 {
            pts[1] = world;
        }
        let mut tx = self.tx();
        set_world_points(&mut tx, id, &pts);
        update_connector(&mut tx, id);
    }

    // ---------------------------------------------------------------------- quick connect

    /// Screen positions of the quick-connect knobs around a single selected shape.
    pub fn quick_knobs(&self) -> Vec<(usize, V2)> {
        if self.tool != Tool::Select || self.text_edit.is_some() || self.is_busy() {
            return vec![];
        }
        let [id] = self.selection[..] else {
            return vec![];
        };
        let Some(n) = self.doc.get(id) else {
            return vec![];
        };
        if !connector::bindable(n) || n.is_frame() || matches!(n.kind, NodeKind::Text { .. }) {
            return vec![];
        }
        let Some(r) = self.doc.world_rect(id) else {
            return vec![];
        };
        let sr = self.camera.world_rect_to_screen(&r);
        if sr.w < 24.0 || sr.h < 24.0 {
            return vec![];
        }
        let c = sr.center();
        QUICK_DIRS
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let off = v2(sr.w * 0.5 + 22.0, sr.h * 0.5 + 22.0);
                (i, c + v2(d.x * off.x, d.y * off.y))
            })
            .collect()
    }

    /// Duplicate the selected shape in direction `dir` and connect the two with an arrow.
    pub fn quick_connect(&mut self, dir: usize) -> Option<NodeId> {
        let [src] = self.selection[..] else {
            return None;
        };
        let n = self.doc.get(src)?.clone();
        let wr = self.doc.world_rect(src)?;
        let d = QUICK_DIRS[dir];
        let off = v2(d.x * (wr.w + QUICK_GAP), d.y * (wr.h + QUICK_GAP));
        let parent = n.parent;
        let o = self.doc.parent_origin(parent);
        let route = self.arrow_route;
        let rough = n.style.rough;
        let (copy, arrow) = self.edit("Quick connect", |tx| {
            let cid = tx.alloc_id();
            let mut c = n.clone();
            c.id = cid;
            c.children.clear();
            c.instance = None;
            c.component = false;
            if let Some(t) = c.kind.text_mut() {
                t.clear();
            }
            c.text_style = Default::default();
            c.rect = wr.translate(off - o);
            if let Some(r) = &mut c.style.rough {
                r.seed = r.seed.wrapping_add(7919);
            }
            tx.insert(c, parent, None);
            let aid = tx.alloc_id();
            let fixed_a = v2(0.5 + d.x * 0.5, 0.5 + d.y * 0.5);
            let fixed_b = v2(0.5 - d.x * 0.5, 0.5 - d.y * 0.5);
            let fix = |v: V2| connector::fixed_point(v2(v.x, v.y), Rect::new(0.0, 0.0, 1.0, 1.0));
            let mut a = Node::new(
                aid,
                NodeKind::Connector {
                    points: vec![V2::ZERO, V2::ZERO],
                    start: Some(Binding {
                        node: src,
                        fixed: fix(fixed_a),
                        inside: false,
                    }),
                    end: Some(Binding {
                        node: cid,
                        fixed: fix(fixed_b),
                        inside: false,
                    }),
                    route,
                    start_head: Head::None,
                    end_head: Head::Arrow,
                },
                Rect::default(),
            );
            a.style.rough = rough.map(|r| Rough {
                seed: r.seed.wrapping_add(31),
                ..r
            });
            tx.insert(a, None, None);
            update_connector(tx, aid);
            (cid, aid)
        });
        let _ = arrow;
        self.selection = vec![copy];
        Some(copy)
    }

    // ---------------------------------------------------------------------- sticky notes

    pub(super) fn sticky_down(&mut self, world: V2) {
        self.end_text_edit();
        self.begin("Add sticky");
        let size = 200.0;
        let parent = self.frame_at(world, &[]);
        let o = self.doc.parent_origin(parent);
        let color = self.sticky_color;
        let rough = self.sketch;
        let id = {
            let mut tx = self.tx();
            let id = tx.alloc_id();
            let mut n = Node::new(
                id,
                NodeKind::Sticky {
                    text: String::new(),
                    font_size: 24.0,
                },
                Rect::new(
                    (world.x - size * 0.5).round() - o.x,
                    (world.y - size * 0.5).round() - o.y,
                    size,
                    size,
                ),
            );
            n.style.set_fill(Some(color));
            let _ = rough;
            tx.insert(n, parent, None)
        };
        self.selection = vec![id];
        self.text_edit = Some(super::TextEdit {
            id,
            caret: 0,
            anchor: None,
        });
        self.tool = Tool::Select;
    }

    // ---------------------------------------------------------------------- laser

    pub(super) fn laser_point(&mut self, world: V2, time_ms: f64) {
        if self.laser.last().is_some_and(|(p, _)| *p == world) {
            return;
        }
        // Streamline like Excalidraw's laser (0.4).
        let p = match self.laser.last() {
            Some((last, t)) if time_ms - t < LASER_DECAY_MS => last.lerp(world, 0.6),
            _ => world,
        };
        self.laser.push((p, time_ms));
        self.now_ms = self.now_ms.max(time_ms);
    }

    /// Points still visible at `now`, with their size factor (0..1).
    pub fn laser_trail(&self, now: f64) -> Vec<(V2, f64)> {
        let n = self.laser.len();
        let ease = |k: f64| 1.0 - (1.0 - k.clamp(0.0, 1.0)).powi(4);
        self.laser
            .iter()
            .enumerate()
            .filter_map(|(i, (p, t))| {
                let tf = (1.0 - (now - t) / LASER_DECAY_MS).max(0.0);
                let lf = (LASER_MAX_POINTS as f64 - ((n - i) as f64).min(LASER_MAX_POINTS as f64))
                    / LASER_MAX_POINTS as f64;
                let k = ease(tf).min(ease(lf + 1.0 / LASER_MAX_POINTS as f64));
                (k > 0.0).then_some((*p, k))
            })
            .collect()
    }

    /// Whether something on the canvas is animating (the laser trail is fading).
    pub fn animating(&self, now: f64) -> bool {
        self.laser
            .last()
            .is_some_and(|(_, t)| now - t < LASER_DECAY_MS)
    }

    /// Drop faded laser points (call with the current time before drawing).
    pub fn tick(&mut self, now: f64) {
        self.now_ms = now;
        self.laser.retain(|(_, t)| now - t < LASER_DECAY_MS);
    }

    // ---------------------------------------------------------------------- sketch style

    /// Toggle the hand-drawn style on the selection (or the default for new shapes).
    pub fn toggle_sketch(&mut self) {
        let ids = self.top_selection();
        if ids.is_empty() {
            self.sketch = match self.sketch {
                Some(_) => None,
                None => Some(Rough::default()),
            };
            self.status = format!(
                "Hand-drawn style {} for new shapes",
                if self.sketch.is_some() { "on" } else { "off" }
            );
            return;
        }
        let on = ids
            .iter()
            .any(|id| self.doc.get(*id).is_some_and(|n| n.style.rough.is_none()));
        self.edit(if on { "Hand-drawn" } else { "Clean lines" }, |tx| {
            for id in ids {
                let Some(n) = tx.node_mut(id) else { continue };
                n.style.rough = on.then(|| Rough {
                    seed: (id.0 as u32).wrapping_mul(2654435761) | 1,
                    ..Rough::default()
                });
            }
        });
    }
}
