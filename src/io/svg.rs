//! SVG export from the shared draw ops. Text is emitted as glyph outlines so the file renders
//! identically without fonts; images are embedded as data URLs.

use crate::assets;
use crate::geom::Rect;
use crate::model::{BlendMode, Color, Document, NodeId, NodeKind, Stop};
use crate::render::draw::{self, DrawOp, PaintSpec};
use crate::text;
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn n(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r == 0.0 {
        "0".into()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn color_attr(attr: &str, c: [f32; 4]) -> String {
    if c[3] >= 0.999 {
        format!(" {attr}=\"{}\"", hex(c))
    } else {
        format!(
            " {attr}=\"{}\" {attr}-opacity=\"{}\"",
            hex(c),
            n(c[3] as f64)
        )
    }
}

/// Bounds used for export: drawn bounds of the roots and their subtrees.
pub fn export_bounds(doc: &Document, roots: &[NodeId]) -> Option<Rect> {
    let mut rects = Vec::new();
    for r in roots {
        if !doc.get(*r).is_some_and(|n| n.visible) {
            continue;
        }
        for id in doc.subtree(*r) {
            let (Some(nd), Some(wr)) = (doc.get(id), doc.world_rect(id)) else {
                continue;
            };
            if nd.is_group() || !doc.is_effectively_visible(id) {
                continue;
            }
            // Children of clipping frames can't draw outside them.
            if doc.ancestors(id).iter().any(|a| {
                matches!(
                    doc.get(*a).map(|x| &x.kind),
                    Some(NodeKind::Frame { clip: true })
                )
            }) {
                continue;
            }
            rects.push(draw::draw_bounds(nd, wr));
        }
    }
    Rect::union_all(&rects).map(|r| {
        Rect::new(
            r.x.floor(),
            r.y.floor(),
            (r.right().ceil() - r.x.floor()).max(1.0),
            (r.bottom().ceil() - r.y.floor()).max(1.0),
        )
    })
}

struct Writer<'a> {
    doc: &'a Document,
    out: String,
    defs: String,
    next_def: usize,
}

impl Writer<'_> {
    fn def_id(&mut self, prefix: &str) -> String {
        self.next_def += 1;
        format!("{prefix}{}", self.next_def)
    }

    fn stops(&mut self, stops: &[Stop], opacity: f32) -> String {
        let mut s = String::new();
        for st in stops {
            let c: Color = st.color;
            let a = c.a as f64 / 255.0 * opacity as f64;
            let _ = write!(
                s,
                "<stop offset=\"{}\" stop-color=\"#{:02x}{:02x}{:02x}\"{}/>",
                n(st.pos),
                c.r,
                c.g,
                c.b,
                if a < 0.999 {
                    format!(" stop-opacity=\"{}\"", n(a))
                } else {
                    String::new()
                }
            );
        }
        s
    }

    /// The fill attribute(s) for a paint, adding defs as needed. Images return None and are
    /// written as clipped `<image>` elements instead.
    fn fill_attr(&mut self, paint: &PaintSpec) -> Option<String> {
        Some(match paint {
            PaintSpec::Solid(c) => color_attr("fill", *c),
            PaintSpec::Linear {
                p0,
                p1,
                stops,
                opacity,
            } => {
                let id = self.def_id("g");
                let st = self.stops(stops, *opacity);
                let _ = write!(
                    self.defs,
                    "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{st}</linearGradient>",
                    n(p0.x),
                    n(p0.y),
                    n(p1.x),
                    n(p1.y)
                );
                format!(" fill=\"url(#{id})\"")
            }
            PaintSpec::Radial {
                center,
                rx,
                ry,
                stops,
                opacity,
            } => {
                let id = self.def_id("g");
                let st = self.stops(stops, *opacity);
                let _ = write!(
                    self.defs,
                    "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"0\" cy=\"0\" r=\"1\" gradientTransform=\"matrix({} {} {} {} {} {})\">{st}</radialGradient>",
                    n(rx.x),
                    n(rx.y),
                    n(ry.x),
                    n(ry.y),
                    n(center.x),
                    n(center.y)
                );
                format!(" fill=\"url(#{id})\"")
            }
            PaintSpec::Image { .. } => return None,
        })
    }

    fn ops(&mut self, ops: &[DrawOp], ind: &str) {
        for op in ops {
            match op {
                DrawOp::Fill {
                    path,
                    paint,
                    even_odd,
                } => {
                    let rule = if *even_odd {
                        " fill-rule=\"evenodd\""
                    } else {
                        ""
                    };
                    let d = path.to_svg();
                    match self.fill_attr(paint) {
                        Some(fill) => {
                            let _ = writeln!(self.out, "{ind}<path d=\"{d}\"{fill}{rule}/>");
                        }
                        None => {
                            let PaintSpec::Image {
                                asset,
                                place,
                                tile,
                                opacity,
                            } = paint
                            else {
                                unreachable!()
                            };
                            let Some(a) = assets::get(asset) else {
                                continue;
                            };
                            let href =
                                format!("data:{};base64,{}", a.mime(), assets::base64(&a.bytes));
                            let op = if *opacity < 0.999 {
                                format!(" opacity=\"{}\"", n(*opacity as f64))
                            } else {
                                String::new()
                            };
                            if *tile {
                                let id = self.def_id("p");
                                let _ = write!(
                                    self.defs,
                                    "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"><image href=\"{href}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\"/></pattern>",
                                    n(place.x),
                                    n(place.y),
                                    n(place.w),
                                    n(place.h),
                                    n(place.w),
                                    n(place.h)
                                );
                                let _ = writeln!(
                                    self.out,
                                    "{ind}<path d=\"{d}\" fill=\"url(#{id})\"{rule}{op}/>"
                                );
                            } else {
                                let id = self.def_id("c");
                                let _ = write!(
                                    self.defs,
                                    "<clipPath id=\"{id}\"><path d=\"{d}\"{}/></clipPath>",
                                    rule.replace("fill-rule", "clip-rule")
                                );
                                let _ = writeln!(
                                    self.out,
                                    "{ind}<image href=\"{href}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" clip-path=\"url(#{id})\"{op}/>",
                                    n(place.x),
                                    n(place.y),
                                    n(place.w),
                                    n(place.h)
                                );
                            }
                        }
                    }
                }
                DrawOp::Stroke {
                    path,
                    color,
                    width,
                    cap,
                    join,
                    dash,
                } => {
                    let cap = match cap {
                        crate::model::Cap::Butt => "",
                        crate::model::Cap::Round => " stroke-linecap=\"round\"",
                        crate::model::Cap::Square => " stroke-linecap=\"square\"",
                    };
                    let join = match join {
                        crate::model::Join::Miter => "",
                        crate::model::Join::Round => " stroke-linejoin=\"round\"",
                        crate::model::Join::Bevel => " stroke-linejoin=\"bevel\"",
                    };
                    let dash = if dash.is_empty() {
                        String::new()
                    } else {
                        let v: Vec<String> = dash.iter().map(|d| n(*d)).collect();
                        format!(" stroke-dasharray=\"{}\"", v.join(" "))
                    };
                    let _ = writeln!(
                        self.out,
                        "{ind}<path d=\"{}\" fill=\"none\"{} stroke-width=\"{}\"{cap}{join}{dash}/>",
                        path.to_svg(),
                        color_attr("stroke", *color),
                        n(*width)
                    );
                }
                DrawOp::Glyphs {
                    face,
                    size,
                    color,
                    glyphs,
                } => {
                    let mut d = String::new();
                    for (gid, o) in glyphs {
                        d.push_str(&text::glyph_path(*face, *gid, *o, *size).to_svg());
                    }
                    if !d.is_empty() {
                        let _ = writeln!(
                            self.out,
                            "{ind}<path d=\"{d}\"{}/>",
                            color_attr("fill", *color)
                        );
                    }
                }
            }
        }
    }

    fn node(&mut self, id: NodeId, opacity: f64, depth: usize) {
        let doc = self.doc;
        let Some(nd) = doc.get(id) else { return };
        if !nd.visible {
            return;
        }
        let wr = doc.world_rect(id).unwrap();
        let ind = "  ".repeat(depth + 1);
        let op = opacity * nd.style.opacity;
        let blend = if nd.style.blend != BlendMode::Normal {
            format!(" style=\"mix-blend-mode:{}\"", nd.style.blend.css())
        } else {
            String::new()
        };
        let label = esc(&nd.name);
        let aria = match &nd.kind {
            NodeKind::Text { text, .. } => format!(" aria-label=\"{}\"", esc(text)),
            _ => String::new(),
        };
        let _ = writeln!(
            self.out,
            "{ind}<g id=\"n{}\" data-name=\"{label}\"{aria}{blend}>",
            id.0
        );
        let d = draw::node_draw(nd, wr, op);
        self.ops(&d.ops, &format!("{ind}  "));
        if !nd.children.is_empty() {
            let clip = matches!(nd.kind, NodeKind::Frame { clip: true });
            if clip {
                let cid = self.def_id("clip");
                let path = draw::shape_path(nd, wr).unwrap_or_default();
                let _ = write!(
                    self.defs,
                    "<clipPath id=\"{cid}\"><path d=\"{}\"/></clipPath>",
                    path.to_svg()
                );
                let _ = writeln!(self.out, "{ind}  <g clip-path=\"url(#{cid})\">");
            }
            for c in &nd.children {
                self.node(*c, op, depth + 2);
            }
            if clip {
                let _ = writeln!(self.out, "{ind}  </g>");
            }
        }
        let _ = writeln!(self.out, "{ind}</g>");
    }
}

