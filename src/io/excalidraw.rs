//! Import of Excalidraw scenes (`.excalidraw`, clipboard JSON) and libraries
//! (`.excalidrawlib`).
//!
//! Field meanings, defaults and conversions follow Excalidraw's `restore.ts` and element
//! types (Copyright (c) 2020 Excalidraw, MIT; see THIRD_PARTY.md and
//! `analysis/EXCALIDRAW_NOTES.md` §1, §6, §8). Rotated shapes are baked into vector paths
//! (Sketch has no rotation); groups become group layers; frames clip their children.

use crate::geom::{Rect, V2, v2};
use crate::history::{Recording, Tx};
use crate::library::LibItem;
use crate::model::rough::{FillStyle, Rough};
use crate::model::{
    Binding, Color, Document, FillRule, Head, ImageMode, Node, NodeId, NodeKind, Paint, Route,
    StrokePoint, TextAlign,
};
use serde_json::Value;
use std::collections::HashMap;

/// CSS color → Color (hex forms, `transparent`, and a few names Excalidraw uses).
pub fn css_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("transparent") || s.is_empty() {
        return None;
    }
    if let Some(h) = s.strip_prefix('#') {
        let p = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
        return match h.len() {
            3 => Some(Color::rgb(p(0, 1)? * 17, p(1, 1)? * 17, p(2, 1)? * 17)),
            4 => Some(Color::rgba(
                p(0, 1)? * 17,
                p(1, 1)? * 17,
                p(2, 1)? * 17,
                p(3, 1)? * 17,
            )),
            6 => Some(Color::rgb(p(0, 2)?, p(2, 2)?, p(4, 2)?)),
            8 => Some(Color::rgba(p(0, 2)?, p(2, 2)?, p(4, 2)?, p(6, 2)?)),
            _ => None,
        };
    }
    Some(match s.to_ascii_lowercase().as_str() {
        "black" => Color::BLACK,
        "white" => Color::WHITE,
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        _ => return None,
    })
}

fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(d)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str())
}

fn head(v: Option<&str>) -> Head {
    match v.unwrap_or("") {
        "arrow" => Head::Arrow,
        "triangle" | "triangle_outline" => Head::Triangle,
        "circle" | "circle_outline" | "dot" => Head::Dot,
        "bar" => Head::Bar,
        "diamond" | "diamond_outline" => Head::Diamond,
        x if x.starts_with("cardinality") || x.starts_with("crowfoot") => Head::Arrow,
        _ => Head::None,
    }
}

fn rotate(p: V2, c: V2, a: f64) -> V2 {
    let (sn, cs) = a.sin_cos();
    let d = p - c;
    c + v2(d.x * cs - d.y * sn, d.x * sn + d.y * cs)
}

/// Excalidraw's corner radius for a `roundness` setting.
fn corner_radius(el: &Value, w: f64, h: f64) -> f64 {
    let Some(r) = el.get("roundness").filter(|r| !r.is_null()) else {
        return 0.0;
    };
    let x = w.min(h);
    match r.get("type").and_then(|t| t.as_u64()) {
        Some(3) => {
            let fixed = r.get("value").and_then(|v| v.as_f64()).unwrap_or(32.0);
            if x <= fixed / 0.25 { x * 0.25 } else { fixed }
        }
        _ => x * 0.25,
    }
}

struct Imported {
    nodes: Vec<(String, Node)>,
    /// Excalidraw element id → (groupIds innermost first, frameId).
    nesting: HashMap<String, (Vec<String>, Option<String>)>,
    /// Arrow bindings to resolve once all ids are known: (arrow id, end, target, fixed, inside).
    bindings: Vec<(String, usize, String, Option<V2>, bool)>,
}

