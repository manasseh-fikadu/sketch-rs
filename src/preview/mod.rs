//! The prototype preview: plays a project's frames like a website, across all of its pages.
//!
//! Nothing here writes to the editor's documents, history or view. The preview works on its own
//! copy of each page it shows, rebuilt from the original plus what's different right now (hovered
//! and pressed styles, scroll offsets of frames inside the screen). The scene cache only redoes
//! layers whose copy changed, and scrolling the page or sliding a screen only moves a camera, so
//! neither rebuilds geometry.
//!
//! Like the editor, it consumes [`InputEvent`]s, so every interaction can be scripted in tests.

pub mod render;

use crate::editor::{AppRequest, Button, Editor, InputEvent, Key};
use crate::geom::{Camera, Rect, V2, v2};
use crate::model::{
    Document, Node, NodeId, NodeKind, OverlayPlacement, Paint, ProtoAction, ScrollPosition,
    StateStyle, Target, Transition, TransitionKind, Trigger,
};
use std::collections::{HashMap, HashSet};

/// After leaving a hover-opened overlay's trigger, this long to reach the overlay itself.
const HOVER_GRACE_MS: f64 = 180.0;
const SMOOTH_SCROLL_MS: f64 = 320.0;
const KEY_SCROLL_PX: f64 = 48.0;
const DIM: f64 = 0.45;
pub const SCROLLBAR_MS: f64 = 900.0;
const HINT_MS: f64 = 500.0;
const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scale {
    /// Shrink to the window's width, like a browser's responsive view of a fixed design.
    #[default]
    Fit,
    /// Always 100%; wider frames scroll sideways.
    Actual,
}

/// A layer on a particular page (node ids are only unique per page).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub page: String,
    pub node: NodeId,
}

/// A frame being shown, and how far it is scrolled.
#[derive(Debug, Clone)]
pub struct Screen {
    pub page: String,
    pub frame: NodeId,
    /// Page scroll, in design px.
    pub scroll: V2,
    /// Scroll of frames inside this screen.
    pub inner: HashMap<NodeId, V2>,
    /// How we got here: Back plays it in reverse.
    arrived: Transition,
}

impl Screen {
    fn new(page: &str, frame: NodeId, arrived: Transition) -> Screen {
        Screen {
            page: page.to_string(),
            frame,
            scroll: V2::ZERO,
            inner: HashMap::new(),
            arrived,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Overlay {
    pub screen: Screen,
    placement: OverlayPlacement,
    dim: bool,
    close_outside: bool,
    transition: Transition,
    opened_at: f64,
    /// Where the layer that opened it was on screen (`Below` hangs from it).
    anchor: Option<Rect>,
    /// Hover-opened: stays while the pointer is on this layer or the overlay.
    hover_owner: Option<NodeRef>,
    left_at: Option<f64>,
}

impl Overlay {
    /// Clicks outside it stop at its backdrop.
    fn modal(&self) -> bool {
        self.dim || self.close_outside
    }
}

#[derive(Debug, Clone)]
struct Anim {
    from: Screen,
    transition: Transition,
    start: f64,
    back: bool,
}

#[derive(Debug, Clone, Copy)]
struct ScrollAnim {
    from: V2,
    to: V2,
    start: f64,
}

#[derive(Debug, Clone)]
struct Timer {
    at: f64,
    owner: NodeRef,
    /// Which interaction on the owner.
    index: usize,
    /// The frame whose screen armed it: it's cancelled when that screen goes away.
    scope: NodeRef,
}

#[derive(Debug, Clone)]
struct Press {
    chain: Vec<NodeRef>,
    /// The nearest layer in the chain with a click interaction.
    click: Option<NodeRef>,
}

/// What's under a point.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    /// `chain` runs from the deepest layer up to the frame, the way events bubble.
    Layer {
        page: String,
        chain: Vec<NodeId>,
        overlay: Option<usize>,
    },
    /// The backdrop of a modal overlay.
    Backdrop(usize),
    /// The empty stage around the frame.
    Stage,
}

/// A layer pinned against scrolling.
#[derive(Debug, Clone, Copy)]
pub struct Pinned {
    pub id: NodeId,
    pub position: ScrollPosition,
    /// Distance from the frame's top, in design px.
    pub top: f64,
}

impl Pinned {
    /// The scroll this layer is drawn with when the page is scrolled by `page`.
    pub fn scroll(&self, page: V2) -> V2 {
        match self.position {
            ScrollPosition::Fixed => V2::ZERO,
            _ => v2(page.x, page.y.min(self.top).max(0.0)),
        }
    }
}

/// The layers of `frame` that don't scroll with it, bottom to top. Anything inside one moves with
/// it.
pub fn pinned(doc: &Document, frame: NodeId) -> Vec<Pinned> {
    fn walk(doc: &Document, id: NodeId, top0: f64, out: &mut Vec<Pinned>) {
        let Some(n) = doc.get(id) else { return };
        for c in &n.children {
            let Some(k) = doc.get(*c).filter(|k| k.visible) else {
                continue;
            };
            let position = k
                .proto
                .as_ref()
                .map_or_else(Default::default, |p| p.position);
            if position == ScrollPosition::Scrolls {
                walk(doc, *c, top0, out);
            } else {
                let top = doc.world_rect(*c).map_or(0.0, |r| r.y - top0);
                out.push(Pinned {
                    id: *c,
                    position,
                    top,
                });
            }
        }
    }
    let mut out = Vec::new();
    let top0 = doc.world_rect(frame).map_or(0.0, |r| r.y);
    walk(doc, frame, top0, &mut out);
    out
}

/// A screen as it should be drawn right now.
#[derive(Debug, Clone)]
pub struct SceneView {
    pub page: String,
    pub frame: NodeId,
    pub frame_rect: Rect,
    pub zoom: f64,
    /// Where the frame's top-left is on the canvas at zero scroll.
    pub origin: V2,
    pub scroll: V2,
    pub alpha: f64,
    /// Strength of the backdrop behind an overlay.
    pub dim: Option<f64>,
    /// Takes input. False for a screen on its way out.
    pub live: bool,
    pub overlay: Option<usize>,
}

impl SceneView {
    pub fn camera(&self, scroll: V2) -> Camera {
        Camera {
            pan: self.frame_rect.min() + scroll - self.origin / self.zoom,
            zoom: self.zoom,
        }
    }

    /// The frame on the canvas with this much scroll.
    pub fn rect_at(&self, scroll: V2) -> Rect {
        Rect::new(
            self.origin.x - scroll.x * self.zoom,
            self.origin.y - scroll.y * self.zoom,
            self.frame_rect.w * self.zoom,
            self.frame_rect.h * self.zoom,
        )
    }
}

pub struct Preview {
    pub current: Screen,
    start: NodeRef,
    /// Screens Back returns to, oldest first.
    pub history: Vec<Screen>,
    /// Open overlays, bottom to top.
    pub overlays: Vec<Overlay>,
    anim: Option<Anim>,
    scroll_anim: Option<ScrollAnim>,
    timers: Vec<Timer>,
    docs: HashMap<String, Document>,
    source_stamp: u64,
    dirty: bool,
    /// Bumps whenever what's drawn changes other than by moving a camera.
    pub generation: u64,
    viewport: V2,
    now: f64,
    pointer: Option<V2>,
    /// The pointer may be over something new: the content moved under it.
    pointer_stale: bool,
    hover: Vec<NodeRef>,
    press: Option<Press>,
    /// A hotspot is under the pointer.
    pub cursor_hand: bool,
    pub scale: Scale,
    /// Keep hotspots outlined (otherwise they flash on a click that hit nothing).
    pub show_hotspots: bool,
    hint_until: f64,
    last_scroll: f64,
    /// The current screen has something to interact with.
    pub has_interactions: bool,
    /// Asks the editor to leave the preview; the string is why, if something went wrong.
    pub exit: Option<Option<String>>,
}

fn page_index(ed: &Editor, page: &str) -> Option<usize> {
    ed.pages.iter().position(|p| p.id == page)
}

fn source_node<'a>(ed: &'a Editor, r: &NodeRef) -> Option<&'a Node> {
    ed.page_doc(page_index(ed, &r.page)?).get(r.node)
}

