//! Fidelity against a real browser: the same HTML goes through `write_html` and
//! `get_screenshot`, and the result is compared with a committed Chromium render
//! (`scripts/fidelity-refs.sh`). A side-by-side of each page lands in `.artifacts/fidelity/`.

mod common;

use image::{Rgba, RgbaImage};
use serde_json::{Value, json};
use sketch_rs::mcp::{Out, exec};
use sketch_rs::project::Store;
use std::path::PathBuf;

/// A pixel differs when any channel is off by more than this (out of 255).
const THRESHOLD: i32 = 32;

/// Percent of pixels allowed to differ from Chromium. Only ever lower these: a higher score
/// means the renderer or layout regressed. They hold a little headroom, since GPUs anti-alias
/// differently. Sketch scored 4.7% and 57.9% before layout moved to Taffy.
const PAGES: &[(&str, f64)] = &[("billing", 2.5), ("pricing", 5.0)];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn host() -> exec::SessionHost {
    let dir = std::env::temp_dir().join(format!("sketch-rs-fidelity-{:016x}", fastrand::u64(..)));
    let mut h = exec::SessionHost::new(Store::open(dir).unwrap());
    h.session.create(&mut h.ed, "Fidelity").unwrap();
    h
}

fn call(h: &mut exec::SessionHost, name: &str, args: Value) -> Value {
    match exec::call(h, name, &args)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .remove(0)
    {
        Out::Text(t) => serde_json::from_str(&t).unwrap_or(Value::String(t)),
        Out::Image { .. } => panic!("{name} returned an image"),
    }
}

/// Chromium paints white below the page; the page itself stops at the last non-white row.
fn crop_to_page(img: RgbaImage) -> RgbaImage {
    let last = (0..img.height()).rev().find(|y| {
        (0..img.width())
            .step_by(7)
            .any(|x| img.get_pixel(x, *y).0[..3] != [255, 255, 255])
    });
    let h = last.map_or(img.height(), |y| y + 1);
    image::imageops::crop_imm(&img, 0, 0, img.width(), h).to_image()
}

/// Percent of pixels that differ. A height mismatch counts as different pixels.
fn score(a: &RgbaImage, b: &RgbaImage) -> f64 {
    let (w, h) = (a.width().max(b.width()), a.height().max(b.height()));
    let missing = Rgba([255, 0, 255, 255]);
    let mut bad = 0u64;
    for y in 0..h {
        for x in 0..w {
            let p = a.get_pixel_checked(x, y).unwrap_or(&missing).0;
            let q = b.get_pixel_checked(x, y).unwrap_or(&missing).0;
            if (0..3).any(|i| (p[i] as i32 - q[i] as i32).abs() > THRESHOLD) {
                bad += 1;
            }
        }
    }
    100.0 * bad as f64 / (w as u64 * h as u64) as f64
}

fn side_by_side(a: &RgbaImage, b: &RgbaImage) -> RgbaImage {
    let gap = 24;
    let mut out = RgbaImage::from_pixel(
        a.width() + gap + b.width(),
        a.height().max(b.height()),
        Rgba([32, 32, 36, 255]),
    );
    image::imageops::overlay(&mut out, a, 0, 0);
    image::imageops::overlay(&mut out, b, (a.width() + gap) as i64, 0);
    out
}

#[test]
fn pages_match_chromium() {
    if common::gpu().is_none() {
        return;
    }
    let artifacts = root().join(".artifacts/fidelity");
    std::fs::create_dir_all(&artifacts).unwrap();
    let avatar = root().join("tests/fidelity/avatar.png");
    let mut failed = Vec::new();
    for (name, budget) in PAGES {
        let html =
            std::fs::read_to_string(root().join(format!("tests/fidelity/pages/{name}.html")))
                .unwrap()
                .replace("AVATAR", avatar.to_str().unwrap());
        let mut h = host();
        let ab = call(
            &mut h,
            "create_artboard",
            json!({"name": name, "styles": {"width": "1100px", "height": "fit-content"}}),
        );
        let wrote = call(
            &mut h,
            "write_html",
            json!({"targetNodeId": ab["id"], "mode": "insert-children", "html": html}),
        );
        let outs = exec::call(
            &mut h,
            "get_screenshot",
            &json!({"nodeId": ab["id"], "scale": 1}),
        )
        .unwrap();
        let Out::Image { data, .. } = &outs[0] else {
            panic!("get_screenshot returned no image");
        };
        let got = image::load_from_memory(data).unwrap().to_rgba8();
        let reference = image::open(root().join(format!("tests/fidelity/refs/{name}.png")))
            .unwrap()
            .to_rgba8();
        let reference = crop_to_page(reference);
        let pct = score(&reference, &got);
        side_by_side(&reference, &got)
            .save(artifacts.join(format!("{name}.png")))
            .unwrap();
        eprintln!(
            "{name}: {pct:.1}% differ (budget {budget}%), chromium {:?} sketch {:?}, ignored {}",
            reference.dimensions(),
            got.dimensions(),
            wrote.get("ignoredStyles").unwrap_or(&json!([])),
        );
        if pct > *budget {
            failed.push(format!("{name}: {pct:.1}% > {budget}%"));
        }
    }
    assert!(failed.is_empty(), "fidelity regressed: {failed:?}");
}
