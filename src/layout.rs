//! Auto layout and resize constraints.
//!
//! [Taffy](https://github.com/DioxusLabs/taffy) does the layout, so frames get real CSS flexbox,
//! grid and block flow. This module maps nodes onto Taffy styles and writes the results back.
//!
//! Two kinds of frame share one engine. Frames built in the editor follow the Figma rules in
//! `analysis/FIGMA_NOTES.md` §3–4: children never shrink, and `Sizing::Fill` shares free space
//! equally. Frames written as CSS set the extra [`LayoutChild`] fields (`grow`, `shrink`,
//! `margin`, ...), which turn on the CSS behavior for that child.

use crate::geom::{Rect, V2, v2};
use crate::history::Tx;
use crate::model::{
    Align, AutoLayout, Constraint, Dim, Direction, Distribute, Document, GridPos, LayoutChild,
    LayoutMode, Node, NodeId, NodeKind, SelfAlign, Sizing,
};
use taffy::prelude::{
    AlignContent, AlignItems, Dimension, Display, FlexDirection, FlexWrap, FromFr, FromLength,
    FromPercent, GridPlacement, GridTemplateComponent, JustifyContent, Layout, LengthPercentage,
    LengthPercentageAuto, Line, MaxTrackSizingFunction, MinTrackSizingFunction, Position, Size,
    Style, TaffyAuto, TaffyMaxContent, TaffyMinContent, TaffyTree, TrackSizingFunction, auto, fr,
    length, line, max_content, min_content, minmax, percent, repeat, span,
};
use taffy::style::AvailableSpace;

type DefaultCheapStr = String;

fn main_axis(l: &AutoLayout) -> usize {
    match l.direction {
        Direction::Horizontal => 0,
        Direction::Vertical => 1,
    }
}

/// Position and size of a box along one axis.
fn get(r: &Rect, axis: usize) -> (f64, f64) {
    if axis == 0 { (r.x, r.w) } else { (r.y, r.h) }
}

/// Children that take part in the flow.
pub fn flow_children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.children(Some(id))
        .iter()
        .copied()
        .filter(|c| {
            doc.get(*c)
                .is_some_and(|n| n.visible && !n.layout_child.absolute)
        })
        .collect()
}

// ------------------------------------------------------------------------------ grid tracks

/// One grid track, as CSS writes it.
fn track(s: &str) -> Option<TrackSizingFunction> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("minmax(").and_then(|t| t.strip_suffix(')')) {
        let (a, b) = inner.split_once(',')?;
        return Some(minmax(track_min(a.trim())?, track_max(b.trim())?));
    }
    Some(match s {
        "auto" => auto(),
        "min-content" => min_content(),
        "max-content" => max_content(),
        _ => {
            if let Some(v) = s.strip_suffix("fr") {
                fr(v.trim().parse::<f32>().ok()?)
            } else if let Some(v) = s.strip_suffix('%') {
                percent(v.trim().parse::<f32>().ok()? / 100.0)
            } else {
                length(
                    s.strip_suffix("px")
                        .unwrap_or(s)
                        .trim()
                        .parse::<f32>()
                        .ok()?,
                )
            }
        }
    })
}

fn track_min(s: &str) -> Option<MinTrackSizingFunction> {
    Some(match s {
        "auto" => MinTrackSizingFunction::AUTO,
        "min-content" => MinTrackSizingFunction::MIN_CONTENT,
        "max-content" => MinTrackSizingFunction::MAX_CONTENT,
        _ => {
            if let Some(v) = s.strip_suffix('%') {
                MinTrackSizingFunction::from_percent(v.trim().parse::<f32>().ok()? / 100.0)
            } else {
                let v = s.strip_suffix("px").unwrap_or(s).trim();
                MinTrackSizingFunction::from_length(v.parse::<f32>().ok()?)
            }
        }
    })
}

fn track_max(s: &str) -> Option<MaxTrackSizingFunction> {
    Some(match s {
        "auto" => MaxTrackSizingFunction::AUTO,
        "min-content" => MaxTrackSizingFunction::MIN_CONTENT,
        "max-content" => MaxTrackSizingFunction::MAX_CONTENT,
        _ => {
            if let Some(v) = s.strip_suffix("fr") {
                MaxTrackSizingFunction::from_fr(v.trim().parse::<f32>().ok()?)
            } else if let Some(v) = s.strip_suffix('%') {
                MaxTrackSizingFunction::from_percent(v.trim().parse::<f32>().ok()? / 100.0)
            } else {
                let v = s.strip_suffix("px").unwrap_or(s).trim();
                MaxTrackSizingFunction::from_length(v.parse::<f32>().ok()?)
            }
        }
    })
}