/// Whether a link may leave the app. Designs come from files and agents, so a link never gets to
/// launch another app or open a local file.
pub fn is_safe_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:"]
        .iter()
        .any(|s| u.starts_with(s))
}

fn paints(n: &Node) -> bool {
    n.style.stroke.is_some()
        || n.style.borders.is_some()
        || n.style.fills.iter().any(|p| match p {
            Paint::Solid { color } => color.a > 0,
            _ => true,
        })
}

/// Whether a layer stops the pointer. Empty layout frames and drawn lines don't, so a transparent
/// container or a decorative arrow lying over a button can't make the button unclickable.
fn hittable(n: &Node) -> bool {
    if n.proto.is_some() {
        return true;
    }
    match n.kind {
        NodeKind::Line { .. }
        | NodeKind::Connector { .. }
        | NodeKind::Stroke { .. }
        | NodeKind::Group => false,
        NodeKind::Frame { .. } => paints(n),
        _ => true,
    }
}

/// The frontmost layer under `p` in a subtree, children before their parent.
fn deep_hit(
    doc: &Document,
    id: NodeId,
    p: V2,
    origin: V2,
    skip: &HashSet<NodeId>,
) -> Option<NodeId> {
    let n = doc.get(id)?;
    if !n.visible || skip.contains(&id) {
        return None;
    }
    let wr = n.rect.translate(origin);
    if matches!(n.kind, NodeKind::Frame { clip: true }) && !wr.contains(p) {
        return None;
    }
    for c in n.children.iter().rev() {
        if let Some(h) = deep_hit(doc, *c, p, wr.min(), skip) {
            return Some(h);
        }
    }
    (wr.contains(p) && hittable(n)).then_some(id)
}

fn chain_of(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut chain = vec![id];
    chain.extend(doc.ancestors(id));
    chain
}

fn apply_state(doc: &mut Document, id: NodeId, st: StateStyle) {
    if let Some(n) = doc.nodes.get_mut(&id) {
        st.apply(n);
    }
    if st.text.is_some() {
        for d in doc.subtree(id) {
            if let Some(n) = doc.nodes.get_mut(&d) {
                st.apply_text(n);
            }
        }
    }
}

/// How far `frame`'s content reaches past its box, in design px.
fn overflow_range(doc: &Document, frame: NodeId) -> V2 {
    let Some(n) = doc.get(frame) else {
        return V2::ZERO;
    };
    let mut r = v2(0.0, 0.0);
    for c in &n.children {
        if let Some(k) = doc.get(*c).filter(|k| k.visible) {
            r.x = r.x.max(k.rect.right());
            r.y = r.y.max(k.rect.bottom());
        }
    }
    v2((r.x - n.rect.w).max(0.0), (r.y - n.rect.h).max(0.0))
}

fn clamp_v(p: V2, max: V2) -> V2 {
    v2(
        p.x.clamp(0.0, max.x.max(0.0)),
        p.y.clamp(0.0, max.y.max(0.0)),
    )
}

