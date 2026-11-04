//! Structural editing: groups, components and instances, auto layout, alignment and the
//! post-edit fixup pass that keeps derived state (group bounds, layout, instances) in sync.

use super::Editor;
use crate::geom::{Rect, V2, v2};
use crate::history::Tx;
use crate::layout;
use crate::model::{AutoLayout, Direction, InstanceLink, Node, NodeId, NodeKind, Sizing};
use std::collections::{BTreeSet, HashSet};

/// Property groups an instance can override (and that sync copies from the main component).
pub const OVERRIDE_KEYS: [&str; 11] = [
    "fills", "stroke", "radius", "opacity", "text", "font", "visible", "name", "size", "position",
    "layout",
];

/// Map an inspector field name to the override key it changes.
pub fn override_key(field: &str) -> &'static str {
    match field {
        "x" | "y" | "position" => "position",
        "width" | "height" | "size" => "size",
        "fill" | "fills" | "color" => "fills",
        "stroke" | "stroke width" | "stroke align" | "dash" | "cap" | "join" => "stroke",
        "radius" | "radii" => "radius",
        "opacity" | "blend" => "opacity",
        "text" => "text",
        "font size" | "font" | "bold" | "italic" | "align" | "line height" | "letter spacing" => {
            "font"
        }
        "visible" => "visible",
        "name" => "name",
        _ => "layout",
    }
}

/// Copy the properties of `main` onto `inst` except overridden ones.
fn copy_props(main: &Node, inst: &mut Node, overrides: &[String], is_root: bool) {
    let ov = |k: &str| overrides.iter().any(|o| o == k);
    let s = &main.style;
    if !ov("fills") {
        inst.style.fills = s.fills.clone();
    }
    if !ov("stroke") {
        inst.style.stroke = s.stroke;
        inst.style.stroke_width = s.stroke_width;
        inst.style.stroke_align = s.stroke_align;
        inst.style.dash = s.dash.clone();
        inst.style.cap = s.cap;
        inst.style.join = s.join;
    }
    if !ov("radius") {
        inst.style.corner_radius = s.corner_radius;
        inst.style.radii = s.radii;
    }
    if !ov("opacity") {
        inst.style.opacity = s.opacity;
        inst.style.blend = s.blend;
    }
    inst.style.rough = s.rough;
    match (&main.kind, &mut inst.kind) {
        (
            NodeKind::Text {
                text: mt,
                font_size: mf,
                auto_width: ma,
                align: mal,
            },
            NodeKind::Text {
                text,
                font_size,
                auto_width,
                align,
            },
        ) => {
            if !ov("text") {
                *text = mt.clone();
            }
            if !ov("font") {
                *font_size = *mf;
                *auto_width = *ma;
                *align = *mal;
            }
        }
        (mk, ik) if !matches!(mk, NodeKind::Text { .. }) => *ik = mk.clone(),
        _ => {}
    }
    if !ov("font") {
        inst.text_style = main.text_style.clone();
    }
    if !ov("visible") {
        inst.visible = main.visible;
    }
    if !ov("name") && !is_root {
        inst.name = main.name.clone();
    }
    if !ov("layout") {
        inst.layout = main.layout;
        inst.layout_child = main.layout_child;
    }
    if !is_root && !ov("position") {
        inst.rect.x = main.rect.x;
        inst.rect.y = main.rect.y;
    }
    if !ov("size") {
        inst.rect.w = main.rect.w;
        inst.rect.h = main.rect.h;
    }
}

/// Copy a subtree as a new instance subtree under `parent` (ids are fresh).
fn instantiate(
    tx: &mut Tx,
    main: NodeId,
    parent: Option<NodeId>,
    index: Option<usize>,
    root_rect: Option<Rect>,
) -> NodeId {
    let src = tx.doc().get(main).unwrap().clone();
    let id = tx.alloc_id();
    let mut n = src.clone();
    n.id = id;
    n.children = Vec::new();
    n.component = false;
    n.instance = Some(InstanceLink {
        main,
        overrides: Vec::new(),
    });
    if let Some(r) = root_rect {
        n.rect = r;
    }
    tx.insert(n, parent, index);
    for c in src.children {
        instantiate(tx, c, Some(id), None, None);
    }
    id
}