/// Parse a `grid-template-columns` or `-rows` value (`None` if any part isn't understood).
pub fn parse_tracks(s: &str) -> Option<Vec<GridTemplateComponent<DefaultCheapStr>>> {
    let mut out = Vec::new();
    for part in crate::mcp::css::split_ws(s) {
        if let Some(inner) = part
            .strip_prefix("repeat(")
            .and_then(|t| t.strip_suffix(')'))
        {
            let (count, tracks) = inner.split_once(',')?;
            let list: Vec<TrackSizingFunction> = crate::mcp::css::split_ws(tracks.trim())
                .iter()
                .map(|t| track(t))
                .collect::<Option<_>>()?;
            out.push(match count.trim() {
                "auto-fit" => repeat("auto-fit", list),
                "auto-fill" => repeat("auto-fill", list),
                n => repeat(n.parse::<u16>().ok()?, list),
            });
        } else {
            out.push(GridTemplateComponent::Single(track(&part)?));
        }
    }
    (!out.is_empty()).then_some(out)
}

fn grid_pos(p: GridPos) -> GridPlacement {
    match p {
        GridPos::Auto => GridPlacement::Auto,
        GridPos::Line(i) => line(i),
        GridPos::Span(n) => span(n),
    }
}

// ------------------------------------------------------------------------------ node → style

fn lp(v: f64) -> LengthPercentage {
    LengthPercentage::from_length(v as f32)
}

fn lpa(d: Option<Dim>, unset: LengthPercentageAuto) -> LengthPercentageAuto {
    match d {
        None => unset,
        Some(Dim::Px(v)) => LengthPercentageAuto::from_length(v as f32),
        Some(Dim::Pct(p)) => LengthPercentageAuto::from_percent(p as f32),
        Some(Dim::Auto) => LengthPercentageAuto::AUTO,
    }
}

fn dimension(d: Dim) -> Dimension {
    match d {
        Dim::Px(v) => Dimension::from_length(v as f32),
        Dim::Pct(p) => Dimension::from_percent(p as f32),
        Dim::Auto => Dimension::AUTO,
    }
}

fn is_text(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Text { .. })
}

/// Text that keeps its natural width and never wraps (Figma's auto width, `white-space: nowrap`).
fn nowrap_text(n: &Node) -> bool {
    matches!(
        n.kind,
        NodeKind::Text {
            auto_width: true,
            ..
        }
    ) && n.layout_child.sizing[0] == Sizing::Fixed
}

/// A `Fill` child with no CSS flex values: it shares free space equally, whatever its content.
fn figma_fill(lc: &LayoutChild, axis: usize) -> bool {
    lc.sizing[axis] == Sizing::Fill && lc.grow.is_none()
}

/// The Figma rule that a fill child on the main axis turns its hugging parent into a fixed one.
fn hug_held_by_fill(doc: &Document, n: &Node, axis: usize) -> bool {
    n.layout.is_some_and(|l| {
        l.mode == LayoutMode::Flex
            && main_axis(&l) == axis
            && flow_children(doc, n.id)
                .iter()
                .any(|k| figma_fill(&doc.get(*k).unwrap().layout_child, axis))
    })
}

fn size_on(doc: &Document, n: &Node, axis: usize, parent: Option<&AutoLayout>) -> Dimension {
    let (lc, own) = (&n.layout_child, if axis == 0 { n.rect.w } else { n.rect.h });
    let fixed = Dimension::from_length(own as f32);
    let Some(parent) = parent else {
        // The root of a layout pass has no parent to fill, and its size is the caller's.
        let hug = lc.sizing[axis] == Sizing::Hug && !hug_held_by_fill(doc, n, axis);
        return if hug { Dimension::AUTO } else { fixed };
    };
    if let Some(p) = lc.percent[axis] {
        return Dimension::from_percent(p as f32);
    }
    let text = is_text(n);
    match lc.sizing[axis] {
        // A text box is always as tall as its lines; auto width means the content decides.
        Sizing::Fixed if text && (axis == 1 || nowrap_text(n)) => Dimension::AUTO,
        Sizing::Fixed => fixed,
        Sizing::Hug if !text && n.layout.is_none() => fixed,
        Sizing::Hug if hug_held_by_fill(doc, n, axis) => fixed,
        Sizing::Hug => Dimension::AUTO,
        Sizing::Fill => match parent.mode {
            LayoutMode::Block if axis == 1 => Dimension::from_percent(1.0),
            _ => Dimension::AUTO,
        },
    }
}