impl Preview {
    pub fn new(ed: &Editor, page: &str, frame: NodeId, now: f64) -> Preview {
        let mut p = Preview {
            current: Screen::new(page, frame, Transition::default()),
            start: NodeRef {
                page: page.to_string(),
                node: frame,
            },
            history: Vec::new(),
            overlays: Vec::new(),
            anim: None,
            scroll_anim: None,
            timers: Vec::new(),
            docs: HashMap::new(),
            source_stamp: 0,
            dirty: true,
            generation: 0,
            viewport: ed.viewport,
            now,
            pointer: None,
            pointer_stale: false,
            hover: Vec::new(),
            press: None,
            cursor_hand: false,
            scale: Scale::default(),
            show_hotspots: false,
            hint_until: 0.0,
            last_scroll: f64::NEG_INFINITY,
            has_interactions: false,
            exit: None,
        };
        p.arm_timers(ed, page, frame);
        p.sync(ed);
        p
    }

    // -------------------------------------------------------------------------- reading

    /// The page copy being shown (see the module docs).
    pub fn doc(&self, page: &str) -> Option<&Document> {
        self.docs.get(page)
    }

    pub fn now(&self) -> f64 {
        self.now
    }

    pub fn viewport(&self) -> V2 {
        self.viewport
    }

    /// Hotspot outlines are showing.
    pub fn hints_visible(&self) -> bool {
        self.show_hotspots || self.now < self.hint_until
    }

    /// How visible the page's scroll indicator is (0..1).
    pub fn scrollbar_alpha(&self) -> f64 {
        let age = self.now - self.last_scroll;
        if age < 0.0 || age >= SCROLLBAR_MS {
            return 0.0;
        }
        ((SCROLLBAR_MS - age) / (SCROLLBAR_MS * 0.4)).min(1.0)
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty() || !self.overlays.is_empty()
    }

    /// Something moves by itself, so frames must keep coming.
    pub fn animating(&self, now: f64) -> bool {
        self.anim.is_some()
            || self.scroll_anim.is_some()
            || self
                .overlays
                .iter()
                .any(|o| o.transition.animated() && now - o.opened_at < o.transition.ms)
            || now < self.hint_until
            || (now - self.last_scroll) < SCROLLBAR_MS
            || self.pointer_stale
    }

    /// Milliseconds until something scheduled happens (a delayed action, a hover overlay closing).
    pub fn next_wake(&self, now: f64) -> Option<f64> {
        let timers = self.timers.iter().map(|t| t.at);
        let leaving = self
            .overlays
            .iter()
            .filter_map(|o| o.left_at.map(|t| t + HOVER_GRACE_MS));
        timers
            .chain(leaving)
            .fold(None, |a: Option<f64>, t| Some(a.map_or(t, |a| a.min(t))))
            .map(|t| (t - now).max(0.0))
    }

    fn zoom_for(&self, width: f64) -> f64 {
        match self.scale {
            Scale::Fit if width > 0.0 => (self.viewport.x / width).clamp(0.05, 1.0),
            _ => 1.0,
        }
    }

    fn base_view(&self, s: &Screen, delta: V2, alpha: f64, live: bool) -> Option<SceneView> {
        let frame_rect = self.docs.get(&s.page)?.world_rect(s.frame)?;
        let zoom = self.zoom_for(frame_rect.w);
        let x = ((self.viewport.x - frame_rect.w * zoom) * 0.5)
            .max(0.0)
            .floor();
        Some(SceneView {
            page: s.page.clone(),
            frame: s.frame,
            frame_rect,
            zoom,
            origin: v2(x, 0.0) + delta,
            scroll: s.scroll,
            alpha,
            dim: None,
            live,
            overlay: None,
        })
    }

    fn overlay_view(&self, i: usize, o: &Overlay) -> Option<SceneView> {
        let frame_rect = self.docs.get(&o.screen.page)?.world_rect(o.screen.frame)?;
        // Overlays are drawn at the scale of the screen they open over.
        let zoom = self
            .base_view(&self.current, V2::ZERO, 1.0, true)
            .map_or(1.0, |v| v.zoom);
        let vp = self.viewport;
        let mx = (vp.x - frame_rect.w * zoom).max(0.0);
        let my = (vp.y - frame_rect.h * zoom).max(0.0);
        let pos = match o.placement {
            OverlayPlacement::Center => v2(mx * 0.5, my * 0.5),
            OverlayPlacement::TopLeft => v2(0.0, 0.0),
            OverlayPlacement::TopCenter => v2(mx * 0.5, 0.0),
            OverlayPlacement::TopRight => v2(mx, 0.0),
            OverlayPlacement::BottomLeft => v2(0.0, my),
            OverlayPlacement::BottomCenter => v2(mx * 0.5, my),
            OverlayPlacement::BottomRight => v2(mx, my),
            OverlayPlacement::Below => o
                .anchor
                .map_or(v2(mx * 0.5, 0.0), |a| v2(a.x.min(mx), a.bottom().min(my))),
        };
        let t = o.transition;
        let e = if t.animated() {
            t.easing.apply((self.now - o.opened_at) / t.ms)
        } else {
            1.0
        };
        let entry = t.dir.entry();
        let delta = match t.kind {
            TransitionKind::MoveIn | TransitionKind::Push => {
                v2(entry.x * vp.x, entry.y * vp.y) * (1.0 - e)
            }
            _ => V2::ZERO,
        };
        Some(SceneView {
            page: o.screen.page.clone(),
            frame: o.screen.frame,
            frame_rect,
            zoom,
            origin: pos + delta,
            scroll: o.screen.scroll,
            alpha: if t.kind == TransitionKind::Dissolve {
                e
            } else {
                1.0
            },
            dim: o.dim.then_some(DIM * e),
            live: true,
            overlay: Some(i),
        })
    }

