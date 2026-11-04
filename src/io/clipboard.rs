//! System clipboard interop.
//!
//! Copying writes a `sketch-rs/clipboard` JSON payload (layers, their world rects and the
//! images they use) as text, so layers paste between windows, projects and app restarts.
//! Pasting accepts that payload, Excalidraw clipboard/scene JSON, SVG markup, images and
//! plain text (which becomes a text layer).

use crate::geom::{Rect, V2};
use crate::model::{Node, NodeId, Paint};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const TYPE: &str = "sketch-rs/clipboard";
/// Copied by builds from before the rename from paper-rs; still pastes.
pub const OLD_TYPE: &str = "paper-rs/clipboard";

/// Platform clipboard access, injected into the editor (tests use an in-memory one).
pub trait Provider {
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, s: &str);
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        None
    }
    fn write_image(&mut self, _img: &image::RgbaImage) {}
}

/// In-memory clipboard (tests, headless).
#[derive(Default)]
pub struct Memory {
    pub text: Option<String>,
    pub image: Option<image::RgbaImage>,
}

impl Provider for Memory {
    fn read_text(&mut self) -> Option<String> {
        self.text.clone()
    }
    fn write_text(&mut self, s: &str) {
        self.text = Some(s.into());
        self.image = None;
    }
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        self.image.clone()
    }
    fn write_image(&mut self, img: &image::RgbaImage) {
        self.image = Some(img.clone());
    }
}

/// The OS clipboard via arboard (None when no clipboard is available, e.g. headless).
pub struct System(arboard::Clipboard);

impl System {
    pub fn new() -> Option<System> {
        arboard::Clipboard::new().ok().map(System)
    }
}

impl Provider for System {
    fn read_text(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }
    fn write_text(&mut self, s: &str) {
        let _ = self.0.set_text(s.to_string());
    }
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        let img = self.0.get_image().ok()?;
        image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
    }
    fn write_image(&mut self, img: &image::RgbaImage) {
        let _ = self.0.set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: std::borrow::Cow::Borrowed(img.as_raw()),
        });
    }
}

#[derive(Serialize, Deserialize)]
pub struct Payload {
    #[serde(rename = "type")]
    pub kind: String,
    pub nodes: Vec<Node>,
    pub roots: Vec<NodeId>,
    pub world: Vec<Rect>,
    /// Page the layers came from (same page: offset paste).
    #[serde(default)]
    pub page: Option<String>,
    /// Images used by the layers: asset id → base64 bytes.
    #[serde(default)]
    pub assets: HashMap<String, String>,
}

pub fn encode(nodes: &[Node], roots: &[NodeId], world: &[Rect], page: Option<&str>) -> String {
    let mut assets = HashMap::new();
    for n in nodes {
        for p in &n.style.fills {
            if let Paint::Image { asset, .. } = p
                && let Some(a) = crate::assets::get(asset)
            {
                assets.insert(asset.clone(), crate::assets::base64(&a.bytes));
            }
        }
    }
    serde_json::to_string(&Payload {
        kind: TYPE.into(),
        nodes: nodes.to_vec(),
        roots: roots.to_vec(),
        world: world.to_vec(),
        page: page.map(String::from),
        assets,
    })
    .expect("clipboard payload serializes")
}

/// What a clipboard text holds.
pub enum Content {
    Layers(Payload),
    /// Layers from another app (Excalidraw JSON or SVG), in their own coordinates.
    Foreign(Vec<Node>, &'static str),
    Text(String),
}

/// Interpret clipboard text.
pub fn decode(text: &str) -> Content {
    let t = text.trim_start();
    if t.starts_with('{') {
        if let Ok(p) = serde_json::from_str::<Payload>(t)
            && (p.kind == TYPE || p.kind == OLD_TYPE)
        {
            for (id, b64) in &p.assets {
                if crate::assets::get(id).is_none()
                    && let Some(bytes) = crate::assets::base64_decode(b64)
                {
                    let _ = crate::assets::register(&bytes);
                }
            }
            return Content::Layers(p);
        }
        if let Ok(doc) = crate::io::excalidraw::import_scene(t) {
            // Flatten the imported page into root subtrees.
            let nodes: Vec<Node> = doc
                .paint_order()
                .into_iter()
                .filter_map(|id| doc.get(id).cloned())
                .collect();
            if !nodes.is_empty() {
                return Content::Foreign(nodes, "Excalidraw");
            }
        }
    }
    if (t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg")))
        && let Ok(nodes) = crate::io::svg_import::parse(t)
    {
        return Content::Foreign(nodes, "SVG");
    }
    Content::Text(text.to_string())
}

/// Root ids and world rects of a node list whose roots have no parent in the list.
pub fn roots_of(nodes: &[Node]) -> (Vec<NodeId>, Vec<Rect>) {
    let ids: std::collections::HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
    let mut roots = Vec::new();
    let mut world = Vec::new();
    for n in nodes {
        if n.parent.is_none_or(|p| !ids.contains(&p)) {
            roots.push(n.id);
            world.push(n.rect);
        }
    }
    (roots, world)
}

/// Bounds of root rects.
pub fn bounds(world: &[Rect]) -> Option<Rect> {
    Rect::union_all(world)
}

pub fn center_offset(world: &[Rect], at: V2) -> V2 {
    bounds(world).map_or(V2::ZERO, |b| (at - b.center()).round())
}