/// Convert elements into world-space nodes.
fn convert(elements: &[Value], files: Option<&Value>) -> Imported {
    let mut out = Imported {
        nodes: Vec::new(),
        nesting: HashMap::new(),
        bindings: Vec::new(),
    };
    // Bound text: container id → text element, merged into sticky notes.
    let mut sticky_text: HashMap<String, &Value> = HashMap::new();
    for el in elements {
        if s(el, "type") == Some("text")
            && let Some(c) = s(el, "containerId")
            && elements
                .iter()
                .any(|x| s(x, "id") == Some(c) && s(x, "type") == Some("stickynote"))
        {
            sticky_text.insert(c.to_string(), el);
        }
    }
    for el in elements {
        if el.get("isDeleted").and_then(|d| d.as_bool()) == Some(true) {
            continue;
        }
        let ty = s(el, "type").unwrap_or("");
        let id = s(el, "id").unwrap_or("").to_string();
        if ty == "text" && s(el, "containerId").is_some_and(|c| sticky_text.contains_key(c)) {
            continue;
        }
        let (x, y) = (f(el, "x", 0.0), f(el, "y", 0.0));
        let (mut w, mut h) = (f(el, "width", 0.0), f(el, "height", 0.0));
        let (mut x, mut y) = (x, y);
        if w < 0.0 {
            x += w;
            w = -w;
        }
        if h < 0.0 {
            y += h;
            h = -h;
        }
        let angle = f(el, "angle", 0.0);
        let stroke = css_color(s(el, "strokeColor").unwrap_or("#1e1e1e"));
        let bg = css_color(s(el, "backgroundColor").unwrap_or("transparent"));
        let sw = f(el, "strokeWidth", 2.0).max(0.5);
        let opacity = f(el, "opacity", 100.0) / 100.0;
        let roughness = f(el, "roughness", 1.0);
        let seed = el.get("seed").and_then(|x| x.as_i64()).unwrap_or(1) as u32;
        let fill_style = match s(el, "fillStyle").unwrap_or("solid") {
            "hachure" => FillStyle::Hachure,
            "cross-hatch" => FillStyle::CrossHatch,
            "zigzag" => FillStyle::Zigzag,
            "dots" => FillStyle::Dots,
            _ => FillStyle::Solid,
        };
        let dash = match s(el, "strokeStyle").unwrap_or("solid") {
            "dashed" => vec![8.0, 8.0 + sw],
            "dotted" => vec![1.5, 6.0 + sw],
            _ => vec![],
        };
        let rough = Some(Rough {
            roughness,
            seed: seed.max(1),
            fill_style,
            ..Rough::default()
        });
        let r = Rect::new(x, y, w, h);
        let center = r.center();
        let style_shape = |n: &mut Node| {
            n.style.set_fill(bg);
            n.style.stroke = stroke;
            n.style.stroke_width = sw;
            n.style.opacity = opacity;
            n.style.dash = dash.clone();
            n.style.rough = rough;
        };
        let groups: Vec<String> = el
            .get("groupIds")
            .and_then(|g| g.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let frame_id = s(el, "frameId").map(String::from);
        let node = match ty {
            "rectangle" | "diamond" | "ellipse" | "embeddable" | "iframe" => {
                let kind = match ty {
                    "ellipse" => NodeKind::Ellipse,
                    "diamond" => NodeKind::Polygon { sides: 4 },
                    _ => NodeKind::Rect,
                };
                let mut n = Node::new(NodeId(0), kind, r);
                style_shape(&mut n);
                if ty != "ellipse" {
                    n.style.corner_radius = corner_radius(el, w, h);
                }
                if angle.abs() > 1e-9 {
                    // Bake the rotation into a vector path.
                    let path = crate::render::draw::shape_path(&n, r)
                        .unwrap_or_default()
                        .map(|p| rotate(p, center, angle));
                    let b = path.flatten(0.5).iter().flat_map(|p| p.pts.clone()).fold(
                        None::<Rect>,
                        |acc, p| {
                            let pr = Rect::new(p.x, p.y, 0.0, 0.0);
                            Some(acc.map_or(pr, |a| a.union(&pr)))
                        },
                    );
                    let b = b.unwrap_or(r);
                    n.kind = NodeKind::Path {
                        subpaths: crate::vector::to_subpaths(&path, b.min()),
                        fill_rule: FillRule::NonZero,
                    };
                    n.rect = b;
                }
                n
            }
            "stickynote" => {
                let text = sticky_text
                    .get(&id)
                    .and_then(|t| s(t, "originalText").or(s(t, "text")))
                    .unwrap_or("")
                    .to_string();
                let size = sticky_text
                    .get(&id)
                    .map_or(28.0, |t| f(t, "fontSize", 28.0));
                let mut n = Node::new(
                    NodeId(0),
                    NodeKind::Sticky {
                        text,
                        font_size: size,
                    },
                    r,
                );
                n.style.set_fill(bg.or(Some(Color::rgb(0xff, 0xdf, 0x6b))));
                n.style.opacity = opacity;
                n
            }
            "text" => {
                let text = s(el, "text").unwrap_or("").to_string();
                let size = f(el, "fontSize", 20.0);
                let align = match s(el, "textAlign").unwrap_or("left") {
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => TextAlign::Left,
                };
                let mut n = Node::new(
                    NodeId(0),
                    NodeKind::Text {
                        text,
                        font_size: size,
                        auto_width: false,
                        align,
                    },
                    r,
                );
                n.style.set_fill(stroke.or(Some(Color::INK)));
                n.style.opacity = opacity;
                n.text_style.line_height = Some(f(el, "lineHeight", 1.25));
                n
            }
            "line" | "arrow" => {
                let pts: Vec<V2> = el
                    .get("points")
                    .and_then(|p| p.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| {
                                let p = p.as_array()?;
                                Some(v2(x + p.first()?.as_f64()?, y + p.get(1)?.as_f64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if pts.len() < 2 {
                    continue;
                }
                let pts: Vec<V2> = if angle.abs() > 1e-9 {
                    let b = pts
                        .iter()
                        .fold(Rect::new(pts[0].x, pts[0].y, 0.0, 0.0), |a, p| {
                            a.union(&Rect::new(p.x, p.y, 0.0, 0.0))
                        });
                    pts.iter().map(|p| rotate(*p, b.center(), angle)).collect()
                } else {
                    pts
                };
                let closed =
                    el.get("polygon").and_then(|p| p.as_bool()).unwrap_or(false) && ty == "line";
                let b = pts
                    .iter()
                    .fold(Rect::new(pts[0].x, pts[0].y, 0.0, 0.0), |a, p| {
                        a.union(&Rect::new(p.x, p.y, 0.0, 0.0))
                    });
                if closed {
                    let mut n = Node::new(
                        NodeId(0),
                        NodeKind::Path {
                            subpaths: vec![crate::model::SubPath {
                                points: pts
                                    .iter()
                                    .map(|p| crate::model::PathPoint::corner(*p - b.min()))
                                    .collect(),
                                closed: true,
                            }],
                            fill_rule: FillRule::NonZero,
                        },
                        b,
                    );
                    style_shape(&mut n);
                    n
                } else {
                    let route = if el.get("elbowed").and_then(|e| e.as_bool()) == Some(true) {
                        Route::Elbow
                    } else if el.get("roundness").is_some_and(|r| !r.is_null()) {
                        Route::Curved
                    } else {
                        Route::Straight
                    };
                    let (sh, eh) = if ty == "arrow" {
                        (
                            head(s(el, "startArrowhead")),
                            // Old arrows without the key point at their end.
                            match el.get("endArrowhead") {
                                None => Head::Arrow,
                                Some(v) => head(v.as_str()),
                            },
                        )
                    } else {
                        (Head::None, Head::None)
                    };
                    for (end, key) in [(0usize, "startBinding"), (1, "endBinding")] {
                        if let Some(bd) = el.get(key).filter(|b| !b.is_null())
                            && let Some(t) = s(bd, "elementId")
                        {
                            let fixed = bd.get("fixedPoint").and_then(|fp| {
                                let a = fp.as_array()?;
                                Some(v2(a.first()?.as_f64()?, a.get(1)?.as_f64()?))
                            });
                            let inside = s(bd, "mode") == Some("inside");
                            out.bindings
                                .push((id.clone(), end, t.to_string(), fixed, inside));
                        }
                    }
                    let mut n = Node::new(
                        NodeId(0),
                        NodeKind::Connector {
                            points: pts.iter().map(|p| *p - b.min()).collect(),
                            start: None,
                            end: None,
                            route,
                            start_head: sh,
                            end_head: eh,
                        },
                        b,
                    );
                    n.style.stroke = stroke;
                    n.style.stroke_width = sw;
                    n.style.opacity = opacity;
                    n.style.dash = dash.clone();
                    n.style.rough = rough;
                    n
                }
            }
            "freedraw" => {
                let pressures: Vec<f64> = el
                    .get("pressures")
                    .and_then(|p| p.as_array())
                    .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.5)).collect())
                    .unwrap_or_default();
                let simulate = el
                    .get("simulatePressure")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(true);
                let raw: Vec<V2> = el
                    .get("points")
                    .and_then(|p| p.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| {
                                let p = p.as_array()?;
                                Some(v2(x + p.first()?.as_f64()?, y + p.get(1)?.as_f64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if raw.is_empty() {
                    continue;
                }
                let width = sw * 4.25 * 0.6;
                let world: Vec<StrokePoint> = raw
                    .iter()
                    .enumerate()
                    .map(|(i, p)| StrokePoint {
                        x: p.x,
                        y: p.y,
                        p: if simulate {
                            0.5
                        } else {
                            pressures.get(i).copied().unwrap_or(0.5)
                        },
                    })
                    .collect();
                let b = crate::ink::bounds(&world, width, 0.6);
                let mut n = Node::new(
                    NodeId(0),
                    NodeKind::Stroke {
                        points: world
                            .iter()
                            .map(|p| StrokePoint {
                                x: p.x - b.x,
                                y: p.y - b.y,
                                p: p.p,
                            })
                            .collect(),
                        width,
                        thinning: if simulate { 0.3 } else { 0.6 },
                    },
                    b,
                );
                n.style.set_fill(stroke);
                n.style.opacity = opacity;
                n
            }
            "image" => {
                let mut n = Node::new(NodeId(0), NodeKind::Rect, r);
                n.name = "Image".into();
                n.style.opacity = opacity;
                let data = s(el, "fileId")
                    .and_then(|fid| files?.get(fid))
                    .and_then(|file| s(file, "dataURL"))
                    .and_then(|u| u.split_once(",").map(|(_, b)| b))
                    .and_then(crate::assets::base64_decode)
                    .and_then(|bytes| crate::assets::register(&bytes).ok());
                n.style.fills = match data {
                    Some(asset) => vec![Paint::Image {
                        asset,
                        mode: ImageMode::Stretch,
                        opacity: 1.0,
                    }],
                    None => vec![Paint::solid(Color::rgb(0xe0, 0xe0, 0xe6))],
                };
                n
            }
            "frame" | "magicframe" => {
                let mut n = Node::new(NodeId(0), NodeKind::Frame { clip: true }, r);
                n.name = s(el, "name").unwrap_or("Frame").to_string();
                n.style.fills.clear();
                n.style.stroke = Some(Color::rgb(0xbb, 0xbb, 0xc4));
                n
            }
            _ => continue,
        };
        let mut node = node;
        if el.get("locked").and_then(|l| l.as_bool()) == Some(true) {
            node.locked = true;
        }
        out.nesting.insert(id.clone(), (groups, frame_id));
        out.nodes.push((id, node));
    }
    out
}

/// Build a document from converted nodes: insert in z-order, then nest into frames and
/// groups (keeping world positions) and resolve arrow bindings.
fn build(mut imp: Imported) -> (Document, Vec<NodeId>) {
    let mut doc = Document::default();
    let mut rec = Recording::new("import", &doc);
    let mut tx = Tx::new(&mut doc, &mut rec);
    let mut ids: HashMap<String, NodeId> = HashMap::new();
    let mut order = Vec::new();
    for (eid, mut n) in imp.nodes.drain(..) {
        let id = tx.alloc_id();
        n.id = id;
        tx.insert(n, None, None);
        ids.insert(eid, id);
        order.push(id);
    }
    // Frames: frame children (listed below the frame in Excalidraw) move inside it.
    for (eid, (_, frame)) in &imp.nesting {
        if let (Some(fr), Some(id)) = (frame.as_ref().and_then(|f| ids.get(f)), ids.get(eid))
            && fr != id
        {
            tx.reparent(*id, Some(*fr), None);
        }
    }
    // Groups, outermost first so inner groups nest inside outer ones.
    let mut group_nodes: HashMap<String, NodeId> = HashMap::new();
    for id in &order {
        let Some((eid, _)) = ids.iter().find(|(_, v)| *v == id) else {
            continue;
        };
        let (groups, _) = imp.nesting[eid].clone();
        let mut parent = tx.doc().get(*id).and_then(|n| n.parent);
        for g in groups.iter().rev() {
            let gid = match group_nodes.get(g) {
                Some(x) => *x,
                None => {
                    let gid = tx.alloc_id();
                    let wr = tx.doc().world_rect(*id).unwrap_or_default();
                    let o = tx.doc().parent_origin(parent);
                    tx.insert(
                        Node::new(gid, NodeKind::Group, wr.translate(V2::ZERO - o)),
                        parent,
                        None,
                    );
                    group_nodes.insert(g.clone(), gid);
                    gid
                }
            };
            parent = Some(gid);
        }
        if parent != tx.doc().get(*id).and_then(|n| n.parent) {
            tx.reparent(*id, parent, None);
        }
    }
    let mut groups: Vec<NodeId> = group_nodes.values().copied().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(tx.doc().ancestors(*g).len()));
    for g in groups {
        crate::layout::normalize_group(&mut tx, g);
    }
    // Bindings.
    for (aid, end, target, fixed, inside) in imp.bindings {
        let (Some(a), Some(t)) = (ids.get(&aid).copied(), ids.get(&target).copied()) else {
            continue;
        };
        let Some(tr) = tx.doc().world_rect(t) else {
            continue;
        };
        let fixed = fixed.unwrap_or_else(|| {
            // Legacy focus/gap bindings: attach where the endpoint is today.
            let pts = crate::connector::resolve(tx.doc(), a).unwrap_or_default();
            let p = if end == 0 { pts.first() } else { pts.last() };
            crate::connector::fixed_point(p.copied().unwrap_or(tr.center()), tr)
        });
        let b = Binding {
            node: t,
            fixed,
            inside,
        };
        if let Some(n) = tx.node_mut(a)
            && let NodeKind::Connector { start, end: e, .. } = &mut n.kind
        {
            if end == 0 {
                *start = Some(b);
            } else {
                *e = Some(b);
            }
        }
    }
    let roots = tx.doc().root.clone();
    (doc, roots)
}

/// Parse a `.excalidraw` scene (or Excalidraw clipboard JSON) into a page document.
pub fn import_scene(json: &str) -> Result<Document, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
    let ty = s(&v, "type").unwrap_or("");
    if !matches!(
        ty,
        "excalidraw" | "excalidraw/clipboard" | "excalidraw-api/clipboard"
    ) {
        return Err(format!("not an Excalidraw scene (type {ty:?})"));
    }
    let elements = v
        .get("elements")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    let (mut doc, _) = build(convert(&elements, v.get("files")));
    doc.validate()?;
    doc.name = "Excalidraw".into();
    Ok(doc)
}

/// Parse a `.excalidrawlib` (version 1 or 2) into library items.
pub fn import_library(json: &str) -> Result<Vec<LibItem>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
    if s(&v, "type") != Some("excalidrawlib") {
        return Err("not an Excalidraw library".into());
    }
    let mut sets: Vec<(String, Vec<Value>)> = Vec::new();
    if let Some(items) = v.get("libraryItems").and_then(|x| x.as_array()) {
        for it in items {
            let els = it
                .get("elements")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            sets.push((s(it, "name").unwrap_or("Item").to_string(), els));
        }
    } else if let Some(lib) = v.get("library").and_then(|x| x.as_array()) {
        for els in lib {
            sets.push(("Item".into(), els.as_array().cloned().unwrap_or_default()));
        }
    }
    let mut out = Vec::new();
    for (name, els) in sets {
        let (doc, roots) = build(convert(&els, None));
        if roots.is_empty() {
            continue;
        }
        if let Some(item) = LibItem::from_selection(&doc, &roots, &name) {
            out.push(item);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(css_color("#1e1e1e"), Some(Color::rgb(0x1e, 0x1e, 0x1e)));
        assert_eq!(css_color("#fff"), Some(Color::WHITE));
        assert_eq!(css_color("transparent"), None);
        assert_eq!(css_color("#ff000080"), Some(Color::rgba(255, 0, 0, 128)));
    }

    #[test]
    fn imports_the_basic_fixture() {
        let json = include_str!("../../tests/fixtures/excalidraw/basic.excalidraw");
        let doc = import_scene(json).unwrap();
        let kinds: Vec<&str> = doc.nodes.values().map(|n| n.kind.type_name()).collect();
        for k in [
            "Rectangle",
            "Ellipse",
            "Polygon",
            "Text",
            "Arrow",
            "Frame",
            "Drawing",
            "Group",
        ] {
            assert!(kinds.contains(&k), "missing {k}: {kinds:?}");
        }
        // The arrow is bound at both ends.
        let arrow = doc.nodes.values().find(|n| {
            matches!(
                &n.kind,
                NodeKind::Connector {
                    start: Some(_),
                    end: Some(_),
                    ..
                }
            )
        });
        assert!(arrow.is_some(), "bound arrow");
        // Frame children live inside the frame.
        let frame = doc.nodes.values().find(|n| n.is_frame()).unwrap();
        assert!(!frame.children.is_empty());
    }

    #[test]
    fn imports_the_library_fixture() {
        let json = include_str!("../../tests/fixtures/excalidraw/library.excalidrawlib");
        let items = import_library(json).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| !i.nodes.is_empty()));
    }
}