    /// Everything on screen, bottom to top.
    pub fn views(&self) -> Vec<SceneView> {
        let mut out = Vec::new();
        match &self.anim {
            Some(a) => {
                let e = a
                    .transition
                    .easing
                    .apply((self.now - a.start) / a.transition.ms);
                let entry = a.transition.dir.entry();
                let span = v2(entry.x * self.viewport.x, entry.y * self.viewport.y);
                let z = V2::ZERO;
                // (old screen's offset and opacity, new screen's, old screen on top)
                let (fd, fa, td, ta, old_on_top) = match (a.transition.kind, a.back) {
                    (TransitionKind::Dissolve, false) => (z, 1.0, z, e, false),
                    (TransitionKind::Dissolve, true) => (z, 1.0 - e, z, 1.0, true),
                    (TransitionKind::MoveIn, false) => (z, 1.0, span * (1.0 - e), 1.0, false),
                    (TransitionKind::MoveIn, true) => (span * e, 1.0, z, 1.0, true),
                    (TransitionKind::Push, false) => (-span * e, 1.0, span * (1.0 - e), 1.0, false),
                    (TransitionKind::Push, true) => (span * e, 1.0, -span * (1.0 - e), 1.0, false),
                    (TransitionKind::Instant, _) => (z, 0.0, z, 1.0, false),
                };
                let old = self.base_view(&a.from, fd, fa, false);
                let new = self.base_view(&self.current, td, ta, true);
                let (first, second) = if old_on_top { (new, old) } else { (old, new) };
                out.extend(first);
                out.extend(second);
            }
            None => out.extend(self.base_view(&self.current, V2::ZERO, 1.0, true)),
        }
        for (i, o) in self.overlays.iter().enumerate() {
            out.extend(self.overlay_view(i, o));
        }
        out
    }

    /// What's under a canvas position.
    pub fn pick(&self, pos: V2) -> Hit {
        for v in self.views().iter().rev().filter(|v| v.live) {
            let Some(doc) = self.docs.get(&v.page) else {
                continue;
            };
            let layer = |id: NodeId| Hit::Layer {
                page: v.page.clone(),
                chain: chain_of(doc, id),
                overlay: v.overlay,
            };
            let pins = pinned(doc, v.frame);
            for g in pins.iter().rev() {
                let world = v.camera(g.scroll(v.scroll)).screen_to_world(pos);
                let origin = doc.parent_origin(doc.get(g.id).and_then(|n| n.parent));
                if let Some(h) = deep_hit(doc, g.id, world, origin, &HashSet::new()) {
                    return layer(h);
                }
            }
            let skip: HashSet<NodeId> = pins.iter().map(|g| g.id).collect();
            let world = v.camera(v.scroll).screen_to_world(pos);
            if v.frame_rect.contains(world) {
                let h = deep_hit(doc, v.frame, world, V2::ZERO, &skip).unwrap_or(v.frame);
                return layer(h);
            }
            match v.overlay {
                Some(i) if self.overlays[i].modal() => return Hit::Backdrop(i),
                Some(_) => {}
                None => return Hit::Stage,
            }
        }
        Hit::Stage
    }

    /// Where a layer is on the canvas now.
    pub fn node_screen_rect(&self, r: &NodeRef) -> Option<Rect> {
        for v in self.views().iter().rev().filter(|v| v.page == r.page) {
            let doc = self.docs.get(&v.page)?;
            if r.node != v.frame && !doc.is_ancestor(v.frame, r.node) {
                continue;
            }
            let pins = pinned(doc, v.frame);
            let pin = pins
                .iter()
                .find(|g| g.id == r.node || doc.is_ancestor(g.id, r.node));
            let scroll = pin.map_or(v.scroll, |g| g.scroll(v.scroll));
            return Some(
                v.camera(scroll)
                    .world_rect_to_screen(&doc.world_rect(r.node)?),
            );
        }
        None
    }

    fn max_scroll(&self, v: &SceneView) -> V2 {
        let vp = self.viewport;
        let (w, h) = (v.frame_rect.w * v.zoom, v.frame_rect.h * v.zoom);
        // An overlay placed partway down the window has less room than the window.
        let top = if v.overlay.is_some() {
            v.origin.y.max(0.0)
        } else {
            0.0
        };
        v2(
            (w - vp.x).max(0.0) / v.zoom,
            (h + top - vp.y).max(0.0) / v.zoom,
        )
    }

    // -------------------------------------------------------------------------- keeping up

    /// Bring the page copies and scroll limits up to date. Call before drawing.
    pub fn sync(&mut self, ed: &Editor) {
        if self.viewport != ed.viewport {
            self.viewport = ed.viewport;
            self.pointer_stale = true;
            self.clamp_scroll();
        }
        if ed.doc_stamp != self.source_stamp {
            // An agent (or undo over MCP) edited something: show it, keeping our place.
            self.source_stamp = ed.doc_stamp;
            self.dirty = true;
        }
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.generation += 1;
        let exists = |s: &Screen| {
            page_index(ed, &s.page)
                .is_some_and(|i| ed.page_doc(i).get(s.frame).is_some_and(|n| n.is_frame()))
        };
        if !exists(&self.current) {
            self.exit = Some(Some("The frame being previewed was deleted".into()));
            return;
        }
        self.overlays.retain(|o| exists(&o.screen));
        if self.anim.as_ref().is_some_and(|a| !exists(&a.from)) {
            self.anim = None;
        }
        let mut pages: Vec<String> = vec![self.current.page.clone()];
        pages.extend(self.overlays.iter().map(|o| o.screen.page.clone()));
        pages.extend(self.anim.as_ref().map(|a| a.from.page.clone()));
        pages.sort();
        pages.dedup();
        self.docs.retain(|k, _| pages.contains(k));
        for page in pages {
            let Some(i) = page_index(ed, &page) else {
                continue;
            };
            let mut doc = ed.page_doc(i).clone();
            self.patch(&page, &mut doc);
            self.docs.insert(page, doc);
        }
        self.has_interactions = self.docs.get(&self.current.page).is_some_and(|d| {
            d.subtree(self.current.frame).iter().any(|id| {
                d.get(*id)
                    .and_then(|n| n.proto.as_ref())
                    .is_some_and(|p| !p.interactions.is_empty())
            })
        });
        self.clamp_scroll();
    }

