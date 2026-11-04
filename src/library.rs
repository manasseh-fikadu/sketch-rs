//! A personal library of reusable shapes, shared by all projects
//! (`<projects folder>/library.json`), like Excalidraw's library.

use crate::geom::{Rect, V2, v2};
use crate::model::{Color, Document, Node, NodeId, NodeKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "sketch-rs-library";
/// The tag from before the rename from paper-rs, still read so a saved library survives it.
pub const OLD_FORMAT: &str = "paper-rs-library";
pub const VERSION: u32 = 1;
pub const FILE: &str = "library.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibItem {
    pub id: String,
    pub name: String,
    /// The item's nodes; roots are positioned relative to the item's top-left corner.
    pub nodes: Vec<Node>,
    pub roots: Vec<NodeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub format: String,
    pub version: u32,
    pub items: Vec<LibItem>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            format: FORMAT.into(),
            version: VERSION,
            items: Vec::new(),
            path: None,
        }
    }
}

impl LibItem {
    /// The item as a standalone document (for previews and insertion).
    pub fn doc(&self) -> Document {
        let mut d = Document::default();
        for n in &self.nodes {
            d.nodes.insert(n.id, n.clone());
            d.next_id = d.next_id.max(n.id.0 + 1);
        }
        d.root = self.roots.clone();
        d
    }

    pub fn bounds(&self) -> Option<Rect> {
        let d = self.doc();
        crate::io::svg::export_bounds(&d, &d.root)
    }

    /// Build an item from layers of a document (copied, re-based to the origin).
    pub fn from_selection(doc: &Document, ids: &[NodeId], name: &str) -> Option<LibItem> {
        let bounds = Rect::union_all(
            &ids.iter()
                .filter_map(|id| doc.world_rect(*id))
                .collect::<Vec<_>>(),
        )?;
        let mut nodes = Vec::new();
        for r in ids {
            for id in doc.subtree(*r) {
                let mut n = doc.get(id)?.clone();
                if id == *r {
                    n.rect = doc.world_rect(id)?.translate(V2::ZERO - bounds.min());
                    n.parent = None;
                }
                // Instances in the library are plain copies.
                n.instance = None;
                nodes.push(n);
            }
        }
        Some(LibItem {
            id: crate::project::new_id(),
            name: name.into(),
            nodes,
            roots: ids.to_vec(),
        })
    }
}

impl Library {
    pub fn path_in(root: &Path) -> PathBuf {
        root.join(FILE)
    }

    /// Load the library, seeding it with starter shapes on first use.
    pub fn load(path: &Path) -> Library {
        let mut lib = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Library>(&s).ok())
            .filter(|l| (l.format == FORMAT || l.format == OLD_FORMAT) && l.version <= VERSION)
            .unwrap_or_else(starter);
        lib.format = FORMAT.into();
        lib.path = Some(path.to_path_buf());
        lib
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = &self.path else { return Ok(()) };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::io::atomic::write(
            p,
            serde_json::to_string_pretty(self)
                .expect("library serializes")
                .as_bytes(),
        )
    }
}

fn node(kind: NodeKind, r: Rect, fill: Option<Color>, name: &str) -> Node {
    let mut n = Node::new(NodeId(0), kind, r);
    n.style.set_fill(fill);
    n.style.stroke = Some(Color::INK);
    n.style.stroke_width = 2.0;
    n.name = name.into();
    n
}

fn item(name: &str, nodes: Vec<Node>) -> LibItem {
    let mut out = Vec::new();
    let mut roots = Vec::new();
    for (i, mut n) in nodes.into_iter().enumerate() {
        n.id = NodeId(i as u64 + 1);
        roots.push(n.id);
        out.push(n);
    }
    LibItem {
        id: format!("starter-{}", name.to_lowercase().replace(' ', "-")),
        name: name.into(),
        nodes: out,
        roots,
    }
}