fn style_of(doc: &Document, n: &Node, parent: Option<&AutoLayout>) -> Style {
    let lc = &n.layout_child;
    // Every frame is a containing block, as in Figma: an absolute child offsets from its parent.
    let mut st = Style {
        position: Position::Relative,
        ..Style::default()
    };
    if let Some(l) = &n.layout {
        st.display = match l.mode {
            LayoutMode::Flex => Display::Flex,
            LayoutMode::Block => Display::Block,
            LayoutMode::Grid => Display::Grid,
        };
        st.flex_direction = match l.direction {
            Direction::Horizontal => FlexDirection::Row,
            Direction::Vertical => FlexDirection::Column,
        };
        st.flex_wrap = if l.wrap {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        };
        let main_gap = if l.gap_auto { 0.0 } else { l.gap };
        let (column, row) = match l.direction {
            Direction::Horizontal => (main_gap, l.cross_gap),
            Direction::Vertical => (l.cross_gap, main_gap),
        };
        st.gap = Size {
            width: lp(column),
            height: lp(row),
        };
        st.padding = taffy::Rect {
            top: lp(l.padding[0]),
            right: lp(l.padding[1]),
            bottom: lp(l.padding[2]),
            left: lp(l.padding[3]),
        };
        st.border = taffy::Rect {
            top: lp(l.border[0]),
            right: lp(l.border[1]),
            bottom: lp(l.border[2]),
            left: lp(l.border[3]),
        };
        let many = flow_children(doc, n.id).len() > 1;
        st.justify_content = match (l.gap_auto && many, l.distribute, l.align_main) {
            (true, Distribute::Between, _) => JustifyContent::SPACE_BETWEEN,
            (true, Distribute::Around, _) => JustifyContent::SPACE_AROUND,
            (true, Distribute::Evenly, _) => JustifyContent::SPACE_EVENLY,
            (false, _, Align::Start) => JustifyContent::FLEX_START,
            (false, _, Align::Center) => JustifyContent::CENTER,
            (false, _, Align::End) => JustifyContent::FLEX_END,
        };
        st.align_items = match l.align_cross {
            Align::Start => AlignItems::FLEX_START,
            Align::Center => AlignItems::CENTER,
            Align::End => AlignItems::FLEX_END,
        };
        // Wrapped lines stack from the start rather than stretching apart.
        st.align_content = AlignContent::FLEX_START;
        if let Some(g) = &n.grid {
            st.grid_template_columns = parse_tracks(&g.columns).unwrap_or_default();
            st.grid_template_rows = parse_tracks(&g.rows).unwrap_or_default();
            st.align_items = AlignItems::STRETCH;
            st.justify_items = AlignItems::STRETCH;
        }
    } else {
        st.display = Display::Block;
    }
    st.size = Size {
        width: size_on(doc, n, 0, parent),
        height: size_on(doc, n, 1, parent),
    };
    let unset = LengthPercentageAuto::AUTO;
    st.min_size = Size {
        width: lpa(lc.min[0], unset),
        height: lpa(lc.min[1], unset),
    };
    st.max_size = Size {
        width: lpa(lc.max[0], unset),
        height: lpa(lc.max[1], unset),
    };
    st.aspect_ratio = lc.aspect.map(|a| a as f32);
    let Some(parent) = parent else {
        return st;
    };
    let zero = LengthPercentageAuto::from_length(0.0);
    st.margin = taffy::Rect {
        top: lpa(lc.margin[0], zero),
        right: lpa(lc.margin[1], zero),
        bottom: lpa(lc.margin[2], zero),
        left: lpa(lc.margin[3], zero),
    };
    if lc.absolute {
        st.position = Position::Absolute;
        st.inset = taffy::Rect {
            top: lpa(lc.inset[0], unset),
            right: lpa(lc.inset[1], unset),
            bottom: lpa(lc.inset[2], unset),
            left: lpa(lc.inset[3], unset),
        };
    } else if lc.shift != [0.0; 2] {
        st.inset = taffy::Rect {
            top: lpa(Some(Dim::Px(lc.shift[1])), unset),
            left: lpa(Some(Dim::Px(lc.shift[0])), unset),
            right: unset,
            bottom: unset,
        };
    }
    st.flex_shrink = lc.shrink.unwrap_or(0.0) as f32;
    st.flex_grow = lc.grow.unwrap_or(0.0) as f32;
    st.flex_basis = lc.basis.map_or(Dimension::AUTO, dimension);
    st.align_self = lc.align_self.map(|a| match a {
        SelfAlign::Start => AlignItems::FLEX_START,
        SelfAlign::Center => AlignItems::CENTER,
        SelfAlign::End => AlignItems::FLEX_END,
        SelfAlign::Stretch => AlignItems::STRETCH,
    });
    if parent.mode == LayoutMode::Flex {
        let ma = main_axis(parent);
        if figma_fill(lc, ma) {
            st.flex_grow = 1.0;
            st.flex_shrink = 1.0;
            st.flex_basis = Dimension::from_length(0.0);
            if ma == 0 {
                st.min_size.width = zero;
            } else {
                st.min_size.height = zero;
            }
        }
        if lc.sizing[1 - ma] == Sizing::Fill && lc.align_self.is_none() {
            st.align_self = Some(AlignItems::STRETCH);
        }
    }
    if parent.mode == LayoutMode::Grid {
        st.grid_column = Line {
            start: grid_pos(lc.grid_column[0]),
            end: grid_pos(lc.grid_column[1]),
        };
        st.grid_row = Line {
            start: grid_pos(lc.grid_row[0]),
            end: grid_pos(lc.grid_row[1]),
        };
    }
    st
}

