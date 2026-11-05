//! Turns the preview's current state into what the GPU draws.
//!
//! Each screen becomes one render layer for its scrolling content plus one per pinned layer, so
//! scrolling and sliding only move cameras. The item list is rebuilt only when the preview's
//! `generation` says something other than a camera changed.

use super::{Preview, SceneView, pinned};
use crate::geom::{Camera, Rect, V2, v2};
use crate::model::{BlendMode, Color, NodeId};
use crate::render::mesh::{Mesh, MeshBuilder};
use crate::render::scene::{self, CanvasFrame, Item, RenderLayer, SceneCache};
use crate::render::tess::Piece;
use lyon::tessellation::FillRule;
use std::collections::HashMap;
use std::sync::Arc;

/// The neutral backdrop around a frame narrower than the window.
pub const STAGE: Color = Color::rgb(0x1e, 0x1e, 0x20);
const HOTSPOT: [f32; 3] = [0.18, 0.44, 0.95];

/// GPU-side memory of the preview, kept apart from the editor's cache because the same node id
/// means different layers on different pages.
#[derive(Default)]
pub struct PreviewCache {
    pages: HashMap<String, SceneCache>,
    items: Option<(u64, Vec<i32>, Vec<Item>)>,
}

impl PreviewCache {
    pub fn clear(&mut self) {
        self.pages.clear();
        self.items = None;
    }

    /// Layers re-tessellated by the last rebuild, across all pages (for tests).
    pub fn last_misses(&self) -> usize {
        self.pages.values().map(|c| c.last_misses).sum()
    }
}

fn dim_item(layer: u16) -> Item {
    let mut mb = MeshBuilder::default();
    // Bigger than any window; the layer's scissor cuts it to the canvas.
    mb.rect(
        Rect::new(-100_000.0, -100_000.0, 200_000.0, 200_000.0),
        [0.0, 0.0, 0.0, 1.0],
    );
    let mut mesh = Mesh::default();
    for b in mb.finish() {
        mesh.append(&b.mesh);
    }
    Item {
        id: NodeId(u64::MAX),
        pieces: Arc::new(vec![Piece { image: None, mesh }]),
        clip: None,
        round_clip: 0,
        blend: BlendMode::Normal,
        layer,
    }
}

fn build_items(p: &Preview, cache: &mut PreviewCache, views: &[SceneView]) -> Vec<Item> {
    let mut out = Vec::new();
    let mut layer: u16 = 0;
    for v in views {
        let Some(doc) = p.doc(&v.page) else { continue };
        if v.dim.is_some() {
            out.push(dim_item(layer));
            layer += 1;
        }
        let sc = cache.pages.entry(v.page.clone()).or_default();
        let items = sc.items(doc, Some(&[v.frame]), v.zoom, None);
        let pins = pinned(doc, v.frame);
        let mut group: HashMap<NodeId, usize> = HashMap::new();
        for (i, g) in pins.iter().enumerate() {
            for id in doc.subtree(g.id) {
                group.insert(id, i + 1);
            }
        }
        // Pinned layers paint over everything that scrolls under them.
        let mut buckets: Vec<Vec<Item>> = vec![Vec::new(); pins.len() + 1];
        for it in items {
            let b = group.get(&it.id).copied().unwrap_or(0);
            buckets[b].push(it);
        }
        for bucket in buckets {
            out.extend(bucket.into_iter().map(|it| Item { layer, ..it }));
            layer += 1;
        }
    }
    out
}

fn camera_rect(canvas: Rect, offset: V2, r: Rect) -> Rect {
    canvas
        .intersection(&r.translate(offset))
        .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
}

fn build_layers(p: &Preview, views: &[SceneView], canvas: Rect, offset: V2) -> Vec<RenderLayer> {
    let mut layers = Vec::new();
    for v in views {
        let Some(doc) = p.doc(&v.page) else { continue };
        if let Some(d) = v.dim {
            layers.push(RenderLayer {
                camera: Camera::default(),
                opacity: d,
                clip: canvas,
            });
        }
        layers.push(RenderLayer {
            camera: v.camera(v.scroll),
            opacity: v.alpha,
            clip: camera_rect(canvas, offset, v.rect_at(v.scroll)),
        });
        for g in pinned(doc, v.frame) {
            let s = g.scroll(v.scroll);
            layers.push(RenderLayer {
                camera: v.camera(s),
                opacity: v.alpha,
                clip: camera_rect(canvas, offset, v.rect_at(s)),
            });
        }
    }
    layers
}