    /// Apply what's different right now onto a fresh copy of a page.
    fn patch(&self, page: &str, doc: &mut Document) {
        let screens = std::iter::once(&self.current)
            .chain(self.overlays.iter().map(|o| &o.screen))
            .chain(self.anim.iter().map(|a| &a.from));
        for s in screens.filter(|s| s.page == page) {
            for (frame, off) in &s.inner {
                let kids = doc
                    .get(*frame)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                for k in kids {
                    if let Some(n) = doc.nodes.get_mut(&k) {
                        n.rect = n.rect.translate(-*off);
                    }
                }
            }
        }
        // Outermost first, so a nested layer's state wins inside it.
        for r in self.hover.iter().rev().filter(|r| r.page == page) {
            let st = doc
                .get(r.node)
                .and_then(|n| n.proto.as_ref())
                .and_then(|p| p.hover);
            if let Some(st) = st {
                apply_state(doc, r.node, st);
            }
        }
        if let Some(press) = &self.press {
            for r in press.chain.iter().rev().filter(|r| r.page == page) {
                let st = doc
                    .get(r.node)
                    .and_then(|n| n.proto.as_ref())
                    .and_then(|p| p.pressed);
                if let Some(st) = st {
                    apply_state(doc, r.node, st);
                }
            }
        }
    }

    fn clamp_scroll(&mut self) {
        let base = self.base_view(&self.current, V2::ZERO, 1.0, true);
        if let Some(v) = base {
            self.current.scroll = clamp_v(self.current.scroll, self.max_scroll(&v));
        }
        for i in 0..self.overlays.len() {
            if let Some(v) = self.overlay_view(i, &self.overlays[i]) {
                let max = self.max_scroll(&v);
                let o = &mut self.overlays[i];
                o.screen.scroll = clamp_v(o.screen.scroll, max);
            }
        }
    }

    /// Advance time: finish animations, fire what's due.
    pub fn tick(&mut self, ed: &mut Editor, now: f64) {
        self.now = self.now.max(now);
        if let Some(a) = &self.anim
            && self.now - a.start >= a.transition.ms
        {
            self.anim = None;
            self.dirty = true;
        }
        if let Some(s) = self.scroll_anim {
            let t = ((self.now - s.start) / SMOOTH_SCROLL_MS).min(1.0);
            let e = crate::model::Easing::EaseInOut.apply(t);
            self.current.scroll = s.from.lerp(s.to, e);
            self.last_scroll = self.now;
            self.pointer_stale = true;
            if t >= 1.0 {
                self.scroll_anim = None;
            }
        }
        let due: Vec<Timer> = self
            .timers
            .iter()
            .filter(|t| t.at <= self.now)
            .cloned()
            .collect();
        self.timers.retain(|t| t.at > self.now);
        for t in due {
            let action = source_node(ed, &t.owner)
                .and_then(|n| n.interactions().get(t.index))
                .map(|i| i.action.clone());
            if let Some(a) = action {
                self.perform(ed, &t.owner, &a, false);
            }
        }
        let closing: Vec<usize> = self
            .overlays
            .iter()
            .enumerate()
            .filter(|(_, o)| o.left_at.is_some_and(|t| self.now - t >= HOVER_GRACE_MS))
            .map(|(i, _)| i)
            .collect();
        for i in closing.into_iter().rev() {
            self.close_overlay_at(i);
        }
        self.sync(ed);
        if self.pointer_stale {
            self.pointer_stale = false;
            self.update_hover(ed);
            self.sync(ed);
        }
    }

    // -------------------------------------------------------------------------- input

    pub fn handle(&mut self, ed: &mut Editor, ev: InputEvent) {
        self.sync(ed);
        match ev {
            InputEvent::PointerMove { pos, time_ms, .. } => {
                self.now = self.now.max(time_ms);
                self.pointer = Some(pos);
                self.update_hover(ed);
            }
            InputEvent::PointerDown {
                pos,
                button: Button::Primary,
                time_ms,
                ..
            } => {
                self.now = self.now.max(time_ms);
                self.pointer = Some(pos);
                self.pointer_down(pos);
            }
            InputEvent::PointerUp {
                pos,
                button: Button::Primary,
                time_ms,
            } => {
                self.now = self.now.max(time_ms);
                self.pointer = Some(pos);
                self.pointer_up(ed, pos);
            }
            InputEvent::Scroll { delta, pos } => self.wheel(ed, delta, pos),
            InputEvent::KeyDown { key, .. } => self.key(ed, key),
            _ => {}
        }
        self.sync(ed);
    }

