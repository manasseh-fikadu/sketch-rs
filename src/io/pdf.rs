//! Vector PDF export from the shared draw ops.
//!
//! One PDF page per exported area (a selection, each frame, or a whole page). Shapes, strokes
//! and glyph outlines are vector paths; gradients are axial/radial shadings; images are
//! deflated RGB XObjects with an alpha soft mask; opacity and all 16 blend modes go through
//! ExtGState; clipping frames clip. Text is outlined (not selectable), as in the SVG export.

use crate::assets;
use crate::geom::{Rect, V2};
use crate::model::{BlendMode, Cap, Color, Document, Join, NodeId, NodeKind, Stop};
use crate::render::draw::{self, DrawOp, PaintSpec};
use crate::text;
use crate::vector::{Seg, VPath};
use std::collections::HashMap;
use std::fmt::Write;

/// One page of output: what to draw and the area it covers.
pub struct PdfPage<'a> {
    pub doc: &'a Document,
    pub roots: Vec<NodeId>,
    pub bounds: Rect,
    pub background: Option<Color>,
}

fn num(x: f64) -> String {
    let r = (x * 1000.0).round() / 1000.0;
    if r == 0.0 {
        "0".into()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn blend_name(b: BlendMode) -> &'static str {
    match b {
        BlendMode::Normal => "Normal",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
        BlendMode::Overlay => "Overlay",
        BlendMode::Darken => "Darken",
        BlendMode::Lighten => "Lighten",
        BlendMode::ColorDodge => "ColorDodge",
        BlendMode::ColorBurn => "ColorBurn",
        BlendMode::HardLight => "HardLight",
        BlendMode::SoftLight => "SoftLight",
        BlendMode::Difference => "Difference",
        BlendMode::Exclusion => "Exclusion",
        BlendMode::Hue => "Hue",
        BlendMode::Saturation => "Saturation",
        BlendMode::Color => "Color",
        BlendMode::Luminosity => "Luminosity",
    }
}

/// Collects objects; ids are 1-based.
struct Objects {
    objs: Vec<Vec<u8>>,
}

impl Objects {
    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objs.push(body);
        self.objs.len()
    }
    fn reserve(&mut self) -> usize {
        self.add(Vec::new())
    }
    fn set(&mut self, id: usize, body: Vec<u8>) {
        self.objs[id - 1] = body;
    }
    fn stream(&mut self, dict: &str, data: &[u8]) -> usize {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        let mut b = format!(
            "<< {dict} /Filter /FlateDecode /Length {} >>\nstream\n",
            z.len()
        )
        .into_bytes();
        b.extend_from_slice(&z);
        b.extend_from_slice(b"\nendstream");
        self.add(b)
    }
}

/// Per-page resource registry.
#[derive(Default)]
struct Res {
    gstates: HashMap<(u32, u32, BlendMode), String>,
    gstate_objs: Vec<(String, usize)>,
    shadings: Vec<(String, usize)>,
    images: HashMap<String, (String, usize)>,
}

struct PageWriter<'a> {
    objs: &'a mut Objects,
    res: Res,
    out: String,
}

