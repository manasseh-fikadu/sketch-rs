//! Scene nodes.

use super::paint::{
    BlendMode, Cap, InstanceLink, Join, LayoutChild, Paint, StrokeAlign, TextStyle,
};
use crate::geom::{Rect, V2};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// sRGB color with straight alpha. Serialized as `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const INK: Color = Color::rgb(28, 28, 30);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#')?;
        let p = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
        match s.len() {
            6 => Some(Color::rgb(p(0)?, p(2)?, p(4)?)),
            8 => Some(Color::rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            _ => None,
        }
    }
    /// Normalized RGBA with the alpha multiplied by `opacity`.
    pub fn to_f32(self, opacity: f64) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            (self.a as f64 / 255.0 * opacity) as f32,
        ]
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color {s:?}")))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// Fill layers, bottom to top.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<Paint>,
    pub stroke: Option<Color>,
    pub stroke_width: f64,
    #[serde(skip_serializing_if = "is_default")]
    pub stroke_align: StrokeAlign,
    /// Dash pattern (lengths in px); empty = solid.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f64>,
    #[serde(skip_serializing_if = "is_default")]
    pub cap: Cap,
    #[serde(skip_serializing_if = "is_default")]
    pub join: Join,
    pub corner_radius: f64,
    /// Per-corner radii (top-left, top-right, bottom-right, bottom-left); overrides
    /// `corner_radius` when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radii: Option<[f64; 4]>,
    pub opacity: f64,
    #[serde(skip_serializing_if = "is_default")]
    pub blend: BlendMode,
    /// Hand-drawn (rough.js style) rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rough: Option<super::rough::Rough>,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fills: Vec::new(),
            stroke: None,
            stroke_width: 1.0,
            stroke_align: StrokeAlign::Center,
            dash: Vec::new(),
            cap: Cap::Butt,
            join: Join::Miter,
            corner_radius: 0.0,
            radii: None,
            opacity: 1.0,
            blend: BlendMode::Normal,
            rough: None,
        }
    }
}

impl Style {
    pub fn with_fill(c: Color) -> Style {
        Style {
            fills: vec![Paint::solid(c)],
            ..Style::default()
        }
    }
    /// The top solid fill color, if the top fill is solid.
    pub fn fill(&self) -> Option<Color> {
        match self.fills.last() {
            Some(Paint::Solid { color }) => Some(*color),
            _ => None,
        }
    }
    /// Replace all fills with one solid color (or none).
    pub fn set_fill(&mut self, c: Option<Color>) {
        self.fills = c.map(Paint::solid).into_iter().collect();
    }
    /// Corner radii (tl, tr, br, bl).
    pub fn corner_radii(&self) -> [f64; 4] {
        self.radii.unwrap_or([self.corner_radius; 4])
    }
}

/// One sampled input point of a freehand stroke, in node-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    /// 0..=1. Real pen pressure when available, otherwise simulated from velocity.
    pub p: f64,
}