    /// The pointer went off the canvas. Returns whether that changed anything.
    pub fn pointer_left(&mut self) -> bool {
        self.pointer = None;
        let changed = !self.hover.is_empty() || self.cursor_hand;
        if !self.hover.is_empty() {
            self.hover.clear();
            self.dirty = true;
        }
        self.cursor_hand = false;
        let now = self.now;
        for o in &mut self.overlays {
            if o.hover_owner.is_some() {
                o.left_at.get_or_insert(now);
            }
        }
        changed
    }

    fn hover_styled(&self, chain: &[NodeRef]) -> Vec<NodeRef> {
        chain
            .iter()
            .filter(|r| {
                self.docs
                    .get(&r.page)
                    .and_then(|d| d.get(r.node))
                    .and_then(|n| n.proto.as_ref())
                    .is_some_and(|p| p.hover.is_some())
            })
            .cloned()
            .collect()
    }

    fn update_hover(&mut self, ed: &mut Editor) {
        let Some(pos) = self.pointer else { return };
        let hit = self.pick(pos);
        let (chain, over) = match &hit {
            Hit::Layer {
                page,
                chain,
                overlay,
            } => (
                chain
                    .iter()
                    .map(|n| NodeRef {
                        page: page.clone(),
                        node: *n,
                    })
                    .collect(),
                *overlay,
            ),
            _ => (Vec::new(), None),
        };
        let entered: Vec<NodeRef> = chain
            .iter()
            .filter(|r: &&NodeRef| !self.hover.contains(r))
            .cloned()
            .collect();
        if self.hover_styled(&chain) != self.hover_styled(&self.hover) {
            self.dirty = true;
        }
        self.cursor_hand = chain.iter().any(|r| {
            self.docs
                .get(&r.page)
                .and_then(|d| d.get(r.node))
                .and_then(|n| n.proto.as_ref())
                .is_some_and(|p| p.has_click())
        });
        self.hover = chain;
        let now = self.now;
        for (i, o) in self.overlays.iter_mut().enumerate() {
            let Some(owner) = &o.hover_owner else {
                continue;
            };
            // On the trigger, on this overlay, or on one opened from it.
            if self.hover.contains(owner) || over.is_some_and(|h| h >= i) {
                o.left_at = None;
            } else {
                o.left_at.get_or_insert(now);
            }
        }
        for r in entered {
            let hovers: Vec<ProtoAction> = source_node(ed, &r)
                .map(|n| {
                    n.interactions()
                        .iter()
                        .filter(|i| i.trigger == Trigger::Hover)
                        .map(|i| i.action.clone())
                        .collect()
                })
                .unwrap_or_default();
            for a in hovers {
                self.perform(ed, &r, &a, true);
            }
        }
    }

    fn pointer_down(&mut self, pos: V2) {
        match self.pick(pos) {
            Hit::Layer { page, chain, .. } => {
                let refs: Vec<NodeRef> = chain
                    .iter()
                    .map(|n| NodeRef {
                        page: page.clone(),
                        node: *n,
                    })
                    .collect();
                let protos = |r: &NodeRef| {
                    self.docs
                        .get(&r.page)
                        .and_then(|d| d.get(r.node))
                        .and_then(|n| n.proto.as_ref())
                };
                let click = refs
                    .iter()
                    .find(|r| protos(r).is_some_and(|p| p.has_click()))
                    .cloned();
                if refs
                    .iter()
                    .any(|r| protos(r).is_some_and(|p| p.pressed.is_some()))
                {
                    self.dirty = true;
                }
                if click.is_none() {
                    self.flash_hints();
                }
                self.press = Some(Press { chain: refs, click });
            }
            Hit::Backdrop(i) => {
                self.press = None;
                if self.overlays[i].close_outside {
                    while self.overlays.len() > i {
                        self.close_overlay_at(self.overlays.len() - 1);
                    }
                }
            }
            Hit::Stage => {
                self.press = None;
                self.flash_hints();
            }
        }
    }

    fn pointer_up(&mut self, ed: &mut Editor, pos: V2) {
        let Some(press) = self.press.take() else {
            return;
        };
        self.dirty = true;
        let Some(click) = press.click else { return };
        // Like a browser: a click only counts if the pointer is still on the same layer.
        let still = match self.pick(pos) {
            Hit::Layer { page, chain, .. } => page == click.page && chain.contains(&click.node),
            _ => false,
        };
        if still {
            self.click(ed, &click);
        }
    }

    fn flash_hints(&mut self) {
        self.hint_until = self.now + HINT_MS;
    }

    /// Run a layer's click interactions.
    pub fn click(&mut self, ed: &mut Editor, r: &NodeRef) {
        let actions: Vec<ProtoAction> = source_node(ed, r)
            .map(|n| {
                n.interactions()
                    .iter()
                    .filter(|i| i.trigger == Trigger::Click)
                    .map(|i| i.action.clone())
                    .collect()
            })
            .unwrap_or_default();
        for a in actions {
            self.perform(ed, r, &a, false);
        }
    }