// ------------------------------------------------------------------------------ layout pass

/// A node's place in the Taffy tree.
struct Built {
    tid: taffy::NodeId,
    id: NodeId,
    kids: Vec<Built>,
}

/// Whether a child takes part in its parent's layout pass. An absolute child with no offsets
/// stays where constraints put it, as it does in Figma.
fn in_pass(n: &Node) -> bool {
    n.visible && (!n.layout_child.absolute || n.layout_child.inset.iter().any(Option::is_some))
}

fn build(
    tree: &mut TaffyTree<NodeId>,
    doc: &Document,
    id: NodeId,
    parent: Option<&AutoLayout>,
) -> Option<Built> {
    let n = doc.get(id)?;
    let style = style_of(doc, n, parent);
    if is_text(n) {
        let tid = tree.new_leaf_with_context(style, id).ok()?;
        return Some(Built {
            tid,
            id,
            kids: Vec::new(),
        });
    }
    let mut kids = Vec::new();
    if let Some(l) = &n.layout {
        for k in doc.children(Some(id)) {
            if doc.get(*k).is_some_and(in_pass)
                && let Some(b) = build(tree, doc, *k, Some(l))
            {
                kids.push(b);
            }
        }
    }
    let ids: Vec<taffy::NodeId> = kids.iter().map(|k| k.tid).collect();
    let tid = tree.new_with_children(style, &ids).ok()?;
    Some(Built { tid, id, kids })
}

fn measure(
    doc: &Document,
    id: NodeId,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> Size<f32> {
    let Some(n) = doc.get(id) else {
        return Size::ZERO;
    };
    let wrap_at = if nowrap_text(n) {
        None
    } else {
        match (known.width, available.width) {
            (Some(w), _) => Some(w as f64),
            (None, AvailableSpace::Definite(w)) => Some(w as f64),
            (None, AvailableSpace::MaxContent) => None,
            (None, AvailableSpace::MinContent) => Some(crate::text::min_content_width(n)),
        }
    };
    let Some(l) = crate::text::measure_node(n, wrap_at) else {
        return Size::ZERO;
    };
    // Rounded up as browsers do, so text in a box sized to fit it can't wrap on rounding alone.
    Size {
        width: known.width.unwrap_or(l.width.ceil() as f32),
        height: known.height.unwrap_or(l.height as f32),
    }
}

/// Copy one node's computed box back. The root keeps its position; everything else is placed
/// relative to its parent, which is how Taffy reports it.
fn write_back(tx: &mut Tx, tree: &TaffyTree<NodeId>, b: &Built, root: bool) {
    let Ok(l): Result<&Layout, _> = tree.layout(b.tid) else {
        return;
    };
    let Some(n) = tx.doc().get(b.id) else {
        return;
    };
    let old = n.rect;
    let (x, y) = if root {
        (old.x, old.y)
    } else {
        (l.location.x as f64, l.location.y as f64)
    };
    let mut r = Rect::new(x, y, l.size.width as f64, l.size.height as f64);
    // Taffy snaps boxes to whole pixels. A snap must not push the last word of a line that fits
    // onto a new line, so a text box may stick out of its slot by that one pixel.
    if is_text(n) && !nowrap_text(n) {
        let natural = crate::text::measure_node(n, None).map_or(r.w, |t| t.width);
        if natural > r.w && natural - r.w <= 1.0 {
            r.w = natural.ceil();
        }
    }
    // CSS text fits its box. It keeps its natural width unless the box made it wrap or stretch.
    let fit_width = (n.layout_child.sizing[0] == Sizing::Hug && is_text(n)).then(|| {
        let natural = crate::text::measure_node(n, None).map_or(r.w, |t| t.width);
        (r.w - natural).abs() <= 1.0
    });
    let flag_changes = matches!(
        (&n.kind, fit_width),
        (NodeKind::Text { auto_width, .. }, Some(fit)) if *auto_width != fit
    );
    if r != old || flag_changes {
        let resized = (r.w - old.w).abs() > 1e-9 || (r.h - old.h).abs() > 1e-9;
        let text = is_text(n);
        let free_frame = n.layout.is_none();
        let node = tx.node_mut(b.id).unwrap();
        if text || !resized {
            node.rect = r;
        } else {
            node.set_rect_scaling(r);
        }
        if let (NodeKind::Text { auto_width, .. }, Some(fit)) = (&mut node.kind, fit_width) {
            *auto_width = fit;
        }
        if resized && free_frame && !text {
            resize_children(tx, b.id, old, r);
        }
    }
    for k in &b.kids {
        write_back(tx, tree, k, false);
    }
}

fn snapshot(doc: &Document, id: NodeId) -> Vec<(NodeId, Rect, bool)> {
    doc.subtree(id)
        .iter()
        .filter_map(|i| {
            doc.get(*i).map(|n| {
                let auto = matches!(
                    n.kind,
                    NodeKind::Text {
                        auto_width: true,
                        ..
                    }
                );
                (*i, n.rect, auto)
            })
        })
        .collect()
}

/// Run Taffy over a frame and everything inside it that takes part in layout.
fn pass(tx: &mut Tx, id: NodeId) {
    let mut tree = TaffyTree::<NodeId>::new();
    let Some(built) = build(&mut tree, tx.doc(), id, None) else {
        return;
    };
    let doc = tx.doc();
    let computed = tree.compute_layout_with_measure(
        built.tid,
        Size::MAX_CONTENT,
        |inputs, _, context, style| {
            taffy::compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, available| match context {
                    Some(text) => measure(doc, *text, known, available),
                    None => Size::ZERO,
                },
            )
        },
    );
    if computed.is_ok() {
        write_back(tx, &tree, &built, true);
    }
}