/// Bring one instance subtree in line with its main counterpart.
fn sync_node(tx: &mut Tx, inst: NodeId, is_root: bool) {
    let Some(n) = tx.doc().get(inst) else { return };
    let Some(link) = n.instance.clone() else {
        return;
    };
    let Some(main) = tx.doc().get(link.main).cloned() else {
        return;
    };
    let mut updated = n.clone();
    copy_props(&main, &mut updated, &link.overrides, is_root);
    if updated != *n {
        *tx.node_mut(inst).unwrap() = updated;
    }
    // Children: match by link; create missing, remove stale, follow main's order.
    let kids: Vec<NodeId> = tx.doc().get(inst).unwrap().children.clone();
    let mut by_main: Vec<(NodeId, NodeId)> = kids
        .iter()
        .filter_map(|k| {
            tx.doc()
                .get(*k)
                .and_then(|c| c.instance.as_ref())
                .map(|l| (l.main, *k))
        })
        .collect();
    for k in &kids {
        let stale = tx
            .doc()
            .get(*k)
            .and_then(|c| c.instance.as_ref())
            .is_none_or(|l| !main.children.contains(&l.main));
        if stale {
            tx.remove(*k);
            by_main.retain(|(_, x)| x != k);
        }
    }
    for (i, mc) in main.children.iter().enumerate() {
        match by_main.iter().find(|(m, _)| m == mc) {
            Some((_, k)) => {
                let k = *k;
                if tx.doc().children(Some(inst)).iter().position(|x| *x == k) != Some(i) {
                    tx.move_to_index(k, i);
                }
                sync_node(tx, k, false);
            }
            None => {
                instantiate(tx, *mc, Some(inst), Some(i), None);
            }
        }
    }
}

impl Editor {
    /// Keep derived structure consistent after edits in the open transaction: group bounds,
    /// auto layout, and instances of edited components. Runs inside the transaction, so it
    /// undoes together with the edit that caused it.
    pub fn fixup(&mut self) {
        let Some((rec, _)) = &self.rec else { return };
        let touched: Vec<NodeId> = rec.touched().to_vec();
        if touched.is_empty() {
            return;
        }
        let mut seeds: HashSet<NodeId> = HashSet::new();
        for id in &touched {
            if self.doc.nodes.contains_key(id) {
                seeds.insert(*id);
            }
            // Parents of removed or moved nodes.
            if let Some(Some(b)) = rec.before(*id)
                && let Some(p) = b.parent
                && self.doc.nodes.contains_key(&p)
            {
                seeds.insert(p);
            }
            if let Some(p) = self.doc.get(*id).and_then(|n| n.parent) {
                seeds.insert(p);
            }
        }
        let mut cands: HashSet<NodeId> = seeds.clone();
        for s in &seeds {
            cands.extend(self.doc.ancestors(*s));
        }
        let depth = |e: &Editor, id: NodeId| e.doc.ancestors(id).len();

        // 1. Groups, deepest first.
        let mut groups: Vec<NodeId> = cands
            .iter()
            .copied()
            .filter(|id| self.doc.get(*id).is_some_and(|n| n.is_group()))
            .collect();
        groups.sort_by_key(|g| std::cmp::Reverse(depth(self, *g)));
        // 2. Components touched (self or ancestor is a main component).
        let comps: Vec<NodeId> = cands
            .iter()
            .copied()
            .filter(|id| self.doc.get(*id).is_some_and(|n| n.component))
            .collect();
        {
            let mut tx = self.tx();
            for g in &groups {
                layout::normalize_group(&mut tx, *g);
            }
            for c in &comps {
                let instances: Vec<NodeId> = tx
                    .doc()
                    .nodes
                    .values()
                    .filter(|n| n.instance.as_ref().is_some_and(|l| l.main == *c))
                    .map(|n| n.id)
                    .collect();
                for i in instances {
                    sync_node(&mut tx, i, true);
                    cands.insert(i);
                }
            }
        }
        // 3. Auto layout from the outermost auto-layout ancestor of each candidate.
        let mut roots: BTreeSet<(usize, NodeId)> = BTreeSet::new();
        for id in cands.iter().copied() {
            let Some(n) = self.doc.get(id) else { continue };
            if n.layout.is_none()
                && n.parent
                    .and_then(|p| self.doc.get(p))
                    .is_none_or(|p| p.layout.is_none())
            {
                continue;
            }
            let mut top = if n.layout.is_some() {
                Some(id)
            } else {
                n.parent
            };
            let mut cur = top;
            while let Some(c) = cur {
                let cn = self.doc.get(c).unwrap();
                if cn.layout.is_some() {
                    top = Some(c);
                }
                // Climb while this frame sits in an auto-layout parent's flow.
                match cn.parent.and_then(|p| self.doc.get(p)) {
                    Some(p) if p.layout.is_some() && !cn.layout_child.absolute => cur = Some(p.id),
                    _ => break,
                }
            }
            if let Some(t) = top
                && !self.layout_suspended.contains(&t)
            {
                roots.insert((depth(self, t), t));
            }
        }
        if !roots.is_empty() {
            let mut tx = self.tx();
            for (_, r) in roots {
                layout::layout_frame(&mut tx, r);
            }
        }
        // 4. Arrows bound to anything that moved follow it.
        let touched: HashSet<NodeId> = self
            .rec
            .as_ref()
            .map(|(r, _)| r.touched().iter().copied().collect())
            .unwrap_or_default();
        self.reroute_connectors(&touched);
    }