impl StrokePoint {
    pub fn pos(&self) -> V2 {
        V2 {
            x: self.x,
            y: self.y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NodeKind {
    /// Artboard-like container; children are positioned relative to it.
    Frame {
        #[serde(default = "yes")]
        clip: bool,
    },
    Rect,
    Ellipse,
    /// Straight line between two node-local points.
    Line {
        a: V2,
        b: V2,
    },
    Text {
        text: String,
        font_size: f64,
        /// When true the box grows to fit the text; otherwise it wraps at `rect.w`.
        #[serde(default = "yes")]
        auto_width: bool,
        #[serde(default)]
        align: TextAlign,
    },
    /// Groups children; its rect is always the union of the children's rects.
    Group,
    /// Vector path (pen tool, boolean results, imported SVG). Points are node-local.
    Path {
        subpaths: Vec<SubPath>,
        #[serde(default)]
        fill_rule: FillRule,
    },
    /// Regular polygon inscribed in the box, first vertex at the top.
    Polygon {
        sides: u32,
    },
    /// Star inscribed in the box; `inner` is the inner radius ratio.
    Star {
        points: u32,
        inner: f64,
    },
    /// Arrow or connector line; ends may bind to other layers and follow them.
    Connector {
        /// Node-local points (at least two).
        points: Vec<V2>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<Binding>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<Binding>,
        #[serde(default)]
        route: Route,
        #[serde(default)]
        start_head: Head,
        #[serde(default = "arrow_head")]
        end_head: Head,
    },
    /// Sticky note: a colored square whose text wraps inside it.
    Sticky {
        text: String,
        font_size: f64,
    },
    /// Freehand ink. Points are node-local; the outline is computed on render.
    Stroke {
        points: Vec<StrokePoint>,
        /// Nominal diameter at pressure 0.5.
        width: f64,
        /// How much pressure modulates width (0 = uniform width).
        #[serde(default = "default_thinning")]
        thinning: f64,
    },
}

fn yes() -> bool {
    true
}
fn arrow_head() -> Head {
    Head::Arrow
}

/// How a connector's line is drawn between its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    #[default]
    Straight,
    /// Smooth curve through the points.
    Curved,
    /// Orthogonal segments routed around the bound shapes.
    Elbow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Head {
    #[default]
    None,
    Arrow,
    Triangle,
    Dot,
    Bar,
    Diamond,
}

impl Head {
    pub const ALL: [Head; 6] = [
        Head::None,
        Head::Arrow,
        Head::Triangle,
        Head::Dot,
        Head::Bar,
        Head::Diamond,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Head::None => "None",
            Head::Arrow => "Arrow",
            Head::Triangle => "Triangle",
            Head::Dot => "Dot",
            Head::Bar => "Bar",
            Head::Diamond => "Diamond",
        }
    }
}

/// A connector end attached to another layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub node: NodeId,
    /// Attachment point as a fraction of the target's box (0..1 inside it).
    pub fixed: V2,
    /// Inside: the end sits exactly on the fixed point. Otherwise it stops at the outline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inside: bool,
}
fn default_thinning() -> f64 {
    0.6
}

impl NodeKind {
    /// Editable text content (text layers and sticky notes).
    pub fn text(&self) -> Option<&String> {
        match self {
            NodeKind::Text { text, .. } | NodeKind::Sticky { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            NodeKind::Text { text, .. } | NodeKind::Sticky { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn font_size(&self) -> Option<f64> {
        match self {
            NodeKind::Text { font_size, .. } | NodeKind::Sticky { font_size, .. } => {
                Some(*font_size)
            }
            _ => None,
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::Frame { .. } => "Frame",
            NodeKind::Rect => "Rectangle",
            NodeKind::Ellipse => "Ellipse",
            NodeKind::Line { .. } => "Line",
            NodeKind::Text { .. } => "Text",
            NodeKind::Stroke { .. } => "Drawing",
            NodeKind::Group => "Group",
            NodeKind::Path { .. } => "Vector",
            NodeKind::Polygon { .. } => "Polygon",
            NodeKind::Star { .. } => "Star",
            NodeKind::Connector { .. } => "Arrow",
            NodeKind::Sticky { .. } => "Sticky",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Bounds relative to the parent frame's origin (or the world for root nodes).
    pub rect: Rect,
    #[serde(default)]
    pub style: Style,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeId>,
    /// Paint order, bottom to top.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeId>,
    /// Rich text attributes (text nodes).
    #[serde(default, skip_serializing_if = "TextStyle::is_default")]
    pub text_style: TextStyle,
    /// Auto layout (frames only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<super::paint::AutoLayout>,
    /// How this node sizes and positions itself inside its parent frame.
    #[serde(default, skip_serializing_if = "LayoutChild::is_default")]
    pub layout_child: LayoutChild,
    /// This frame is a main component.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub component: bool,
    /// Instance link (instance roots and every node inside an instance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<InstanceLink>,
}

/// One anchor of a vector path, with optional bezier handles (node-local, absolute).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    pub p: V2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_in: Option<V2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_out: Option<V2>,
}

impl PathPoint {
    pub fn corner(p: V2) -> PathPoint {
        PathPoint {
            p,
            h_in: None,
            h_out: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubPath {
    pub points: Vec<PathPoint>,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl Node {
    pub fn new(id: NodeId, kind: NodeKind, rect: Rect) -> Node {
        let gray = Color::rgb(0xd9, 0xd9, 0xd9);
        let style = match &kind {
            NodeKind::Frame { .. } => Style::with_fill(Color::WHITE),
            NodeKind::Rect
            | NodeKind::Ellipse
            | NodeKind::Polygon { .. }
            | NodeKind::Star { .. }
            | NodeKind::Path { .. } => Style::with_fill(gray),
            NodeKind::Line { .. } => Style {
                stroke: Some(Color::INK),
                stroke_width: 2.0,
                ..Style::default()
            },
            NodeKind::Text { .. } | NodeKind::Stroke { .. } => Style::with_fill(Color::INK),
            NodeKind::Group => Style::default(),
            NodeKind::Connector { .. } => Style {
                stroke: Some(Color::INK),
                stroke_width: 2.0,
                ..Style::default()
            },
            NodeKind::Sticky { .. } => Style::with_fill(Color::rgb(0xff, 0xdf, 0x6b)),
        };
        Node {
            id,
            name: kind.type_name().to_string(),
            kind,
            rect,
            style,
            visible: true,
            locked: false,
            parent: None,
            children: Vec::new(),
            text_style: TextStyle::default(),
            layout: None,
            layout_child: LayoutChild::default(),
            component: false,
            instance: None,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, NodeKind::Group)
    }

    /// Frames and groups hold children.
    pub fn is_container(&self) -> bool {
        matches!(self.kind, NodeKind::Frame { .. } | NodeKind::Group)
    }

    pub fn is_frame(&self) -> bool {
        matches!(self.kind, NodeKind::Frame { .. })
    }

    /// Change the local bounds, scaling intrinsic geometry (ink, lines) to match.
    pub fn set_rect_scaling(&mut self, new: Rect) {
        let old = self.rect;
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
        match &mut self.kind {
            NodeKind::Stroke { points, .. } => {
                for p in points.iter_mut() {
                    p.x *= sx;
                    p.y *= sy;
                }
            }
            NodeKind::Line { a, b } => {
                a.x *= sx;
                a.y *= sy;
                b.x *= sx;
                b.y *= sy;
            }
            NodeKind::Connector { points, .. } => {
                for p in points.iter_mut() {
                    p.x *= sx;
                    p.y *= sy;
                }
            }
            NodeKind::Path { subpaths, .. } => {
                let sc = |v: &mut V2| {
                    v.x *= sx;
                    v.y *= sy;
                };
                for sp in subpaths.iter_mut() {
                    for pt in sp.points.iter_mut() {
                        sc(&mut pt.p);
                        if let Some(h) = &mut pt.h_in {
                            sc(h);
                        }
                        if let Some(h) = &mut pt.h_out {
                            sc(h);
                        }
                    }
                }
            }
            NodeKind::Text { auto_width, .. } if (new.w - old.w).abs() > 1e-9 => {
                *auto_width = false;
            }
            _ => {}
        }
        self.rect = new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_hex_roundtrip() {
        for c in [Color::rgb(1, 2, 255), Color::rgba(0x12, 0x34, 0x56, 0x78)] {
            assert_eq!(Color::from_hex(&c.to_hex()), Some(c));
        }
        assert_eq!(Color::from_hex("#abc"), None);
        assert_eq!(Color::from_hex("ffffff"), None);
    }

    #[test]
    fn scaling_rect_scales_ink() {
        let mut n = Node::new(
            NodeId(1),
            NodeKind::Stroke {
                points: vec![StrokePoint {
                    x: 10.0,
                    y: 20.0,
                    p: 0.5,
                }],
                width: 4.0,
                thinning: 0.5,
            },
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        n.set_rect_scaling(Rect::new(5.0, 5.0, 200.0, 50.0));
        let NodeKind::Stroke { points, .. } = &n.kind else {
            unreachable!()
        };
        assert_eq!((points[0].x, points[0].y), (20.0, 10.0));
    }

    #[test]
    fn node_json_shape() {
        let n = Node::new(
            NodeId(7),
            NodeKind::Frame { clip: true },
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let s = serde_json::to_string(&n).unwrap();
        assert!(s.contains("\"type\":\"frame\""), "{s}");
        assert!(s.contains("\"color\":\"#ffffff\""), "{s}");
        let back: Node = serde_json::from_str(&s).unwrap();
        assert_eq!(back, n);
    }
}