    fn wheel(&mut self, ed: &Editor, delta: V2, pos: V2) {
        self.scroll_anim = None;
        let hit = self.pick(pos);
        let views = self.views();
        let zoom = views.iter().rev().find(|v| v.live).map_or(1.0, |v| v.zoom);
        if let Hit::Layer {
            page,
            chain,
            overlay,
        } = &hit
        {
            // The innermost frame that can still move this way takes the scroll.
            if let Some(doc) = page_index(ed, page).map(|i| ed.page_doc(i)) {
                for id in chain {
                    let Some(n) = doc.get(*id).filter(|n| n.proto.is_some()) else {
                        continue;
                    };
                    let ov = n.proto.as_ref().unwrap().overflow;
                    let d = v2(
                        if ov.x() { delta.x / zoom } else { 0.0 },
                        if ov.y() { delta.y / zoom } else { 0.0 },
                    );
                    if d == V2::ZERO {
                        continue;
                    }
                    let screen = match overlay {
                        Some(i) => &mut self.overlays[*i].screen,
                        None => &mut self.current,
                    };
                    let cur = screen.inner.get(id).copied().unwrap_or(V2::ZERO);
                    let next = clamp_v(cur + d, overflow_range(doc, *id));
                    if next != cur {
                        screen.inner.insert(*id, next);
                        self.dirty = true;
                        self.pointer_stale = true;
                        return;
                    }
                }
            }
            if let Some(i) = overlay {
                if let Some(v) = self.overlay_view(*i, &self.overlays[*i]) {
                    let max = self.max_scroll(&v);
                    let o = &mut self.overlays[*i];
                    let next = clamp_v(o.screen.scroll + delta / v.zoom, max);
                    if next != o.screen.scroll {
                        o.screen.scroll = next;
                        self.pointer_stale = true;
                        return;
                    }
                }
                if self.overlays[*i].modal() {
                    return;
                }
            }
        }
        if matches!(hit, Hit::Backdrop(_)) {
            return;
        }
        self.scroll_page_by(delta);
    }

    /// Scroll the page by screen pixels.
    fn scroll_page_by(&mut self, delta: V2) {
        let Some(v) = self.base_view(&self.current, V2::ZERO, 1.0, true) else {
            return;
        };
        let next = clamp_v(self.current.scroll + delta / v.zoom, self.max_scroll(&v));
        if next != self.current.scroll {
            self.current.scroll = next;
            self.last_scroll = self.now;
            self.pointer_stale = true;
        }
    }

    fn smooth_scroll_to(&mut self, to: V2) {
        let Some(v) = self.base_view(&self.current, V2::ZERO, 1.0, true) else {
            return;
        };
        let to = clamp_v(to, self.max_scroll(&v));
        if to != self.current.scroll {
            self.scroll_anim = Some(ScrollAnim {
                from: self.current.scroll,
                to,
                start: self.now,
            });
        }
    }

    fn key(&mut self, ed: &mut Editor, key: Key) {
        let page_h = self
            .base_view(&self.current, V2::ZERO, 1.0, true)
            .map_or(self.viewport.y, |v| {
                (self.viewport.y - KEY_SCROLL_PX) / v.zoom
            });
        let scroll = self.current.scroll;
        match key {
            Key::Escape => {
                if self.overlays.is_empty() {
                    self.exit = Some(None);
                } else {
                    self.close_overlay_at(self.overlays.len() - 1);
                }
            }
            Key::Left | Key::Backspace => self.back(ed),
            Key::Char('r' | 'R') => self.restart(ed),
            Key::Up => self.scroll_page_by(v2(0.0, -KEY_SCROLL_PX)),
            Key::Down => self.scroll_page_by(v2(0.0, KEY_SCROLL_PX)),
            Key::PageUp => self.smooth_scroll_to(scroll - v2(0.0, page_h)),
            Key::PageDown | Key::Space => self.smooth_scroll_to(scroll + v2(0.0, page_h)),
            Key::Home => self.smooth_scroll_to(v2(scroll.x, 0.0)),
            Key::End => self.smooth_scroll_to(v2(scroll.x, f64::MAX)),
            _ => {}
        }
    }

    // -------------------------------------------------------------------------- actions

    fn perform(&mut self, ed: &mut Editor, owner: &NodeRef, action: &ProtoAction, hover: bool) {
        match action {
            ProtoAction::Navigate { target, transition } => {
                self.navigate(ed, &owner.page, target, *transition);
            }
            ProtoAction::Back => self.back(ed),
            ProtoAction::OpenOverlay {
                target,
                placement,
                dim,
                close_outside,
                transition,
            } => self.open_overlay(
                ed,
                owner,
                target,
                *placement,
                *dim,
                *close_outside,
                *transition,
                hover,
            ),
            ProtoAction::CloseOverlay => {
                if !self.overlays.is_empty() {
                    self.close_overlay_at(self.overlays.len() - 1);
                }
            }
            ProtoAction::OpenUrl { url } => {
                if is_safe_url(url) {
                    ed.requests
                        .push(AppRequest::OpenUrl(url.trim().to_string()));
                } else {
                    ed.status =
                        format!("Not opening {url}: links must be http, https, mailto or tel");
                }
            }
        }
    }

    fn arm_timers(&mut self, ed: &Editor, page: &str, frame: NodeId) {
        let Some(i) = page_index(ed, page) else {
            return;
        };
        let doc = ed.page_doc(i);
        let scope = NodeRef {
            page: page.to_string(),
            node: frame,
        };
        for id in doc.subtree(frame) {
            let Some(n) = doc.get(id) else { continue };
            for (index, it) in n.interactions().iter().enumerate() {
                if let Trigger::Delay { ms } = it.trigger {
                    self.timers.push(Timer {
                        at: self.now + ms,
                        owner: NodeRef {
                            page: page.to_string(),
                            node: id,
                        },
                        index,
                        scope: scope.clone(),
                    });
                }
            }
        }
    }

    /// Everything tied to the screen being left.
    fn leave_screen(&mut self) {
        self.overlays.clear();
        self.timers.clear();
        self.hover.clear();
        self.press = None;
        self.scroll_anim = None;
        self.cursor_hand = false;
        self.pointer_stale = true;
        self.dirty = true;
    }