    /// Record that `prop` was explicitly changed on nodes inside instances.
    pub fn mark_overrides(&mut self, ids: &[NodeId], field: &str) {
        let key = override_key(field);
        let mut tx = self.tx();
        for id in ids {
            let is_inst = tx.doc().get(*id).is_some_and(|n| n.instance.is_some());
            if is_inst
                && let Some(n) = tx.node_mut(*id)
                && let Some(l) = &mut n.instance
                && !l.overrides.iter().any(|o| o == key)
            {
                l.overrides.push(key.into());
            }
        }
    }

    // ---------------------------------------------------------------------- groups

    /// Group the selection (Cmd+G). Selected layers must share a parent.
    pub fn group_selection(&mut self) {
        let ids = self.top_selection();
        if ids.is_empty() {
            return;
        }
        let parent = self.doc.get(ids[0]).and_then(|n| n.parent);
        if ids
            .iter()
            .any(|id| self.doc.get(*id).and_then(|n| n.parent) != parent)
        {
            self.status = "Selection must share a parent".into();
            return;
        }
        let Some(b) = self.selection_bounds() else {
            return;
        };
        let index = ids
            .iter()
            .filter_map(|id| self.doc.children(parent).iter().position(|x| x == id))
            .max()
            .map(|i| i + 1);
        let g = self.edit("Group", |tx| {
            let gid = tx.alloc_id();
            let o = tx.doc().parent_origin(parent);
            let g = Node::new(gid, NodeKind::Group, b.translate(V2::ZERO - o));
            tx.insert(g, parent, index);
            let mut ordered = ids.clone();
            ordered.sort_by_key(|id| tx.doc().children(parent).iter().position(|x| x == id));
            for id in ordered {
                tx.reparent(id, Some(gid), None);
            }
            gid
        });
        self.selection = vec![g];
    }