/// Lay out one auto-layout frame and everything inside it. Returns true if anything moved or
/// resized.
pub fn layout_frame(tx: &mut Tx, id: NodeId) -> bool {
    if tx.doc().get(id).and_then(|n| n.layout).is_none() {
        return false;
    }
    let before = snapshot(tx.doc(), id);
    pass(tx, id);
    before != snapshot(tx.doc(), id)
}

/// Re-measure a text node's height after its width changed.
pub fn fit_text_height(tx: &mut Tx, id: NodeId) {
    let Some(n) = tx.doc().get(id) else { return };
    if !matches!(n.kind, NodeKind::Text { .. }) {
        return;
    }
    if let Some(l) = crate::text::layout_node(n)
        && (l.height - n.rect.h).abs() > 1e-9
    {
        tx.node_mut(id).unwrap().rect.h = l.height;
    }
}

/// After a container resized from `old` to `new` (both in its parent's space), update its
/// children: constraints for frames, proportional scaling for groups, layout for auto layout.
pub fn resize_children(tx: &mut Tx, id: NodeId, old: Rect, new: Rect) {
    let Some(n) = tx.doc().get(id) else { return };
    if n.children.is_empty() {
        return;
    }
    let group = n.is_group();
    let auto = n.layout.is_some();
    let kids = n.children.clone();
    if auto {
        // Absolute children with no offsets still follow constraints.
        for k in &kids {
            if tx
                .doc()
                .get(*k)
                .is_some_and(|c| c.layout_child.absolute && !in_pass(c))
            {
                apply_constraints(tx, *k, old, new);
            }
        }
        // A resize settles the frame's size, even one that was hugging its content.
        let sizing = tx.doc().get(id).unwrap().layout_child.sizing;
        tx.node_mut(id).unwrap().layout_child.sizing = [Sizing::Fixed; 2];
        pass(tx, id);
        tx.node_mut(id).unwrap().layout_child.sizing = sizing;
        return;
    }
    for k in kids {
        if group {
            scale_child(tx, k, old, new);
        } else {
            apply_constraints(tx, k, old, new);
        }
    }
}

fn scale_child(tx: &mut Tx, k: NodeId, old: Rect, new: Rect) {
    let sx = if old.w.abs() > 1e-9 {
        new.w / old.w
    } else {
        1.0
    };
    let sy = if old.h.abs() > 1e-9 {
        new.h / old.h
    } else {
        1.0
    };
    let Some(c) = tx.doc().get(k) else { return };
    let r0 = c.rect;
    let r = Rect::new(r0.x * sx, r0.y * sy, r0.w * sx, r0.h * sy);
    tx.node_mut(k).unwrap().set_rect_scaling(r);
    resize_children(tx, k, r0, r);
}