impl PageWriter<'_> {
    fn gstate(&mut self, fill_a: f32, stroke_a: f32, blend: BlendMode) -> String {
        let key = (
            (fill_a.clamp(0.0, 1.0) * 1000.0) as u32,
            (stroke_a.clamp(0.0, 1.0) * 1000.0) as u32,
            blend,
        );
        if let Some(n) = self.res.gstates.get(&key) {
            return n.clone();
        }
        let name = format!("G{}", self.res.gstates.len());
        let id = self.objs.add(
            format!(
                "<< /Type /ExtGState /ca {} /CA {} /BM /{} >>",
                num(key.0 as f64 / 1000.0),
                num(key.1 as f64 / 1000.0),
                blend_name(blend)
            )
            .into_bytes(),
        );
        self.res.gstates.insert(key, name.clone());
        self.res.gstate_objs.push((name.clone(), id));
        name
    }

    /// PDF path construction operators (quadratics elevated to cubics).
    fn emit_path(&mut self, p: &VPath) -> String {
        let mut o = String::new();
        let mut cur = V2::ZERO;
        let mut start = V2::ZERO;
        let pt = |o: &mut String, a: V2| {
            let _ = write!(o, "{} {} ", num(a.x), num(a.y));
        };
        for s in &p.segs {
            match *s {
                Seg::M(a) => {
                    pt(&mut o, a);
                    o.push_str("m ");
                    cur = a;
                    start = a;
                }
                Seg::L(a) => {
                    pt(&mut o, a);
                    o.push_str("l ");
                    cur = a;
                }
                Seg::Q(c, a) => {
                    pt(&mut o, cur + (c - cur) * (2.0 / 3.0));
                    pt(&mut o, a + (c - a) * (2.0 / 3.0));
                    pt(&mut o, a);
                    o.push_str("c ");
                    cur = a;
                }
                Seg::C(a, b, c) => {
                    pt(&mut o, a);
                    pt(&mut o, b);
                    pt(&mut o, c);
                    o.push_str("c ");
                    cur = c;
                }
                Seg::Z => {
                    o.push_str("h ");
                    cur = start;
                }
            }
        }
        o
    }

    fn rgb(c: [f32; 4]) -> String {
        format!(
            "{} {} {}",
            num(c[0] as f64),
            num(c[1] as f64),
            num(c[2] as f64)
        )
    }

    fn function(&mut self, stops: &[Stop]) -> usize {
        let c = |s: &Stop| {
            format!(
                "{} {} {}",
                num(s.color.r as f64 / 255.0),
                num(s.color.g as f64 / 255.0),
                num(s.color.b as f64 / 255.0)
            )
        };
        if stops.len() <= 2 {
            let (a, b) = (stops.first().unwrap(), stops.last().unwrap());
            return self.objs.add(
                format!(
                    "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
                    c(a),
                    c(b)
                )
                .into_bytes(),
            );
        }
        let mut fns = Vec::new();
        for w in stops.windows(2) {
            fns.push(
                self.objs.add(
                    format!(
                        "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
                        c(&w[0]),
                        c(&w[1])
                    )
                    .into_bytes(),
                ),
            );
        }
        let first = stops[0].pos;
        let last = stops.last().unwrap().pos;
        let bounds: Vec<String> = stops[1..stops.len() - 1]
            .iter()
            .map(|s| num(s.pos))
            .collect();
        let encode: Vec<&str> = (0..fns.len()).map(|_| "0 1").collect();
        self.objs.add(
            format!(
                "<< /FunctionType 3 /Domain [{} {}] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
                num(first.min(0.0)),
                num(last.max(1.0)),
                fns.iter()
                    .map(|f| format!("{f} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" "),
                bounds.join(" "),
                encode.join(" ")
            )
            .into_bytes(),
        )
    }

    fn shading(&mut self, dict: String) -> String {
        let name = format!("Sh{}", self.res.shadings.len());
        let id = self.objs.add(dict.into_bytes());
        self.res.shadings.push((name.clone(), id));
        name
    }

    fn image(&mut self, asset: &str) -> Option<String> {
        if let Some((n, _)) = self.res.images.get(asset) {
            return Some(n.clone());
        }
        let a = assets::get(asset)?;
        let (w, h) = (a.width, a.height);
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        let mut alpha = Vec::with_capacity((w * h) as usize);
        for p in a.rgba.pixels() {
            rgb.extend_from_slice(&p.0[..3]);
            alpha.push(p.0[3]);
        }
        let smask = self.objs.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8"
            ),
            &alpha,
        );
        let id = self.objs.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask {smask} 0 R"
            ),
            &rgb,
        );
        let name = format!("Im{}", self.res.images.len());
        self.res.images.insert(asset.into(), (name.clone(), id));
        Some(name)
    }

    fn ops(&mut self, ops: &[DrawOp], blend: BlendMode) {
        for op in ops {
            match op {
                DrawOp::Fill {
                    path,
                    paint,
                    even_odd,
                } => {
                    let p = self.emit_path(path);
                    let clip_op = if *even_odd { "W* n" } else { "W n" };
                    match paint {
                        PaintSpec::Solid(c) => {
                            let gs = self.gstate(c[3], 1.0, blend);
                            let _ = writeln!(
                                self.out,
                                "q /{gs} gs {} rg {p}{}\nQ",
                                Self::rgb(*c),
                                if *even_odd { "f*" } else { "f" }
                            );
                        }
                        PaintSpec::Linear {
                            p0,
                            p1,
                            stops,
                            opacity,
                        } => {
                            let a = stops.first().map_or(1.0, |s| s.color.a as f32 / 255.0);
                            let gs = self.gstate(opacity * a, 1.0, blend);
                            let f = self.function(stops);
                            let sh = self.shading(format!(
                                "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{} {} {} {}] /Function {f} 0 R /Extend [true true] >>",
                                num(p0.x),
                                num(p0.y),
                                num(p1.x),
                                num(p1.y)
                            ));
                            let _ = writeln!(self.out, "q /{gs} gs {p}{clip_op} /{sh} sh\nQ");
                        }
                        PaintSpec::Radial {
                            center,
                            rx,
                            ry,
                            stops,
                            opacity,
                        } => {
                            let a = stops.first().map_or(1.0, |s| s.color.a as f32 / 255.0);
                            let gs = self.gstate(opacity * a, 1.0, blend);
                            let f = self.function(stops);
                            let sh = self.shading(format!(
                                "<< /ShadingType 3 /ColorSpace /DeviceRGB /Coords [0 0 0 0 0 1] /Function {f} 0 R /Extend [true true] >>"
                            ));
                            let _ = writeln!(
                                self.out,
                                "q /{gs} gs {p}{clip_op} {} {} {} {} {} {} cm /{sh} sh\nQ",
                                num(rx.x),
                                num(rx.y),
                                num(ry.x),
                                num(ry.y),
                                num(center.x),
                                num(center.y)
                            );
                        }
                        PaintSpec::Image {
                            asset,
                            place,
                            tile,
                            opacity,
                        } => {
                            let Some(im) = self.image(asset) else {
                                continue;
                            };
                            let gs = self.gstate(*opacity, 1.0, blend);
                            let _ = write!(self.out, "q /{gs} gs {p}{clip_op} ");
                            let mut placements = vec![*place];
                            if *tile {
                                placements.clear();
                                let b = path.bounds().unwrap_or(*place);
                                let (w, h) = (place.w.max(1.0), place.h.max(1.0));
                                let x0 = place.x + ((b.x - place.x) / w).floor() * w;
                                let y0 = place.y + ((b.y - place.y) / h).floor() * h;
                                let mut y = y0;
                                while y < b.bottom() && placements.len() < 2000 {
                                    let mut x = x0;
                                    while x < b.right() && placements.len() < 2000 {
                                        placements.push(Rect::new(x, y, w, h));
                                        x += w;
                                    }
                                    y += h;
                                }
                            }
                            for r in placements {
                                let _ = write!(
                                    self.out,
                                    "q {} 0 0 {} {} {} cm /{im} Do Q ",
                                    num(r.w),
                                    num(-r.h),
                                    num(r.x),
                                    num(r.y + r.h)
                                );
                            }
                            self.out.push_str("Q\n");
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
                    let p = self.emit_path(path);
                    let gs = self.gstate(1.0, color[3], blend);
                    let cap = match cap {
                        Cap::Butt => 0,
                        Cap::Round => 1,
                        Cap::Square => 2,
                    };
                    let join = match join {
                        Join::Miter => 0,
                        Join::Round => 1,
                        Join::Bevel => 2,
                    };
                    let d: Vec<String> = dash.iter().map(|x| num(*x)).collect();
                    let _ = writeln!(
                        self.out,
                        "q /{gs} gs {} RG {} w {cap} J {join} j [{}] 0 d {p}S\nQ",
                        Self::rgb(*color),
                        num(*width),
                        d.join(" ")
                    );
                }
                DrawOp::Glyphs {
                    face,
                    size,
                    color,
                    glyphs,
                } => {
                    let mut p = VPath::new();
                    for (gid, o) in glyphs {
                        p.append(&text::glyph_path(*face, *gid, *o, *size));
                    }
                    let path = self.emit_path(&p);
                    let gs = self.gstate(color[3], 1.0, blend);
                    let _ = writeln!(self.out, "q /{gs} gs {} rg {path}f\nQ", Self::rgb(*color));
                }
            }
        }
    }

    fn node(&mut self, doc: &Document, id: NodeId, opacity: f64) {
        let Some(n) = doc.get(id) else { return };
        if !n.visible {
            return;
        }
        let wr = doc.world_rect(id).unwrap();
        let op = opacity * n.style.opacity;
        let d = draw::node_draw(n, wr, op);
        self.ops(&d.ops, d.blend);
        if n.children.is_empty() {
            return;
        }
        let clip = matches!(n.kind, NodeKind::Frame { clip: true });
        if clip {
            let p = self.emit_path(&draw::shape_path(n, wr).unwrap_or_default());
            let _ = writeln!(self.out, "q {p}W n");
        }
        for c in &n.children {
            self.node(doc, *c, op);
        }
        if clip {
            self.out.push_str("Q\n");
        }
    }
}

/// Write a PDF with one page per entry.
pub fn export(pages: &[PdfPage]) -> Vec<u8> {
    let mut objs = Objects { objs: Vec::new() };
    let catalog = objs.reserve();
    let pages_id = objs.reserve();
    let mut kids = Vec::new();
    for pg in pages {
        let b = pg.bounds;
        let mut w = PageWriter {
            objs: &mut objs,
            res: Res::default(),
            out: String::new(),
        };
        // y-down user space with the page's top-left at the area's top-left.
        let _ = writeln!(w.out, "1 0 0 -1 {} {} cm", num(-b.x), num(b.y + b.h));
        if let Some(bg) = pg.background {
            let c = bg.to_f32(1.0);
            let _ = writeln!(
                w.out,
                "{} rg {} {} {} {} re f",
                PageWriter::rgb(c),
                num(b.x),
                num(b.y),
                num(b.w),
                num(b.h)
            );
        }
        for r in &pg.roots {
            let op = draw::inherited_opacity(pg.doc, *r);
            w.node(pg.doc, *r, op);
        }
        let content = std::mem::take(&mut w.out);
        let res = std::mem::take(&mut w.res);
        let content_id = objs.stream("", content.as_bytes());
        let dict = |items: &[(String, usize)]| {
            items
                .iter()
                .map(|(n, id)| format!("/{n} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let images: Vec<(String, usize)> = res.images.values().cloned().collect();
        let page = objs.add(
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Contents {content_id} 0 R /Resources << /ExtGState << {} >> /Shading << {} >> /XObject << {} >> >> >>",
                num(b.w),
                num(b.h),
                dict(&res.gstate_objs),
                dict(&res.shadings),
                dict(&images)
            )
            .into_bytes(),
        );
        kids.push(page);
    }
    objs.set(
        catalog,
        format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes(),
    );
    objs.set(
        pages_id,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.iter()
                .map(|k| format!("{k} 0 R"))
                .collect::<Vec<_>>()
                .join(" "),
            kids.len()
        )
        .into_bytes(),
    );
    let mut out: Vec<u8> = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut x = format!("xref\n0 {}\n0000000000 65535 f \n", objs.objs.len() + 1);
    for o in offsets {
        let _ = writeln!(x, "{o:010} 00000 n ");
    }
    let _ = write!(
        x,
        "trailer\n<< /Size {} /Root {catalog} 0 R /Info << /Producer (Sketch) >> >>\nstartxref\n{xref}\n%%EOF\n",
        objs.objs.len() + 1
    );
    out.extend_from_slice(x.as_bytes());
    out
}

/// Pages for a document: one per top-level frame (`frames`), else one for `roots` / all.
pub fn pages_for<'a>(
    doc: &'a Document,
    roots: Option<&[NodeId]>,
    frames: bool,
    background: Option<Color>,
) -> Vec<PdfPage<'a>> {
    let roots: Vec<NodeId> = roots
        .map(|r| r.to_vec())
        .unwrap_or_else(|| doc.root.clone());
    if frames {
        let fr: Vec<NodeId> = roots
            .iter()
            .copied()
            .filter(|id| doc.get(*id).is_some_and(|n| n.is_frame() && n.visible))
            .collect();
        if !fr.is_empty() {
            return fr
                .into_iter()
                .filter_map(|f| {
                    Some(PdfPage {
                        doc,
                        bounds: crate::io::svg::export_bounds(doc, &[f])?,
                        roots: vec![f],
                        background,
                    })
                })
                .collect();
        }
    }
    match crate::io::svg::export_bounds(doc, &roots) {
        Some(b) => vec![PdfPage {
            doc,
            roots,
            bounds: b,
            background,
        }],
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_structure_is_valid() {
        let ed = crate::demo::showcase();
        let pages = pages_for(&ed.doc, None, true, Some(Color::WHITE));
        assert_eq!(pages.len(), 2, "one page per top-level frame");
        let pdf = export(&pages);
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.starts_with("%PDF-1.7"));
        assert!(s.contains("/Count 2"));
        assert!(s.trim_end().ends_with("%%EOF"));
        // The xref offsets point at the objects (check on raw bytes: streams are binary).
        let tail = &pdf[pdf.len() - 64..];
        let tail = String::from_utf8_lossy(tail);
        let xref_at: usize = tail
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(pdf[xref_at..].starts_with(b"xref"));
        let table = String::from_utf8_lossy(&pdf[xref_at..]);
        let first_off: usize = table.lines().nth(3).unwrap()[..10].parse().unwrap();
        assert!(pdf[first_off..].starts_with(b"1 0 obj"));
    }
}