    /// Ungroup selected groups (Cmd+Shift+G); frames lose their frame too, like Figma.
    pub fn ungroup_selection(&mut self) {
        let ids: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| {
                self.doc
                    .get(*id)
                    .is_some_and(|n| n.is_container() && !n.children.is_empty())
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut freed = Vec::new();
        self.edit("Ungroup", |tx| {
            for g in ids {
                let n = tx.doc().get(g).unwrap().clone();
                let idx = tx
                    .doc()
                    .children(n.parent)
                    .iter()
                    .position(|x| *x == g)
                    .unwrap_or(0);
                for (k, c) in n.children.iter().enumerate() {
                    tx.reparent(*c, n.parent, Some(idx + 1 + k));
                    freed.push(*c);
                }
                tx.remove(g);
            }
        });
        self.selection = freed;
    }

    // ---------------------------------------------------------------------- components

    /// Make the selection a main component (Cmd+Alt+K). A single frame is converted in
    /// place; anything else is wrapped in a new component frame.
    pub fn create_component(&mut self) {
        let ids = self.top_selection();
        if ids.is_empty() {
            return;
        }
        if let [id] = ids[..]
            && self
                .doc
                .get(id)
                .is_some_and(|n| n.is_frame() && n.instance.is_none())
        {
            self.edit("Create component", |tx| {
                tx.node_mut(id).unwrap().component = true;
            });
            return;
        }
        self.wrap_in_frame();
        if let [f] = self.selection[..] {
            let n = self.doc.nodes.values().filter(|n| n.component).count();
            self.edit("Create component", |tx| {
                let node = tx.node_mut(f).unwrap();
                node.component = true;
                node.name = format!("Component {}", n + 1);
            });
        }
    }

    /// Place an instance of a main component at world position `at` (top-left).
    pub fn create_instance(&mut self, component: NodeId, at: V2) -> Option<NodeId> {
        let main = self.doc.get(component)?;
        if !main.component {
            return None;
        }
        let size = (main.rect.w, main.rect.h);
        let id = self.edit("Create instance", |tx| {
            instantiate(
                tx,
                component,
                None,
                None,
                Some(Rect::new(at.x, at.y, size.0, size.1)),
            )
        });
        self.selection = vec![id];
        Some(id)
    }

    /// Duplicate the selected component as an instance to its right (Figma: Alt-drag).
    pub fn instance_of_selection(&mut self) {
        let Some(&c) = self.selection.first() else {
            return;
        };
        let Some(r) = self.doc.world_rect(c) else {
            return;
        };
        if self.doc.get(c).is_some_and(|n| n.component) {
            self.create_instance(c, v2(r.right() + 40.0, r.y));
        }
    }

    /// Detach selected instances: they become plain frames (Cmd+Alt+B).
    pub fn detach_instance(&mut self) {
        let roots: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| self.is_instance_root(*id))
            .collect();
        if roots.is_empty() {
            return;
        }
        self.edit("Detach instance", |tx| {
            for r in roots {
                for id in tx.doc().subtree(r) {
                    tx.node_mut(id).unwrap().instance = None;
                }
            }
        });
    }

    /// Clear overrides on the selected instance nodes and resync from the main component.
    pub fn reset_overrides(&mut self) {
        let ids = self.top_selection();
        let roots: Vec<NodeId> = ids
            .iter()
            .filter_map(|id| {
                std::iter::once(*id)
                    .chain(self.doc.ancestors(*id))
                    .find(|a| self.is_instance_root(*a))
            })
            .collect();
        self.edit("Reset overrides", |tx| {
            for r in &ids {
                for id in tx.doc().subtree(*r) {
                    if let Some(l) = &mut tx.node_mut(id).unwrap().instance {
                        l.overrides.clear();
                    }
                }
            }
            for r in roots {
                sync_node(tx, r, true);
            }
        });
    }

    pub fn is_instance_root(&self, id: NodeId) -> bool {
        self.doc.get(id).is_some_and(|n| {
            n.instance
                .as_ref()
                .is_some_and(|l| self.doc.get(l.main).is_some_and(|m| m.component))
        })
    }

    /// The main component an instance root points to.
    pub fn instance_main_root(&self, id: NodeId) -> Option<NodeId> {
        let l = self.doc.get(id)?.instance.as_ref()?;
        self.doc.get(l.main).filter(|m| m.component).map(|m| m.id)
    }