/// Reposition/resize one child for a parent size change (positions relative to the parent).
pub fn apply_constraints(tx: &mut Tx, k: NodeId, old: Rect, new: Rect) {
    let Some(c) = tx.doc().get(k) else { return };
    let r0 = c.rect;
    let cons = c.layout_child.constraints;
    let axis = |con: Constraint, p: f64, s: f64, old_s: f64, new_s: f64| -> (f64, f64) {
        let far = old_s - (p + s);
        match con {
            Constraint::Min => (p, s),
            Constraint::Max => (new_s - far - s, s),
            Constraint::Stretch => (p, (new_s - p - far).max(0.0)),
            Constraint::Center => (new_s * 0.5 + (p + s * 0.5 - old_s * 0.5) - s * 0.5, s),
            Constraint::Scale => {
                let k = if old_s.abs() > 1e-9 {
                    new_s / old_s
                } else {
                    1.0
                };
                (p * k, s * k)
            }
        }
    };
    let (x, w) = axis(cons.h, r0.x, r0.w, old.w, new.w);
    let (y, h) = axis(cons.v, r0.y, r0.h, old.h, new.h);
    let r = Rect::new(x, y, w, h);
    if r == r0 {
        return;
    }
    let is_text = matches!(c.kind, NodeKind::Text { .. });
    let n = tx.node_mut(k).unwrap();
    if (r.w - r0.w).abs() > 1e-9 || (r.h - r0.h).abs() > 1e-9 {
        n.set_rect_scaling(r);
    } else {
        n.rect = r;
    }
    let _ = is_text;
    resize_children(tx, k, r0, r);
}

/// Recompute a group's box to the union of its children (keeping their world positions).
/// Empty groups are removed. Returns true if changed.
pub fn normalize_group(tx: &mut Tx, id: NodeId) -> bool {
    let Some(g) = tx.doc().get(id) else {
        return false;
    };
    if !g.is_group() {
        return false;
    }
    if g.children.is_empty() {
        tx.remove(id);
        return true;
    }
    let rects: Vec<Rect> = g
        .children
        .iter()
        .filter_map(|c| tx.doc().get(*c).map(|n| n.rect))
        .collect();
    let u = Rect::union_all(&rects).unwrap();
    if u.x == 0.0 && u.y == 0.0 && u.w == g.rect.w && u.h == g.rect.h {
        return false;
    }
    let d = v2(u.x, u.y);
    let kids = g.children.clone();
    let gr = g.rect;
    for c in kids {
        let n = tx.node_mut(c).unwrap();
        n.rect = n.rect.translate(V2::ZERO - d);
    }
    tx.node_mut(id).unwrap().rect = Rect::new(gr.x + d.x, gr.y + d.y, u.w, u.h);
    true
}

