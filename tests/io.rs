//! Milestone 6: export (PNG/SVG/PDF of selections, frames, pages), import (.excalidraw,
//! .excalidrawlib, SVG) and the clipboard between pages, projects and apps.

mod common;

use common::*;
use sketch_rs::editor::shortcuts::Action;
use sketch_rs::editor::{Editor, Key, Modifiers};
use sketch_rs::geom::{Rect, V2, v2};
use sketch_rs::io::clipboard::{Memory, Provider};
use sketch_rs::io::export::{self, Format, Scope, Spec};
use sketch_rs::model::{Color, ImageMode, Node, NodeId, NodeKind, Paint};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sketch-rs-io-{tag}-{:016x}", fastrand::u64(..)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// pdftoppm, when installed, renders a PDF page so we can look at it.
fn rasterize_pdf(pdf: &PathBuf, out_stem: &PathBuf, page: usize) -> Option<image::RgbaImage> {
    let st = std::process::Command::new("pdftoppm")
        .args([
            "-png",
            "-r",
            "96",
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-singlefile",
        ])
        .arg(pdf)
        .arg(out_stem)
        .status()
        .ok()?;
    if !st.success() {
        return None;
    }
    Some(image::open(out_stem.with_extension("png")).ok()?.to_rgba8())
}

fn board() -> Editor {
    let mut e = sketch_rs::demo::showcase();
    // A gradient, an image fill and a multiply blend, so the PDF exercises every paint.
    let mut g = Node::new(
        NodeId(0),
        NodeKind::Ellipse,
        Rect::new(700.0, 500.0, 160.0, 120.0),
    );
    g.style.fills = vec![Paint::linear(
        Color::rgb(255, 80, 80),
        Color::rgb(60, 60, 255),
    )];
    e.add_node(g, None);
    let img = image::RgbaImage::from_fn(40, 30, |x, y| {
        image::Rgba([(x * 6) as u8, (y * 8) as u8, 180, 255])
    });
    let asset = sketch_rs::assets::register_rgba(&img);
    let mut r = Node::new(
        NodeId(0),
        NodeKind::Rect,
        Rect::new(900.0, 500.0, 160.0, 120.0),
    );
    r.style.fills = vec![Paint::Image {
        asset,
        mode: ImageMode::Fill,
        opacity: 1.0,
    }];
    r.style.corner_radius = 16.0;
    e.add_node(r, None);
    e
}