    /// Select the main component of the selected instance.
    pub fn go_to_main(&mut self) {
        if let Some(m) = self
            .selection
            .first()
            .and_then(|id| self.instance_main_root(*id))
        {
            self.selection = vec![m];
            if let Some(r) = self.doc.world_rect(m) {
                self.camera
                    .fit(&r, self.viewport, 80.0, self.camera.zoom.max(1.0));
                self.revision += 1;
            }
        }
    }

    // ---------------------------------------------------------------------- auto layout

    /// Add auto layout (Shift+A): converts a selected frame, or wraps the selection in a new
    /// auto-layout frame whose direction, gap and padding are inferred from the arrangement.
    pub fn add_auto_layout(&mut self) {
        let ids = self.top_selection();
        if ids.is_empty() {
            return;
        }
        if let [id] = ids[..]
            && self.doc.get(id).is_some_and(|n| n.is_frame())
        {
            if self.doc.get(id).unwrap().layout.is_some() {
                return;
            }
            let kids = self.doc.get(id).unwrap().children.clone();
            let rects: Vec<Rect> = kids
                .iter()
                .map(|k| self.doc.get(*k).unwrap().rect)
                .collect();
            let l = infer_layout(
                &rects,
                self.doc.get(id).unwrap().rect.w,
                self.doc.get(id).unwrap().rect.h,
            );
            self.edit("Add auto layout", |tx| {
                // Order children along the inferred axis.
                let mut order = kids.clone();
                order.sort_by(|a, b| {
                    let (ra, rb) = (
                        tx.doc().get(*a).unwrap().rect,
                        tx.doc().get(*b).unwrap().rect,
                    );
                    if l.direction == Direction::Horizontal {
                        ra.x.total_cmp(&rb.x)
                    } else {
                        ra.y.total_cmp(&rb.y)
                    }
                });
                for (i, k) in order.iter().enumerate() {
                    tx.move_to_index(*k, i);
                }
                let n = tx.node_mut(id).unwrap();
                n.layout = Some(l);
                n.layout_child.sizing = [Sizing::Hug; 2];
            });
            self.edit("Add auto layout", |_| {});
            return;
        }
        let rects: Vec<Rect> = ids.iter().filter_map(|i| self.doc.world_rect(*i)).collect();
        self.wrap_in_frame();
        let Some(&f) = self.selection.first() else {
            return;
        };
        let fr = self.doc.world_rect(f).unwrap();
        let rel: Vec<Rect> = rects
            .iter()
            .map(|r| r.translate(V2::ZERO - fr.min()))
            .collect();
        let mut l = infer_layout(&rel, fr.w, fr.h);
        l.padding = [0.0; 4];
        let kids = self.doc.get(f).unwrap().children.clone();
        self.edit("Add auto layout", |tx| {
            let mut order = kids.clone();
            order.sort_by(|a, b| {
                let (ra, rb) = (
                    tx.doc().get(*a).unwrap().rect,
                    tx.doc().get(*b).unwrap().rect,
                );
                if l.direction == Direction::Horizontal {
                    ra.x.total_cmp(&rb.x)
                } else {
                    ra.y.total_cmp(&rb.y)
                }
            });
            for (i, k) in order.iter().enumerate() {
                tx.move_to_index(*k, i);
            }
            let n = tx.node_mut(f).unwrap();
            n.layout = Some(l);
            n.layout_child.sizing = [Sizing::Hug; 2];
            n.name = "Auto layout".into();
        });
    }