/// Export the given roots (or all top-level nodes) to an SVG string.
pub fn export(
    doc: &Document,
    roots: Option<&[NodeId]>,
    background: Option<Color>,
) -> Option<String> {
    let roots: Vec<NodeId> = roots
        .map(|r| r.to_vec())
        .unwrap_or_else(|| doc.root.clone());
    let b = export_bounds(doc, &roots)?;
    export_in(doc, &roots, b, background)
}

/// Export `roots` into the view box `b`.
pub fn export_in(
    doc: &Document,
    roots: &[NodeId],
    b: Rect,
    background: Option<Color>,
) -> Option<String> {
    let mut w = Writer {
        doc,
        out: String::new(),
        defs: String::new(),
        next_def: 0,
    };
    if let Some(bg) = background {
        let _ = writeln!(
            w.out,
            r#"  <rect x="{}" y="{}" width="{}" height="{}"{}/>"#,
            n(b.x),
            n(b.y),
            n(b.w),
            n(b.h),
            color_attr("fill", bg.to_f32(1.0))
        );
    }
    for id in roots {
        let op = draw::inherited_opacity(doc, *id);
        w.node(*id, op, 0);
    }
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        n(b.w),
        n(b.h),
        n(b.x),
        n(b.y),
        n(b.w),
        n(b.h)
    );
    let _ = writeln!(s, "<title>{}</title>", esc(&doc.name));
    if !w.defs.is_empty() {
        let _ = writeln!(s, "<defs>{}</defs>", w.defs);
    }
    s.push_str(&w.out);
    s.push_str("</svg>\n");
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::geom::Rect;
    use crate::model::Paint;

    #[test]
    fn svg_is_well_formed_and_contains_all_kinds() {
        let mut ed = demo::showcase();
        let mut g =
            crate::model::Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 50.0, 50.0));
        g.style.fills = vec![
            Paint::linear(Color::BLACK, Color::WHITE),
            Paint::radial(Color::WHITE, Color::rgba(0, 0, 0, 0)),
        ];
        g.style.blend = BlendMode::Multiply;
        g.style.dash = vec![4.0, 2.0];
        g.style.stroke = Some(Color::BLACK);
        ed.add_node(g, None);
        let svg = export(&ed.doc, None, Some(Color::WHITE)).unwrap();
        for tag in [
            "<svg",
            "<path",
            "clipPath",
            "linearGradient",
            "radialGradient",
            "mix-blend-mode:multiply",
            "stroke-dasharray",
            "</svg>",
        ] {
            assert!(svg.contains(tag), "missing {tag}");
        }
        assert_eq!(svg.matches("<g").count(), svg.matches("</g>").count());
        assert!(!svg.contains("NaN"));
    }

    #[test]
    fn escapes_names() {
        let mut ed = crate::editor::Editor::default();
        let mut n =
            crate::model::Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 5.0, 5.0));
        n.name = "<a & \"b\">".into();
        ed.add_node(n, None);
        let svg = export(&ed.doc, None, None).unwrap();
        assert!(svg.contains("&lt;a &amp; &quot;b&quot;&gt;"));
    }

    #[test]
    fn image_fills_embed_data_urls() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 128, 255, 255]));
        let id = assets::register_rgba(&img);
        let mut ed = crate::editor::Editor::default();
        let mut node =
            crate::model::Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 40.0, 20.0));
        node.style.fills = vec![Paint::Image {
            asset: id,
            mode: crate::model::ImageMode::Fill,
            opacity: 1.0,
        }];
        ed.add_node(node, None);
        let svg = export(&ed.doc, None, None).unwrap();
        assert!(svg.contains("data:image/png;base64,"));
        assert!(svg.contains("<image"));
    }
}