#[test]
fn m6_pdf_export_pages_frames_and_selection() {
    let e = board();
    let dir = tmp("pdf");
    let pages = [("Launch plan", &e.doc)];
    // Each frame → one PDF page per frame.
    let spec = Spec {
        format: Format::Pdf,
        scope: Scope::Frames,
        ..Spec::default()
    };
    let items = export::items(&spec, &pages, 0, &[]);
    assert_eq!(items.len(), 2);
    let out = dir.join("frames.pdf");
    export::write(&spec, &items, &out, None).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Count 2"));
    // Whole page, rendered back through pdftoppm when available.
    let spec = Spec {
        format: Format::Pdf,
        scope: Scope::Page,
        ..Spec::default()
    };
    let out = dir.join("page.pdf");
    export::write(&spec, &export::items(&spec, &pages, 0, &[]), &out, None).unwrap();
    let text = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).to_string();
    assert!(text.contains("/ShadingType 2"), "gradient as a shading");
    assert!(text.contains("/Subtype /Image"), "image XObject");
    if let Some(img) = rasterize_pdf(&out, &shot("milestone6_pdf_page"), 1) {
        // The yellow note on the week plan is visible in the rasterized PDF.
        let yellow = Color::rgb(0xff, 0xe0, 0x82);
        let n = img.pixels().filter(|p| close(p.0, yellow, 12)).count();
        assert!(n > 1000, "yellow pixels in PDF render: {n}");
    }
    // Selection: one layer → its own bounds.
    let id = e.doc.root[0];
    let spec = Spec {
        format: Format::Pdf,
        scope: Scope::Selection,
        ..Spec::default()
    };
    let items = export::items(&spec, &pages, 0, &[id]);
    assert_eq!(items[0].roots, vec![id]);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn m6_png_and_svg_export_of_frames_and_all_pages() {
    let Some(g) = gpu() else { return };
    let mut e = board();
    let p2 = e.add_page();
    e.add_node(
        Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 50.0, 50.0)),
        None,
    );
    e.switch_page(0);
    let names: Vec<String> = e.pages.iter().map(|p| p.name.clone()).collect();
    let pages: Vec<(&str, &sketch_rs::model::Document)> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), e.page_doc(i)))
        .collect();
    let dir = tmp("png");
    let spec = Spec {
        format: Format::Png,
        scope: Scope::Frames,
        scale: 1.0,
        transparent: true,
    };
    let files = export::write(
        &spec,
        &export::items(&spec, &pages, 0, &[]),
        &dir.join("frames"),
        Some(g),
    )
    .unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .any(|f| f.file_name().unwrap() == "Week plan.png")
    );
    let img = image::open(&files[0]).unwrap();
    assert!(img.width() >= 400);
    let spec = Spec {
        format: Format::Svg,
        scope: Scope::AllPages,
        ..Spec::default()
    };
    let files = export::write(
        &spec,
        &export::items(&spec, &pages, 0, &[]),
        &dir.join("pages"),
        Some(g),
    )
    .unwrap();
    assert_eq!(files.len(), 2);
    let _ = p2;
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn m6_import_excalidraw_and_svg_screenshots() {
    let Some(g) = gpu() else { return };
    let json = include_str!("fixtures/excalidraw/basic.excalidraw");
    let doc = sketch_rs::io::excalidraw::import_scene(json).unwrap();
    let mut e = editor(1200.0, 800.0);
    e.load(doc, None, None);
    e.viewport = v2(1200.0, 800.0);
    e.zoom_to_fit();
    let img = render_canvas(g, &e);
    img.save(shot("milestone6_excalidraw_import.png")).unwrap();
    // Hand-drawn shapes rendered: plenty of dark stroke pixels.
    assert!(count(&img, Rect::new(0.0, 0.0, 1200.0, 800.0), is_dark) > 2000);

    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="200" viewBox="0 0 300 200">
      <defs><radialGradient id="r"><stop offset="0" stop-color="#ffe066"/><stop offset="1" stop-color="#ff6b6b"/></radialGradient></defs>
      <circle cx="80" cy="100" r="70" fill="url(#r)"/>
      <path d="M170 30 L280 30 L225 170 Z" fill="#4dabf7" stroke="#1c3d5a" stroke-width="6" stroke-linejoin="round"/>
      <text x="40" y="195" font-size="18" fill="#222">SVG import</text></svg>"##;
    let doc = sketch_rs::io::svg_import::import_page(svg, "logo").unwrap();
    let mut e = editor(800.0, 560.0);
    e.load(doc, None, None);
    e.viewport = v2(800.0, 560.0);
    e.zoom_to_fit();
    let img = render_canvas(g, &e);
    img.save(shot("milestone6_svg_import.png")).unwrap();
    let blue = Color::rgb(0x4d, 0xab, 0xf7);
    assert!(
        count(&img, Rect::new(0.0, 0.0, 800.0, 560.0), |p| close(
            p, blue, 10
        )) > 1000
    );
}

/// A clipboard shared by two "apps".
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Memory>>);

impl Provider for Shared {
    fn read_text(&mut self) -> Option<String> {
        self.0.lock().unwrap().read_text()
    }
    fn write_text(&mut self, s: &str) {
        self.0.lock().unwrap().write_text(s)
    }
    fn read_image(&mut self) -> Option<image::RgbaImage> {
        self.0.lock().unwrap().read_image()
    }
    fn write_image(&mut self, img: &image::RgbaImage) {
        self.0.lock().unwrap().write_image(img)
    }
}

