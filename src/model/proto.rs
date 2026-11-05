//! Prototype data: how a layer behaves when the design is previewed like a real site.
//!
//! It lives on [`Node::proto`](super::Node) and only the preview (`crate::preview`), the
//! Prototype panel and MCP read it, so exports and the canvas are unaffected. The field is
//! optional and skipped when empty, which keeps page files without prototypes byte-identical.

use super::node::{Color, Node, NodeId, NodeKind};
use crate::geom::V2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn yes() -> bool {
    true
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

/// A link target: any layer on any page. A top-level frame opens that screen; a layer inside a
/// frame opens its screen scrolled to the layer, like a `#section` link.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target {
    /// Page id. `None` means the page the link is on, so links inside a duplicated page point
    /// at the duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    pub node: NodeId,
}

impl Target {
    pub fn same_page(node: NodeId) -> Target {
        Target { page: None, node }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    #[default]
    Instant,
    Dissolve,
    /// The new screen slides in over the old one.
    MoveIn,
    /// The new screen pushes the old one out.
    Push,
}

/// Which way the new screen travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideDir {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl SlideDir {
    /// Offset, as a multiple of the viewport, that a screen entering this way starts from.
    pub fn entry(self) -> V2 {
        match self {
            SlideDir::Left => crate::geom::v2(1.0, 0.0),
            SlideDir::Right => crate::geom::v2(-1.0, 0.0),
            SlideDir::Up => crate::geom::v2(0.0, 1.0),
            SlideDir::Down => crate::geom::v2(0.0, -1.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    #[default]
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    pub dir: SlideDir,
    pub ms: f64,
    pub easing: Easing,
}

impl Default for Transition {
    fn default() -> Self {
        Transition {
            kind: TransitionKind::Instant,
            dir: SlideDir::Left,
            ms: 300.0,
            easing: Easing::EaseOut,
        }
    }
}

impl Transition {
    pub fn animated(&self) -> bool {
        self.kind != TransitionKind::Instant && self.ms > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPlacement {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// Directly under the layer that opened it (dropdowns).
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Click,
    /// The pointer enters the layer. An overlay opened this way closes again when the pointer
    /// has left both the layer and the overlay.
    Hover,
    /// This long after the screen holding the layer opens.
    Delay { ms: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtoAction {
    Navigate {
        target: Target,
        #[serde(default)]
        transition: Transition,
    },
    Back,
    OpenOverlay {
        target: Target,
        #[serde(default)]
        placement: OverlayPlacement,
        /// Darken what's behind the overlay.
        #[serde(default = "yes")]
        dim: bool,
        /// A click outside closes it.
        #[serde(default = "yes")]
        close_outside: bool,
        #[serde(default)]
        transition: Transition,
    },
    CloseOverlay,
    OpenUrl {
        url: String,
    },
}

impl ProtoAction {
    pub fn target(&self) -> Option<&Target> {
        match self {
            ProtoAction::Navigate { target, .. } | ProtoAction::OpenOverlay { target, .. } => {
                Some(target)
            }
            _ => None,
        }
    }
    pub fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            ProtoAction::Navigate { target, .. } | ProtoAction::OpenOverlay { target, .. } => {
                Some(target)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    #[serde(default)]
    pub trigger: Trigger,
    pub action: ProtoAction,
}

/// Style overrides while a layer is hovered or pressed, the way `:hover` and `:active` apply to a
/// box whenever the pointer is over it or anything inside it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateStyle {
    /// Background of a frame or shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    /// Color of this layer's text and of every text layer inside it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Moves the layer and its contents, in px.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<V2>,
}

impl StateStyle {
    pub fn is_empty(&self) -> bool {
        *self == StateStyle::default()
    }

    /// Apply to the layer itself. Text colors of the layers inside it are [`Self::apply_text`].
    pub fn apply(&self, n: &mut Node) {
        let is_text = matches!(n.kind, NodeKind::Text { .. });
        if let Some(c) = self.fill.filter(|_| !is_text) {
            n.style.set_fill(Some(c));
        }
        self.apply_text(n);
        if let Some(c) = self.stroke {
            n.style.stroke = Some(c);
            if let Some(sides) = &mut n.style.borders {
                for s in sides.iter_mut().filter(|s| s.width > 0.0) {
                    s.color = Some(c);
                }
            }
        }
        if let Some(o) = self.opacity {
            n.style.opacity = o.clamp(0.0, 1.0);
        }
        if let Some(d) = self.offset {
            n.rect = n.rect.translate(d);
        }
    }

    pub fn apply_text(&self, n: &mut Node) {
        if let Some(c) = self.text
            && matches!(n.kind, NodeKind::Text { .. })
        {
            n.style.set_fill(Some(c));
        }
    }
}

/// How a layer moves when the screen scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollPosition {
    #[default]
    Scrolls,
    /// Stays where it is, like `position: fixed`. Paints over scrolling content.
    Fixed,
    /// Scrolls until it reaches the top, then stays, like `position: sticky; top: 0`.
    Sticky,
}

/// Which way a frame scrolls content that overflows it. Top-level frames always scroll; this is
/// for frames inside them, such as carousels and scrolling panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Overflow {
    #[default]
    None,
    Vertical,
    Horizontal,
    Both,
}

impl Overflow {
    pub fn x(self) -> bool {
        matches!(self, Overflow::Horizontal | Overflow::Both)
    }
    pub fn y(self) -> bool {
        matches!(self, Overflow::Vertical | Overflow::Both)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prototype {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub interactions: Vec<Interaction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<StateStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<StateStyle>,
    #[serde(skip_serializing_if = "is_default")]
    pub position: ScrollPosition,
    #[serde(skip_serializing_if = "is_default")]
    pub overflow: Overflow,
}

impl Prototype {
    pub fn is_empty(&self) -> bool {
        *self == Prototype::default()
    }

    pub fn has_click(&self) -> bool {
        self.interactions
            .iter()
            .any(|i| i.trigger == Trigger::Click)
    }

    /// Point links inside a copied set of layers at the copies. Without `same_page`, a same-page
    /// link leaving the set has nothing to point at, so it goes.
    pub fn remap(&mut self, map: &HashMap<NodeId, NodeId>, same_page: bool) {
        self.interactions.retain_mut(|i| {
            let Some(t) = i.action.target_mut().filter(|t| t.page.is_none()) else {
                return true;
            };
            match map.get(&t.node) {
                Some(n) => {
                    t.node = *n;
                    true
                }
                None => same_page,
            }
        });
    }

    /// Before layers leave their page: same-page links to layers outside the copied set name the
    /// page, so they keep working wherever the copy lands.
    pub fn name_page(&mut self, page: &str, copied: impl Fn(NodeId) -> bool) {
        for i in &mut self.interactions {
            if let Some(t) = i.action.target_mut()
                && t.page.is_none()
                && !copied(t.node)
            {
                t.page = Some(page.to_string());
            }
        }
    }
}

impl Node {
    /// Edit the prototype settings, dropping them once they're all defaults.
    pub fn edit_proto(&mut self, f: impl FnOnce(&mut Prototype)) {
        let mut p = self.proto.take().map(|b| *b).unwrap_or_default();
        f(&mut p);
        self.proto = (!p.is_empty()).then(|| Box::new(p));
    }

    /// Like [`Node::edit_proto`], for layers that have prototype settings at all.
    pub fn edit_proto_if_set(&mut self, f: impl FnOnce(&mut Prototype)) {
        if self.proto.is_some() {
            self.edit_proto(f);
        }
    }

    pub fn interactions(&self) -> &[Interaction] {
        self.proto.as_ref().map_or(&[], |p| &p.interactions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;

    fn link(node: u64) -> Interaction {
        Interaction {
            trigger: Trigger::Click,
            action: ProtoAction::Navigate {
                target: Target::same_page(NodeId(node)),
                transition: Transition::default(),
            },
        }
    }

    fn target_of(p: &Prototype) -> &Target {
        p.interactions[0].action.target().unwrap()
    }

    #[test]
    fn nodes_without_prototypes_serialize_as_before() {
        let n = Node::new(NodeId(1), NodeKind::Rect, Rect::new(0.0, 0.0, 1.0, 1.0));
        let s = serde_json::to_string(&n).unwrap();
        assert!(!s.contains("proto"), "{s}");
        let back: Node = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn prototype_roundtrips_through_json() {
        let mut n = Node::new(NodeId(1), NodeKind::Rect, Rect::new(0.0, 0.0, 1.0, 1.0));
        n.edit_proto(|p| {
            p.interactions = vec![
                link(7),
                Interaction {
                    trigger: Trigger::Delay { ms: 2000.0 },
                    action: ProtoAction::OpenOverlay {
                        target: Target {
                            page: Some("abc".into()),
                            node: NodeId(9),
                        },
                        placement: OverlayPlacement::Below,
                        dim: false,
                        close_outside: false,
                        transition: Transition {
                            kind: TransitionKind::Push,
                            dir: SlideDir::Up,
                            ms: 150.0,
                            easing: Easing::Linear,
                        },
                    },
                },
                Interaction {
                    trigger: Trigger::Hover,
                    action: ProtoAction::OpenUrl {
                        url: "https://example.com".into(),
                    },
                },
                Interaction {
                    trigger: Trigger::Click,
                    action: ProtoAction::Back,
                },
            ];
            p.hover = Some(StateStyle {
                fill: Some(Color::rgb(1, 2, 3)),
                offset: Some(crate::geom::v2(0.0, -2.0)),
                ..StateStyle::default()
            });
            p.position = ScrollPosition::Sticky;
            p.overflow = Overflow::Horizontal;
        });
        let s = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn clearing_the_last_setting_removes_the_field() {
        let mut n = Node::new(NodeId(1), NodeKind::Rect, Rect::new(0.0, 0.0, 1.0, 1.0));
        n.edit_proto(|p| p.interactions.push(link(2)));
        assert!(n.proto.is_some());
        n.edit_proto(|p| p.interactions.clear());
        assert!(n.proto.is_none());
    }

    #[test]
    fn remap_follows_copies_and_drops_links_it_cannot_keep() {
        let map: HashMap<_, _> = [(NodeId(1), NodeId(10))].into_iter().collect();
        let mut inside = Prototype {
            interactions: vec![link(1)],
            ..Prototype::default()
        };
        inside.remap(&map, false);
        assert_eq!(target_of(&inside).node, NodeId(10));

        let mut outside = Prototype {
            interactions: vec![link(2)],
            ..Prototype::default()
        };
        outside.remap(&map, true);
        assert_eq!(target_of(&outside).node, NodeId(2), "same page keeps it");
        outside.remap(&map, false);
        assert!(
            outside.interactions.is_empty(),
            "elsewhere it means nothing"
        );
    }

    #[test]
    fn remap_leaves_links_to_named_pages_alone() {
        let map: HashMap<_, _> = [(NodeId(1), NodeId(10))].into_iter().collect();
        let mut p = Prototype {
            interactions: vec![link(1)],
            ..Prototype::default()
        };
        p.interactions[0].action.target_mut().unwrap().page = Some("other".into());
        p.remap(&map, false);
        assert_eq!(target_of(&p).node, NodeId(1));
    }

    #[test]
    fn name_page_only_touches_links_leaving_the_copied_set() {
        let mut p = Prototype {
            interactions: vec![link(1), link(2)],
            ..Prototype::default()
        };
        p.name_page("home", |id| id == NodeId(1));
        assert_eq!(p.interactions[0].action.target().unwrap().page, None);
        assert_eq!(
            p.interactions[1].action.target().unwrap().page.as_deref(),
            Some("home")
        );
    }

    #[test]
    fn hover_style_recolors_shapes_and_text_separately() {
        let style = StateStyle {
            fill: Some(Color::rgb(1, 1, 1)),
            text: Some(Color::rgb(2, 2, 2)),
            ..StateStyle::default()
        };
        let mut shape = Node::new(NodeId(1), NodeKind::Rect, Rect::new(0.0, 0.0, 1.0, 1.0));
        style.apply(&mut shape);
        assert_eq!(shape.style.fill(), Some(Color::rgb(1, 1, 1)));

        let mut text = crate::demo::text_node("a", 12.0, crate::geom::V2::ZERO, Color::BLACK);
        style.apply(&mut text);
        assert_eq!(text.style.fill(), Some(Color::rgb(2, 2, 2)));
    }

    #[test]
    fn easing_stays_within_its_ends() {
        for e in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            assert_eq!(e.apply(0.0), 0.0);
            assert_eq!(e.apply(1.0), 1.0);
            assert_eq!(e.apply(2.0), 1.0);
        }
    }
}