/// Outlines on everything clickable.
fn hotspots(mb: &mut MeshBuilder, p: &Preview, views: &[SceneView], offset: V2) {
    for v in views.iter().filter(|v| v.live) {
        let Some(doc) = p.doc(&v.page) else { continue };
        let pins = pinned(doc, v.frame);
        for id in doc.subtree(v.frame) {
            let clickable = doc
                .get(id)
                .and_then(|n| n.proto.as_ref())
                .is_some_and(|pr| pr.has_click());
            if !clickable || !doc.is_effectively_visible(id) {
                continue;
            }
            let Some(wr) = doc.world_rect(id) else {
                continue;
            };
            let pin = pins
                .iter()
                .find(|g| g.id == id || doc.is_ancestor(g.id, id));
            let s = pin.map_or(v.scroll, |g| g.scroll(v.scroll));
            let r = v.camera(s).world_rect_to_screen(&wr).translate(offset);
            mb.rect(r, [HOTSPOT[0], HOTSPOT[1], HOTSPOT[2], 0.16]);
            mb.rect_outline(r, 1.5, [HOTSPOT[0], HOTSPOT[1], HOTSPOT[2], 0.9]);
        }
    }
}

/// The page's scroll position: a thin bar that shows while scrolling, as on macOS.
fn scrollbar(mb: &mut MeshBuilder, p: &Preview, v: &SceneView, canvas: Rect) {
    let alpha = p.scrollbar_alpha();
    let vp = p.viewport();
    let content = v.frame_rect.h * v.zoom;
    if alpha <= 0.0 || content <= vp.y {
        return;
    }
    let h = (vp.y * vp.y / content).max(32.0);
    let max = content - vp.y;
    let y = (v.scroll.y * v.zoom / max).clamp(0.0, 1.0) * (vp.y - h);
    let r = Rect::new(canvas.right() - 9.0, canvas.y + y + 2.0, 6.0, h - 4.0);
    mb.fill_path(
        &MeshBuilder::rounded_rect_path(r.inflate(1.0), 4.0),
        [1.0, 1.0, 1.0, (alpha * 0.35) as f32],
        FillRule::NonZero,
    );
    mb.fill_path(
        &MeshBuilder::rounded_rect_path(r, 3.0),
        [0.0, 0.0, 0.0, (alpha * 0.5) as f32],
        FillRule::NonZero,
    );
}

/// The frame to draw for the current preview state.
pub fn frame(p: &Preview, cache: &mut PreviewCache, offset: V2) -> CanvasFrame {
    let views = p.views();
    let vp = p.viewport();
    let canvas = Rect::new(offset.x, offset.y, vp.x, vp.y);

    let mut under = MeshBuilder::default();
    under.set_clip(Some(canvas));
    under.rect(canvas, scene::rgba(STAGE));
    // A page filling the window's width carries its background below its end, like `body`.
    if let Some(v) = views.iter().find(|v| v.live && v.overlay.is_none())
        && v.frame_rect.w * v.zoom >= vp.x - 1.0
        && let Some(c) = p
            .doc(&v.page)
            .and_then(|d| d.get(v.frame))
            .and_then(|n| n.style.fill())
    {
        under.rect(canvas, scene::rgba(c));
    }

    let buckets: Vec<i32> = views.iter().map(|v| scene::zoom_bucket(v.zoom)).collect();
    let cached = cache
        .items
        .as_ref()
        .filter(|(g, b, _)| *g == p.generation && *b == buckets)
        .map(|(_, _, items)| items.clone());
    let items = cached.unwrap_or_else(|| {
        let items = build_items(p, cache, &views);
        cache.items = Some((p.generation, buckets, items.clone()));
        items
    });
    let layers = build_layers(p, &views, canvas, offset);

    let mut over = MeshBuilder::default();
    over.set_clip(Some(canvas));
    if p.hints_visible() {
        hotspots(&mut over, p, &views, offset);
    }
    if let Some(v) = views.iter().find(|v| v.live && v.overlay.is_none()) {
        scrollbar(&mut over, p, v, canvas);
    }

    let camera = layers.first().map_or(
        Camera {
            pan: v2(0.0, 0.0),
            zoom: 1.0,
        },
        |l| l.camera,
    );
    CanvasFrame {
        under: under.finish(),
        items,
        over: over.finish(),
        camera,
        offset,
        viewport: canvas,
        layers,
    }
}