#[test]
fn m6_clipboard_between_apps_and_from_other_tools() {
    let clip = Shared::default();
    let mut a = board();
    a.clipboard_io = Some(Box::new(clip.clone()));
    let mut b = editor(1000.0, 700.0);
    b.clipboard_io = Some(Box::new(clip.clone()));
    // Copy a frame (with children) and the image layer in "app A", paste in "app B".
    let frame = a.doc.root[0];
    let img_layer = *a.doc.root.last().unwrap();
    a.selection = vec![frame, img_layer];
    key(&mut a, Key::Char('c'), Modifiers::cmd());
    key(&mut b, Key::Char('v'), Modifiers::cmd());
    assert_eq!(b.selection.len(), 2);
    let pasted_frame = *b
        .selection
        .iter()
        .find(|id| b.doc.get(**id).unwrap().is_frame())
        .unwrap();
    assert_eq!(
        b.doc.subtree(pasted_frame).len(),
        a.doc.subtree(frame).len(),
        "children came along"
    );
    assert_eq!(
        b.doc.world_rect(pasted_frame),
        a.doc.world_rect(frame),
        "positions kept"
    );
    // Pasting in A again (its own copy) uses the internal clipboard: offset on the same page.
    let n = a.doc.nodes.len();
    key(&mut a, Key::Char('v'), Modifiers::cmd());
    assert!(a.doc.nodes.len() > n);
    // SVG markup from another app.
    clip.0.lock().unwrap().write_text(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#);
    let n = b.doc.nodes.len();
    b.run(Action::Paste);
    assert!(b.doc.nodes.len() > n);
    // Excalidraw clipboard JSON.
    clip.0.lock().unwrap().write_text(
        r##"{"type":"excalidraw/clipboard","elements":[{"id":"x","type":"rectangle","x":0,"y":0,"width":80,"height":40,"strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","fillStyle":"hachure","strokeWidth":2,"roughness":1,"opacity":100,"seed":5}]}"##,
    );
    b.run(Action::Paste);
    let r = b.doc.get(b.selection[0]).unwrap();
    assert!(r.style.rough.is_some(), "Excalidraw shapes stay hand-drawn");
    // Plain text becomes a text layer; an image becomes an image layer.
    clip.0
        .lock()
        .unwrap()
        .write_text("Hello from the clipboard");
    b.run(Action::Paste);
    assert_eq!(
        b.doc.get(b.selection[0]).unwrap().kind.text().unwrap(),
        "Hello from the clipboard"
    );
    {
        let mut m = clip.0.lock().unwrap();
        m.text = None;
        m.image = Some(image::RgbaImage::from_pixel(
            20,
            10,
            image::Rgba([0, 200, 0, 255]),
        ));
    }
    b.run(Action::Paste);
    assert!(matches!(
        b.doc.get(b.selection[0]).unwrap().style.fills[0],
        Paint::Image { .. }
    ));
    // Copy as SVG writes markup.
    b.selection = vec![pasted_frame];
    b.run(Action::CopyAsSvg);
    assert!(
        clip.0
            .lock()
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .starts_with("<svg")
    );
    let _ = V2::ZERO;
}

#[test]
fn m6_cli_exports_pdf_and_imports_excalidraw() {
    let dir = tmp("cli");
    let exe = env!("CARGO_BIN_EXE_sketch-rs");
    let pdf = dir.join("demo.pdf");
    let st = std::process::Command::new(exe)
        .args(["--export-pdf"])
        .arg(&pdf)
        .args(["--demo", "--frames"])
        .status()
        .unwrap();
    assert!(st.success());
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-1.7"));
    let out = std::process::Command::new(exe)
        .env("SKETCH_RS_HOME", dir.join("home"))
        .arg("--import")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/excalidraw/basic.excalidraw"
        ))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let proj = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    assert!(proj.join("project.json").is_file());
    let p = sketch_rs::project::Project::open(&proj).unwrap();
    let pages = p.load_pages().unwrap();
    assert!(pages[0].doc.nodes.len() >= 8);
    // The fixture's image came along as a project asset.
    assert!(
        std::fs::read_dir(proj.join("assets"))
            .map(|d| d.count())
            .unwrap_or(0)
            >= 1
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn cli_reads_paper_rs_home_from_before_the_rename() {
    let dir = tmp("old-home");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sketch-rs"))
        .env_remove("SKETCH_RS_HOME")
        .env("PAPER_RS_HOME", dir.join("home"))
        .arg("--import")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/excalidraw/basic.excalidraw"
        ))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let proj = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    assert!(proj.starts_with(dir.join("home")), "{}", proj.display());
    std::fs::remove_dir_all(dir).ok();
}
