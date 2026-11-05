//! Editor side of prototyping: entering and leaving the preview, routing input to it, and drawing
//! links on the canvas by dragging from a layer's knob.

use super::shortcuts::{self, Action};
use super::{Editor, Gesture, InputEvent};
use crate::geom::{Rect, V2, v2};
use crate::model::{Interaction, NodeId, ProtoAction, Target, Transition, Trigger};
use crate::preview::Preview;

/// A link target found in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub page: String,
    pub page_index: usize,
    /// The top-level frame holding the target: the screen it opens.
    pub frame: NodeId,
    /// The target itself, which may be a section inside the frame.
    pub node: NodeId,
}

/// How far right of its layer the link knob sits, so it clears the resize handle.
const KNOB_GAP: f64 = 20.0;

impl Editor {
    /// Find what a link on `from_page` points at. `None` when it was deleted, or isn't inside a
    /// frame (only frames can be shown as screens).
    pub fn resolve_target(&self, from_page: &str, t: &Target) -> Option<ResolvedTarget> {
        let page = t.page.as_deref().unwrap_or(from_page);
        let page_index = self.pages.iter().position(|p| p.id == page)?;
        let doc = self.page_doc(page_index);
        doc.get(t.node)?;
        let frame = doc.ancestors(t.node).last().copied().unwrap_or(t.node);
        doc.get(frame).filter(|n| n.is_frame())?;
        Some(ResolvedTarget {
            page: page.to_string(),
            page_index,
            frame,
            node: t.node,
        })
    }

    /// Where a preview starts: the selection's frame, else the frame in the middle of the view,
    /// else the page's first frame.
    fn preview_start_frame(&self) -> Option<NodeId> {
        let top = |id: NodeId| self.doc.ancestors(id).last().copied().unwrap_or(id);
        let is_frame = |id: &NodeId| self.doc.get(*id).is_some_and(|n| n.is_frame() && n.visible);
        if let Some(f) = self.selection.first().map(|s| top(*s)).filter(is_frame) {
            return Some(f);
        }
        let center = self.camera.screen_to_world(self.viewport * 0.5);
        let frames: Vec<NodeId> = self
            .doc
            .root
            .iter()
            .copied()
            .filter(|i| is_frame(i))
            .collect();
        frames
            .iter()
            .rev()
            .find(|id| {
                self.doc
                    .world_rect(**id)
                    .is_some_and(|r| r.contains(center))
            })
            .or(frames.first())
            .copied()
    }

    /// Start playing the prototype. `at` picks the frame to start on.
    pub fn start_preview(&mut self, at: Option<(usize, NodeId)>) -> bool {
        self.cancel_gesture();
        self.end_text_edit();
        self.finish_pen();
        self.path_edit = None;
        self.commit();
        let start = match at {
            Some((page, node)) => Some((page, node)),
            None => self.preview_start_frame().map(|f| (self.active_page, f)),
        };
        let Some((page, frame)) = start else {
            self.status = "Nothing to preview: this page has no frames".into();
            return false;
        };
        let id = self.pages[page].id.clone();
        self.status.clear();
        self.hover = None;
        self.preview = Some(Box::new(Preview::new(self, &id, frame, self.now_ms)));
        self.revision += 1;
        true
    }

    pub fn stop_preview(&mut self) {
        if self.preview.take().is_some() {
            self.revision += 1;
        }
    }

    /// Run `f` on the preview, which is lent out of the editor so `f` can read the pages and queue
    /// requests. Leaves the preview if `f` asked to.
    pub fn with_preview<R>(&mut self, f: impl FnOnce(&mut Preview, &mut Editor) -> R) -> Option<R> {
        let mut p = self.preview.take()?;
        let r = f(&mut p, self);
        match p.exit.take() {
            Some(why) => {
                if let Some(why) = why {
                    self.status = why;
                }
                self.revision += 1;
            }
            None => self.preview = Some(p),
        }
        Some(r)
    }

    /// Bring the preview up to date before drawing (the window may have just resized).
    pub fn preview_sync(&mut self) {
        self.with_preview(|p, ed| p.sync(ed));
    }

    /// The pointer went off the canvas. Returns whether that changed anything.
    pub fn preview_pointer_left(&mut self) -> bool {
        self.preview.as_mut().is_some_and(|p| p.pointer_left())
    }