    pub fn remove_auto_layout(&mut self) {
        let ids: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| self.doc.get(*id).is_some_and(|n| n.layout.is_some()))
            .collect();
        self.edit("Remove auto layout", |tx| {
            for id in ids {
                let n = tx.node_mut(id).unwrap();
                n.layout = None;
                n.layout_child.sizing = [Sizing::Fixed; 2];
            }
        });
    }

    // ---------------------------------------------------------------------- align

    /// Align the selection: to its bounds, or a single layer to its parent frame.
    /// `axis` 0 = x, 1 = y; `to` 0 = start, 0.5 = center, 1 = end.
    pub fn align(&mut self, axis: usize, to: f64) {
        let ids: Vec<NodeId> = self
            .top_selection()
            .into_iter()
            .filter(|id| !self.doc.is_effectively_locked(*id))
            .collect();
        let target = if ids.len() == 1 {
            match self.doc.get(ids[0]).and_then(|n| n.parent) {
                Some(p) => self.doc.world_rect(p),
                None => return,
            }
        } else {
            self.selection_bounds()
        };
        let Some(t) = target else { return };
        let moves: Vec<(NodeId, V2)> = ids
            .iter()
            .filter_map(|id| {
                let r = self.doc.world_rect(*id)?;
                let d = if axis == 0 {
                    v2(t.x + (t.w - r.w) * to - r.x, 0.0)
                } else {
                    v2(0.0, t.y + (t.h - r.h) * to - r.y)
                };
                Some((*id, d))
            })
            .collect();
        self.edit("Align", |tx| {
            for (id, d) in moves {
                let n = tx.node_mut(id).unwrap();
                n.rect = n.rect.translate(d);
            }
        });
    }

    /// Equalize the gaps between 3+ selected layers along an axis.
    pub fn distribute(&mut self, axis: usize) {
        let mut items: Vec<(NodeId, Rect)> = self
            .top_selection()
            .into_iter()
            .filter_map(|id| Some((id, self.doc.world_rect(id)?)))
            .collect();
        if items.len() < 3 {
            return;
        }
        let key = |r: &Rect| if axis == 0 { r.x } else { r.y };
        let size = |r: &Rect| if axis == 0 { r.w } else { r.h };
        items.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
        let first = key(&items[0].1);
        let last = items.last().unwrap();
        let span = key(&last.1) + size(&last.1) - first;
        let total: f64 = items.iter().map(|(_, r)| size(r)).sum();
        let gap = (span - total) / (items.len() - 1) as f64;
        let mut cursor = first;
        let mut moves = Vec::new();
        for (id, r) in &items {
            let d = cursor - key(r);
            moves.push((*id, if axis == 0 { v2(d, 0.0) } else { v2(0.0, d) }));
            cursor += size(r) + gap;
        }
        self.edit("Distribute", |tx| {
            for (id, d) in moves {
                let n = tx.node_mut(id).unwrap();
                n.rect = n.rect.translate(d);
            }
        });
    }
}

/// Infer auto-layout settings from children's current arrangement (frame-local rects).
pub fn infer_layout(rects: &[Rect], fw: f64, fh: f64) -> AutoLayout {
    let mut l = AutoLayout::default();
    if rects.is_empty() {
        return l;
    }
    let u = Rect::union_all(rects).unwrap();
    let horizontal = u.w / fw.max(1.0) >= u.h / fh.max(1.0) && rects.len() > 1 && {
        let spread_x = u.w - rects.iter().map(|r| r.w).fold(0.0, f64::max);
        let spread_y = u.h - rects.iter().map(|r| r.h).fold(0.0, f64::max);
        spread_x >= spread_y
    };
    l.direction = if horizontal {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let mut sorted = rects.to_vec();
    if horizontal {
        sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    } else {
        sorted.sort_by(|a, b| a.y.total_cmp(&b.y));
    }
    let gaps: Vec<f64> = sorted
        .windows(2)
        .map(|w| {
            if horizontal {
                w[1].x - w[0].right()
            } else {
                w[1].y - w[0].bottom()
            }
        })
        .collect();
    if !gaps.is_empty() {
        l.gap = (gaps.iter().sum::<f64>() / gaps.len() as f64)
            .round()
            .max(0.0);
    }
    l.padding = [
        u.y.max(0.0).round(),
        (fw - u.right()).max(0.0).round(),
        (fh - u.bottom()).max(0.0).round(),
        u.x.max(0.0).round(),
    ];
    l
}