    /// Follow a link from `from_page`. A link to a layer inside a frame opens that frame scrolled
    /// to the layer; on the frame already showing, it just scrolls there.
    pub fn navigate(
        &mut self,
        ed: &Editor,
        from_page: &str,
        target: &Target,
        transition: Transition,
    ) {
        let Some(to) = ed.resolve_target(from_page, target) else {
            return;
        };
        let doc = ed.page_doc(to.page_index);
        let y = section_offset(doc, to.frame, to.node);
        if to.page == self.current.page && to.frame == self.current.frame {
            self.overlays.clear();
            self.dirty = true;
            self.smooth_scroll_to(v2(self.current.scroll.x, y));
            return;
        }
        let mut next = Screen::new(&to.page, to.frame, transition);
        next.scroll = v2(0.0, y);
        self.leave_screen();
        self.anim = transition.animated().then(|| Anim {
            from: self.current.clone(),
            transition,
            start: self.now,
            back: false,
        });
        let prev = std::mem::replace(&mut self.current, next);
        self.history.push(prev);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.arm_timers(ed, &to.page, to.frame);
    }

    pub fn back(&mut self, ed: &Editor) {
        if !self.overlays.is_empty() {
            self.close_overlay_at(self.overlays.len() - 1);
            return;
        }
        while let Some(prev) = self.history.pop() {
            let exists = page_index(ed, &prev.page)
                .is_some_and(|i| ed.page_doc(i).get(prev.frame).is_some());
            if !exists {
                continue;
            }
            let t = self.current.arrived;
            self.leave_screen();
            self.anim = t.animated().then(|| Anim {
                from: self.current.clone(),
                transition: t,
                start: self.now,
                back: true,
            });
            let (page, frame) = (prev.page.clone(), prev.frame);
            self.current = prev;
            self.arm_timers(ed, &page, frame);
            return;
        }
    }

    /// Back to the screen the preview started on.
    pub fn restart(&mut self, ed: &Editor) {
        self.leave_screen();
        self.history.clear();
        self.anim = None;
        let start = self.start.clone();
        self.current = Screen::new(&start.page, start.node, Transition::default());
        self.arm_timers(ed, &start.page, start.node);
    }

    pub fn set_scale(&mut self, scale: Scale) {
        self.scale = scale;
        self.pointer_stale = true;
        self.clamp_scroll();
    }

    #[allow(clippy::too_many_arguments)]
    fn open_overlay(
        &mut self,
        ed: &Editor,
        owner: &NodeRef,
        target: &Target,
        placement: OverlayPlacement,
        dim: bool,
        close_outside: bool,
        transition: Transition,
        hover: bool,
    ) {
        let Some(to) = ed.resolve_target(&owner.page, target) else {
            return;
        };
        if let Some(i) = self
            .overlays
            .iter()
            .position(|o| o.screen.page == to.page && o.screen.frame == to.frame)
        {
            // Hovering again just keeps it; clicking its button again closes it, like a menu.
            if hover {
                self.overlays[i].left_at = None;
            } else {
                while self.overlays.len() > i {
                    self.close_overlay_at(self.overlays.len() - 1);
                }
            }
            return;
        }
        let anchor = self.node_screen_rect(owner);
        self.overlays.push(Overlay {
            screen: Screen::new(&to.page, to.frame, transition),
            placement,
            // Something opened by hovering can't block what's behind it.
            dim: dim && !hover,
            close_outside: close_outside && !hover,
            transition,
            opened_at: self.now,
            anchor,
            hover_owner: hover.then(|| owner.clone()),
            left_at: None,
        });
        self.dirty = true;
        self.arm_timers(ed, &to.page, to.frame);
    }

    fn close_overlay_at(&mut self, i: usize) {
        if i >= self.overlays.len() {
            return;
        }
        let o = self.overlays.remove(i);
        let scope = NodeRef {
            page: o.screen.page,
            node: o.screen.frame,
        };
        self.timers.retain(|t| t.scope != scope);
        self.pointer_stale = true;
        self.dirty = true;
    }
}

/// How far to scroll `frame` to show `node`, leaving room for any header pinned at the top.
fn section_offset(doc: &Document, frame: NodeId, node: NodeId) -> f64 {
    if node == frame {
        return 0.0;
    }
    let (Some(fr), Some(r)) = (doc.world_rect(frame), doc.world_rect(node)) else {
        return 0.0;
    };
    let y = r.y - fr.y;
    let inset = pinned(doc, frame)
        .iter()
        .filter_map(|g| {
            let h = doc.world_rect(g.id)?.h;
            match g.position {
                ScrollPosition::Fixed if g.top < 8.0 => Some(g.top + h),
                ScrollPosition::Sticky if g.top <= y => Some(h),
                _ => None,
            }
        })
        .fold(0.0, f64::max);
    (y - inset).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_may_leave_the_app() {
        for ok in [
            "https://a.com",
            "http://a.com",
            " HTTPS://A.COM ",
            "mailto:a@b.c",
            "tel:+251",
        ] {
            assert!(is_safe_url(ok), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "vscode://x",
            "/Applications/Calculator.app",
            "",
        ] {
            assert!(!is_safe_url(bad), "{bad}");
        }
    }

    #[test]
    fn pinned_layers_scroll_until_they_should_stop() {
        let sticky = Pinned {
            id: NodeId(1),
            position: ScrollPosition::Sticky,
            top: 100.0,
        };
        assert_eq!(sticky.scroll(v2(0.0, 40.0)).y, 40.0, "still scrolling");
        assert_eq!(sticky.scroll(v2(0.0, 400.0)).y, 100.0, "stuck at the top");
        let fixed = Pinned {
            position: ScrollPosition::Fixed,
            ..sticky
        };
        assert_eq!(fixed.scroll(v2(0.0, 400.0)), V2::ZERO);
    }
}