    /// Every event goes here while previewing, so nothing can reach the document by accident.
    pub(super) fn preview_input(&mut self, ev: InputEvent) {
        if let InputEvent::Modifiers(m) = ev {
            self.mods = m;
            return;
        }
        if let InputEvent::KeyDown { key, repeat } = &ev
            && let Some(a) = shortcuts::lookup(key, self.mods)
            && matches!(a, Action::TogglePreview | Action::ToggleUi | Action::Save)
        {
            if !repeat {
                self.run(a);
            }
            return;
        }
        // ⌘/Ctrl chords aren't for the prototype, even when the same key alone means something.
        if matches!(ev, InputEvent::KeyDown { .. }) && self.mods.command() {
            return;
        }
        self.with_preview(|p, ed| p.handle(ed, ev));
    }

    // ------------------------------------------------------------------ the link knob

    /// Screen position of the knob for dragging out a link from the selected layer.
    pub fn proto_knob(&self) -> Option<V2> {
        if !self.proto_mode || self.tool != super::Tool::Select || self.text_edit.is_some() {
            return None;
        }
        let [id] = self.selection[..] else {
            return None;
        };
        let r = self.camera.world_rect_to_screen(&self.doc.world_rect(id)?);
        Some(v2(r.right() + KNOB_GAP, r.center().y))
    }

    pub(super) fn proto_knob_down(&mut self, pos: V2) -> bool {
        let Some(k) = self.proto_knob().filter(|k| k.dist(pos) <= 10.0) else {
            return false;
        };
        let _ = k;
        self.gesture = Gesture::ProtoLink {
            from: self.selection[0],
        };
        true
    }

    /// The layer a link is being dragged from, for the overlay.
    pub fn proto_link_source(&self) -> Option<NodeId> {
        match self.gesture {
            Gesture::ProtoLink { from } => Some(from),
            _ => None,
        }
    }

    /// What dropping a link from `from` at `world` would link to: another screen, or a section
    /// (or the top) of the screen it's already on.
    fn proto_drop_target(&self, from: NodeId, world: V2) -> Option<NodeId> {
        let hit = self.frame_at(world, &[from])?;
        let root = self.doc.ancestors(hit).last().copied().unwrap_or(hit);
        let own_root = self.doc.ancestors(from).last().copied().unwrap_or(from);
        Some(if root == own_root { hit } else { root })
    }

    pub(super) fn proto_link_drag(&mut self, from: NodeId, world: V2) {
        self.bind_hover = self.proto_drop_target(from, world);
    }

    pub(super) fn proto_link_up(&mut self, from: NodeId, world: V2) {
        self.bind_hover = None;
        let Some(target) = self.proto_drop_target(from, world) else {
            self.status = "Drop on a frame to link to it".into();
            return;
        };
        self.edit("Add link", |tx| {
            let Some(n) = tx.node_mut(from) else { return };
            n.edit_proto(|p| {
                let link = Interaction {
                    trigger: Trigger::Click,
                    action: ProtoAction::Navigate {
                        target: Target::same_page(target),
                        transition: Transition::default(),
                    },
                };
                // A new link replaces the layer's old click link rather than firing alongside it.
                let old = p.interactions.iter_mut().find(|i| {
                    i.trigger == Trigger::Click && matches!(i.action, ProtoAction::Navigate { .. })
                });
                match old {
                    Some(i) => *i = link,
                    None => p.interactions.push(link),
                }
            });
        });
        self.mark_overrides(&[from], "prototype");
        self.commit();
    }
}

/// Curve from a layer's right edge to a target's left edge, for drawing links on the canvas.
pub fn link_curve(a: V2, b: V2) -> Vec<V2> {
    let bend = ((b.x - a.x).abs() * 0.5).clamp(40.0, 240.0);
    let (c1, c2) = (a + v2(bend, 0.0), b - v2(bend, 0.0));
    (0..=32)
        .map(|i| {
            let t = i as f64 / 32.0;
            let u = 1.0 - t;
            a * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + b * (t * t * t)
        })
        .collect()
}

/// Where a link ending on `r` comes in.
pub fn link_end(r: Rect) -> V2 {
    v2(r.x, r.y + r.h.min(120.0) * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_curve_runs_between_its_ends() {
        let (a, b) = (v2(0.0, 0.0), v2(300.0, 100.0));
        let c = link_curve(a, b);
        assert_eq!(c[0], a);
        assert!(c.last().unwrap().dist(b) < 1e-9);
    }
}