/// Flowchart basics so the library isn't empty on first launch.
pub fn starter() -> Library {
    let blue = Some(Color::rgb(0xa5, 0xd8, 0xff));
    let yellow = Some(Color::rgb(0xff, 0xec, 0x99));
    let green = Some(Color::rgb(0xb2, 0xf2, 0xbb));
    let pink = Some(Color::rgb(0xff, 0xc9, 0xc9));
    let mut process = node(
        NodeKind::Rect,
        Rect::new(0.0, 0.0, 160.0, 80.0),
        blue,
        "Process",
    );
    process.style.corner_radius = 12.0;
    let decision = node(
        NodeKind::Polygon { sides: 4 },
        Rect::new(0.0, 0.0, 140.0, 100.0),
        yellow,
        "Decision",
    );
    let mut terminal = node(
        NodeKind::Rect,
        Rect::new(0.0, 0.0, 160.0, 60.0),
        green,
        "Start / End",
    );
    terminal.style.corner_radius = 30.0;
    // Database: a cylinder as a vector path.
    let mut db_path = crate::vector::VPath::new();
    let (w, h, e) = (120.0, 110.0, 16.0);
    let k = crate::vector::KAPPA;
    db_path
        .move_to(v2(0.0, e))
        .cubic_to(
            v2(0.0, e - e * k),
            v2(w * 0.5 - w * 0.5 * k, 0.0),
            v2(w * 0.5, 0.0),
        )
        .cubic_to(v2(w * 0.5 + w * 0.5 * k, 0.0), v2(w, e - e * k), v2(w, e))
        .line_to(v2(w, h - e))
        .cubic_to(
            v2(w, h - e + e * k),
            v2(w * 0.5 + w * 0.5 * k, h),
            v2(w * 0.5, h),
        )
        .cubic_to(
            v2(w * 0.5 - w * 0.5 * k, h),
            v2(0.0, h - e + e * k),
            v2(0.0, h - e),
        )
        .close();
    let db = node(
        NodeKind::Path {
            subpaths: crate::vector::to_subpaths(&db_path, V2::ZERO),
            fill_rule: crate::model::FillRule::NonZero,
        },
        Rect::new(0.0, 0.0, w, h),
        pink,
        "Database",
    );
    let mut lid = crate::vector::VPath::new();
    lid.move_to(v2(0.0, e))
        .cubic_to(
            v2(0.0, e + e * k),
            v2(w * 0.5 - w * 0.5 * k, 2.0 * e),
            v2(w * 0.5, 2.0 * e),
        )
        .cubic_to(
            v2(w * 0.5 + w * 0.5 * k, 2.0 * e),
            v2(w, e + e * k),
            v2(w, e),
        );
    let mut lid_node = node(
        NodeKind::Path {
            subpaths: crate::vector::to_subpaths(&lid, V2::ZERO),
            fill_rule: crate::model::FillRule::NonZero,
        },
        Rect::new(0.0, e, w, e),
        None,
        "Lid",
    );
    lid_node.style.fills.clear();
    let mut note = Node::new(
        NodeId(0),
        NodeKind::Sticky {
            text: "Idea".into(),
            font_size: 24.0,
        },
        Rect::new(0.0, 0.0, 160.0, 160.0),
    );
    note.name = "Sticky".into();
    let mut persona = node(
        NodeKind::Ellipse,
        Rect::new(0.0, 0.0, 90.0, 90.0),
        Some(Color::rgb(0xd0, 0xbf, 0xff)),
        "Persona",
    );
    persona.style.stroke_width = 2.0;
    let mut items = vec![
        item("Process", vec![process]),
        item("Decision", vec![decision]),
        item("Start / End", vec![terminal]),
        item("Sticky", vec![note]),
        item("Persona", vec![persona]),
    ];
    // Multi-node item: the cylinder and its lid line, grouped by being one item.
    items.push(item("Database", vec![db, lid_node]));
    Library {
        items,
        ..Library::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip_and_starter() {
        let dir = std::env::temp_dir().join(format!("sketch-rs-lib-{:016x}", fastrand::u64(..)));
        let path = dir.join(FILE);
        let lib = Library::load(&path);
        assert!(lib.items.len() >= 5, "starter shapes on first use");
        lib.save().unwrap();
        let back = Library::load(&path);
        assert_eq!(back.items, lib.items);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn library_from_before_the_rename_loads() {
        let dir = std::env::temp_dir().join(format!("sketch-rs-lib-{:016x}", fastrand::u64(..)));
        let path = dir.join(FILE);
        let mut lib = Library::load(&path);
        lib.items.truncate(1);
        lib.save().unwrap();
        let old = std::fs::read_to_string(&path)
            .unwrap()
            .replace(FORMAT, OLD_FORMAT);
        std::fs::write(&path, old).unwrap();
        let back = Library::load(&path);
        assert_eq!(back.items, lib.items);
        assert_eq!(back.format, FORMAT);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn items_from_selection_rebase_to_origin() {
        let mut ed = crate::editor::Editor::default();
        let a = ed.add_node(
            Node::new(
                NodeId(0),
                NodeKind::Rect,
                Rect::new(100.0, 50.0, 20.0, 20.0),
            ),
            None,
        );
        let b = ed.add_node(
            Node::new(
                NodeId(0),
                NodeKind::Ellipse,
                Rect::new(140.0, 60.0, 20.0, 20.0),
            ),
            None,
        );
        let it = LibItem::from_selection(&ed.doc, &[a, b], "Pair").unwrap();
        assert_eq!(it.bounds().unwrap().min(), V2::ZERO);
        assert_eq!(it.doc().root.len(), 2);
    }
}
