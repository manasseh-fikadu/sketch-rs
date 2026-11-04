//! Smaller editing commands: sibling navigation, flipping, copy/paste of properties, and
//! UI-level requests the chrome handles (palette, rename, hide UI).

use super::Editor;
use crate::geom::V2;
use crate::history::Tx;
use crate::model::{NodeId, NodeKind, Style, TextStyle};

/// Requests for the egui chrome (consumed by `ui::draw`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    TogglePalette,
    Rename(NodeId),
    ToggleUi,
    ToggleHelp,
    ToggleGrid,
}

impl Editor {
    /// Tab / Shift+Tab: the next or previous sibling of the selection (in paint order).
    pub fn select_sibling(&mut self, forward: bool) {
        let first = self.selection.first().copied();
        let parent = first.and_then(|id| self.doc.get(id)).and_then(|n| n.parent);
        let list: Vec<NodeId> = self
            .doc
            .children(parent)
            .iter()
            .copied()
            .filter(|id| self.doc.get(*id).is_some_and(|n| n.visible))
            .collect();
        if list.is_empty() {
            return;
        }
        let next = match first.and_then(|f| list.iter().position(|x| *x == f)) {
            Some(i) if forward => list[(i + list.len() - 1) % list.len()],
            Some(i) => list[(i + 1) % list.len()],
            None => *list.last().unwrap(),
        };
        self.end_text_edit();
        self.selection = vec![next];
    }

    pub fn select_parent(&mut self) {
        let parents: Vec<NodeId> = self
            .selection
            .iter()
            .filter_map(|id| self.doc.get(*id).and_then(|n| n.parent))
            .collect();
        if !parents.is_empty() {
            self.selection = super::dedup(parents);
        }
    }

    /// Mirror the selection horizontally (axis 0) or vertically (axis 1) within its bounds.
    pub fn flip(&mut self, axis: usize) {
        let ids: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| !self.doc.is_effectively_locked(*id))
            .collect();
        let Some(b) = self.selection_bounds() else {
            return;
        };
        self.edit(
            if axis == 0 {
                "Flip horizontal"
            } else {
                "Flip vertical"
            },
            |tx| {
                for id in ids {
                    let Some(wr) = tx.doc().world_rect(id) else {
                        continue;
                    };
                    // Mirror the box within the selection bounds…
                    let d = if axis == 0 {
                        crate::geom::v2(b.x + b.right() - wr.right() - wr.x, 0.0)
                    } else {
                        crate::geom::v2(0.0, b.y + b.bottom() - wr.bottom() - wr.y)
                    };
                    let n = tx.node_mut(id).unwrap();
                    n.rect = n.rect.translate(d);
                    // …and the geometry within the box.
                    flip_node(tx, id, axis);
                }
            },
        );
    }

    pub fn copy_props(&mut self) {
        if let Some(n) = self.selection.first().and_then(|id| self.doc.get(*id)) {
            self.clipboard_style = Some((n.style.clone(), n.text_style.clone()));
            self.status = "Copied properties".into();
        }
    }

    pub fn paste_props(&mut self) {
        let Some((style, ts)) = self.clipboard_style.clone() else {
            return;
        };
        let ids = self.top_selection();
        self.edit("Paste properties", |tx| {
            for id in &ids {
                let Some(n) = tx.node_mut(*id) else { continue };
                n.style = Style { ..style.clone() };
                if n.kind.text().is_some() {
                    n.text_style = TextStyle {
                        spans: Vec::new(),
                        ..ts.clone()
                    };
                }
            }
        });
        self.mark_overrides(&ids, "fills");
        for id in ids {
            self.fit_text(id);
        }
        self.commit();
    }
}

fn mirror(v: &mut V2, w: f64, h: f64, axis: usize) {
    if axis == 0 {
        v.x = w - v.x;
    } else {
        v.y = h - v.y;
    }
}

/// Mirror a node's own geometry and its children inside its box.
fn flip_node(tx: &mut Tx, id: NodeId, axis: usize) {
    let Some(n) = tx.doc().get(id) else { return };
    let (w, h) = (n.rect.w, n.rect.h);
    let kids = n.children.clone();
    let node = tx.node_mut(id).unwrap();
    match &mut node.kind {
        NodeKind::Path { subpaths, .. } => {
            for sp in subpaths.iter_mut() {
                for p in sp.points.iter_mut() {
                    mirror(&mut p.p, w, h, axis);
                    if let Some(q) = &mut p.h_in {
                        mirror(q, w, h, axis);
                    }
                    if let Some(q) = &mut p.h_out {
                        mirror(q, w, h, axis);
                    }
                }
            }
        }
        NodeKind::Line { a, b } => {
            mirror(a, w, h, axis);
            mirror(b, w, h, axis);
        }
        NodeKind::Connector { points, .. } => {
            for p in points.iter_mut() {
                mirror(p, w, h, axis);
            }
        }
        NodeKind::Stroke { points, .. } => {
            for p in points.iter_mut() {
                if axis == 0 {
                    p.x = w - p.x;
                } else {
                    p.y = h - p.y;
                }
            }
        }
        _ => {}
    }
    for k in kids {
        let Some(c) = tx.doc().get(k) else { continue };
        let r = c.rect;
        let n = tx.node_mut(k).unwrap();
        if axis == 0 {
            n.rect.x = w - r.x - r.w;
        } else {
            n.rect.y = h - r.y - r.h;
        }
        flip_node(tx, k, axis);
    }
}