/// Child index at which a dragged child should land in an auto-layout frame, from the
/// child's center (frame-local) along the main axis.
pub fn drop_index(doc: &Document, frame: NodeId, dragged: NodeId, center: V2) -> usize {
    let Some(l) = doc.get(frame).and_then(|f| f.layout) else {
        return 0;
    };
    let ma = main_axis(&l);
    let c = if ma == 0 { center.x } else { center.y };
    let others: Vec<NodeId> = doc
        .children(Some(frame))
        .iter()
        .copied()
        .filter(|k| *k != dragged)
        .collect();
    let mut idx = 0;
    for (i, k) in others.iter().enumerate() {
        let r = doc.get(*k).unwrap().rect;
        let (p, s) = get(&r, ma);
        if c > p + s * 0.5 {
            idx = i + 1;
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Recording;
    use crate::model::{Constraints, Node};

    fn doc_with(
        frame: AutoLayout,
        sizing: [Sizing; 2],
        kids: &[(f64, f64, [Sizing; 2])],
    ) -> (Document, NodeId, Vec<NodeId>) {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let fid = tx.alloc_id();
        let mut f = Node::new(
            fid,
            NodeKind::Frame { clip: true },
            Rect::new(100.0, 100.0, 300.0, 200.0),
        );
        f.layout = Some(frame);
        f.layout_child.sizing = sizing;
        tx.insert(f, None, None);
        let mut ids = Vec::new();
        for (w, h, s) in kids {
            let id = tx.alloc_id();
            let mut n = Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, *w, *h));
            n.layout_child.sizing = *s;
            tx.insert(n, Some(fid), None);
            ids.push(id);
        }
        layout_frame(&mut tx, fid);
        (doc, fid, ids)
    }

    const FX: [Sizing; 2] = [Sizing::Fixed, Sizing::Fixed];

    #[test]
    fn horizontal_hug_with_gap_and_padding() {
        let l = AutoLayout {
            gap: 8.0,
            padding: [10.0, 20.0, 10.0, 20.0],
            ..Default::default()
        };
        let (doc, f, k) = doc_with(l, [Sizing::Hug; 2], &[(50.0, 30.0, FX), (40.0, 60.0, FX)]);
        let fr = doc.get(f).unwrap().rect;
        assert_eq!(
            (fr.w, fr.h),
            (20.0 + 50.0 + 8.0 + 40.0 + 20.0, 10.0 + 60.0 + 10.0)
        );
        assert_eq!(
            doc.get(k[0]).unwrap().rect,
            Rect::new(20.0, 10.0, 50.0, 30.0)
        );
        assert_eq!(
            doc.get(k[1]).unwrap().rect,
            Rect::new(78.0, 10.0, 40.0, 60.0)
        );
        assert_eq!((fr.x, fr.y), (100.0, 100.0), "hug keeps the frame's origin");
    }

    #[test]
    fn fill_children_share_space_and_center_alignment() {
        let l = AutoLayout {
            direction: Direction::Vertical,
            gap: 10.0,
            padding: [0.0; 4],
            align_cross: Align::Center,
            ..Default::default()
        };
        let (doc, _, k) = doc_with(
            l,
            FX,
            &[
                (100.0, 50.0, FX),
                (50.0, 0.0, [Sizing::Fixed, Sizing::Fill]),
                (300.0, 0.0, [Sizing::Fill, Sizing::Fill]),
            ],
        );
        // 200 tall: 50 fixed + 2 gaps of 10 = 70; two fills share 130.
        assert_eq!(
            doc.get(k[1]).unwrap().rect,
            Rect::new(125.0, 60.0, 50.0, 65.0)
        );
        assert_eq!(
            doc.get(k[2]).unwrap().rect,
            Rect::new(0.0, 135.0, 300.0, 65.0)
        );
        assert_eq!(doc.get(k[0]).unwrap().rect.x, 100.0);
    }

    #[test]
    fn space_between_and_end_alignment() {
        let l = AutoLayout {
            gap_auto: true,
            padding: [0.0; 4],
            align_cross: Align::End,
            ..Default::default()
        };
        let (doc, _, k) = doc_with(
            l,
            FX,
            &[(50.0, 20.0, FX), (50.0, 20.0, FX), (50.0, 20.0, FX)],
        );
        let xs: Vec<f64> = k.iter().map(|i| doc.get(*i).unwrap().rect.x).collect();
        assert_eq!(xs, [0.0, 125.0, 250.0]);
        assert_eq!(doc.get(k[0]).unwrap().rect.y, 180.0);
    }

    #[test]
    fn wrap_breaks_lines() {
        let l = AutoLayout {
            wrap: true,
            gap: 10.0,
            cross_gap: 5.0,
            padding: [0.0; 4],
            ..Default::default()
        };
        let (doc, f, k) = doc_with(
            l,
            [Sizing::Fixed, Sizing::Hug],
            &[(120.0, 20.0, FX), (120.0, 30.0, FX), (120.0, 20.0, FX)],
        );
        assert_eq!(doc.get(k[2]).unwrap().rect.min(), v2(0.0, 35.0));
        assert_eq!(doc.get(f).unwrap().rect.h, 30.0 + 5.0 + 20.0);
    }

    /// A frame whose children are styled by `f`, laid out.
    fn css_frame(
        l: AutoLayout,
        size: (f64, f64),
        kids: usize,
        f: impl Fn(usize, &mut Node),
    ) -> (Document, Vec<NodeId>) {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let fid = tx.alloc_id();
        let mut frame = Node::new(
            fid,
            NodeKind::Frame { clip: false },
            Rect::new(0.0, 0.0, size.0, size.1),
        );
        frame.layout = Some(l);
        tx.insert(frame, None, None);
        let mut ids = Vec::new();
        for i in 0..kids {
            let id = tx.alloc_id();
            let mut n = Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0));
            f(i, &mut n);
            tx.insert(n, Some(fid), None);
            ids.push(id);
        }
        layout_frame(&mut tx, fid);
        (doc, ids)
    }

    const CSS: AutoLayout = AutoLayout {
        direction: Direction::Horizontal,
        wrap: false,
        gap: 0.0,
        gap_auto: false,
        cross_gap: 0.0,
        padding: [0.0; 4],
        align_main: Align::Start,
        align_cross: Align::Start,
        mode: LayoutMode::Flex,
        distribute: Distribute::Between,
        border: [0.0; 4],
    };

    #[test]
    fn percent_width_resolves_against_the_parent() {
        let (doc, k) = css_frame(CSS, (200.0, 50.0), 1, |_, n| {
            n.layout_child.percent[0] = Some(0.84);
        });
        assert_eq!(doc.get(k[0]).unwrap().rect.w, 168.0);
    }

    #[test]
    fn margins_push_siblings_and_auto_centers() {
        let (doc, k) = css_frame(CSS, (200.0, 50.0), 2, |i, n| {
            if i == 1 {
                n.layout_child.margin[3] = Some(Dim::Px(30.0));
            }
        });
        assert_eq!(doc.get(k[1]).unwrap().rect.x, 40.0);
        let (doc, k) = css_frame(CSS, (200.0, 50.0), 1, |_, n| {
            n.layout_child.margin[1] = Some(Dim::Auto);
            n.layout_child.margin[3] = Some(Dim::Auto);
        });
        assert_eq!(doc.get(k[0]).unwrap().rect.x, 95.0);
    }

    #[test]
    fn grid_makes_equal_columns_and_spans() {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let fid = tx.alloc_id();
        let mut f = Node::new(
            fid,
            NodeKind::Frame { clip: false },
            Rect::new(0.0, 0.0, 320.0, 100.0),
        );
        f.layout = Some(AutoLayout {
            mode: LayoutMode::Grid,
            gap: 10.0,
            ..CSS
        });
        f.grid = Some(crate::model::GridSpec {
            columns: "repeat(3, 1fr)".into(),
            rows: String::new(),
        });
        tx.insert(f, None, None);
        let mut ids = Vec::new();
        for i in 0..3 {
            let id = tx.alloc_id();
            let mut n = Node::new(id, NodeKind::Rect, Rect::new(0.0, 0.0, 10.0, 10.0));
            n.layout_child.sizing = [Sizing::Hug; 2];
            n.layout_child.sizing[0] = Sizing::Fill;
            n.layout = Some(CSS);
            if i == 2 {
                n.layout_child.grid_column = [GridPos::Span(2), GridPos::Auto];
            }
            tx.insert(n, Some(fid), None);
            ids.push(id);
        }
        layout_frame(&mut tx, fid);
        let r = |i: usize| tx.doc().get(ids[i]).unwrap().rect;
        assert_eq!((r(0).x, r(0).w), (0.0, 100.0));
        assert_eq!((r(1).x, r(1).w), (110.0, 100.0));
        assert_eq!((r(2).x, r(2).w), (0.0, 210.0), "second row, spanning two");
    }

    #[test]
    fn track_lists_parse_or_refuse() {
        for ok in [
            "1fr 2fr",
            "200px 1fr",
            "repeat(3, minmax(0, 1fr))",
            "repeat(auto-fit, 100px)",
        ] {
            assert!(parse_tracks(ok).is_some(), "{ok}");
        }
        for bad in ["", "wat", "repeat(x, 1fr)", "minmax(1fr)"] {
            assert!(parse_tracks(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn hidden_and_absolute_children_are_skipped() {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let fid = tx.alloc_id();
        let mut f = Node::new(
            fid,
            NodeKind::Frame { clip: true },
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        f.layout = Some(AutoLayout {
            padding: [0.0; 4],
            gap: 5.0,
            ..Default::default()
        });
        f.layout_child.sizing = [Sizing::Hug; 2];
        tx.insert(f, None, None);
        for (vis, abs) in [(true, false), (false, false), (true, true), (true, false)] {
            let id = tx.alloc_id();
            let mut n = Node::new(id, NodeKind::Rect, Rect::new(500.0, 500.0, 10.0, 10.0));
            n.visible = vis;
            n.layout_child.absolute = abs;
            tx.insert(n, Some(fid), None);
        }
        layout_frame(&mut tx, fid);
        assert_eq!(tx.doc().get(fid).unwrap().rect.w, 25.0);
        assert_eq!(
            tx.doc().get(NodeId(4)).unwrap().rect.x,
            500.0,
            "absolute untouched"
        );
    }

    #[test]
    fn constraints_on_resize() {
        let mut doc = Document::default();
        let mut rec = Recording::new("t", &doc);
        let mut tx = Tx::new(&mut doc, &mut rec);
        let fid = tx.alloc_id();
        tx.insert(
            Node::new(
                fid,
                NodeKind::Frame { clip: true },
                Rect::new(0.0, 0.0, 200.0, 100.0),
            ),
            None,
            None,
        );
        let mut ids = Vec::new();
        for c in [
            Constraint::Min,
            Constraint::Max,
            Constraint::Stretch,
            Constraint::Center,
            Constraint::Scale,
        ] {
            let id = tx.alloc_id();
            let mut n = Node::new(id, NodeKind::Rect, Rect::new(20.0, 10.0, 40.0, 20.0));
            n.layout_child.constraints = Constraints { h: c, v: c };
            tx.insert(n, Some(fid), None);
            ids.push(id);
        }
        let old = Rect::new(0.0, 0.0, 200.0, 100.0);
        let new = Rect::new(0.0, 0.0, 400.0, 200.0);
        tx.node_mut(fid).unwrap().rect = new;
        resize_children(&mut tx, fid, old, new);
        let r = |i: usize| tx.doc().get(ids[i]).unwrap().rect;
        assert_eq!(r(0), Rect::new(20.0, 10.0, 40.0, 20.0));
        assert_eq!(r(1), Rect::new(220.0, 110.0, 40.0, 20.0));
        assert_eq!(r(2), Rect::new(20.0, 10.0, 240.0, 120.0));
        assert_eq!(r(3), Rect::new(120.0, 60.0, 40.0, 20.0));
        assert_eq!(r(4), Rect::new(40.0, 20.0, 80.0, 40.0));
    }
}
