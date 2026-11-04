//! Triangle meshes built with lyon, in screen space (logical pixels).

use crate::geom::{Rect, V2};
use bytemuck::{Pod, Zeroable};
use lyon::math::point;
use lyon::path::{Path, Winding};
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineCap, LineJoin,
    StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    /// Straight-alpha color; for gradient and image paints only alpha (opacity) is used.
    pub color: [f32; 4],
    /// Paint coordinates: gradient space or image texture coordinates.
    pub uv: [f32; 2],
    /// Paint kind in the low byte (see `PAINT_*`), gradient atlas row above it.
    pub kind: u32,
    pub _pad: u32,
}

pub const PAINT_SOLID: u32 = 0;
pub const PAINT_LINEAR: u32 = 1;
pub const PAINT_RADIAL: u32 = 2;
pub const PAINT_IMAGE: u32 = 3;
pub const PAINT_IMAGE_TILE: u32 = 4;

impl Vertex {
    pub fn solid(pos: [f32; 2], color: [f32; 4]) -> Vertex {
        Vertex {
            pos,
            color,
            uv: [0.0; 2],
            kind: PAINT_SOLID,
            _pad: 0,
        }
    }
}

/// Screen-space tessellation tolerance in pixels.
pub const TOLERANCE: f32 = 0.15;

#[derive(Default, Clone, Debug)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// A run of triangles sharing one clip rectangle (logical px, None = whole canvas).
pub struct Batch {
    pub clip: Option<Rect>,
    pub mesh: Mesh,
}

impl Mesh {
    pub fn append(&mut self, o: &Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&o.vertices);
        self.indices.extend(o.indices.iter().map(|i| i + base));
    }
}

pub struct MeshBuilder {
    pub batches: Vec<Batch>,
    fill: FillTessellator,
    stroke: StrokeTessellator,
}

impl Default for MeshBuilder {
    fn default() -> Self {
        MeshBuilder {
            batches: vec![Batch {
                clip: None,
                mesh: Mesh::default(),
            }],
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
        }
    }
}

pub fn p(v: V2) -> lyon::math::Point {
    point(v.x as f32, v.y as f32)
}

impl MeshBuilder {
    pub fn set_clip(&mut self, clip: Option<Rect>) {
        let cur = self.batches.last().unwrap();
        if cur.clip == clip {
            return;
        }
        if cur.mesh.is_empty() {
            self.batches.last_mut().unwrap().clip = clip;
        } else {
            self.batches.push(Batch {
                clip,
                mesh: Mesh::default(),
            });
        }
    }

    fn buffers(&mut self) -> &mut Mesh {
        &mut self.batches.last_mut().unwrap().mesh
    }

    pub fn fill_path(&mut self, path: &Path, color: [f32; 4], rule: FillRule) {
        if color[3] <= 0.0 {
            return;
        }
        let mut vb: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let _ = self.fill.tessellate_path(
            path,
            &FillOptions::tolerance(TOLERANCE).with_fill_rule(rule),
            &mut BuffersBuilder::new(&mut vb, |v: FillVertex| {
                Vertex::solid(v.position().to_array(), color)
            }),
        );
        self.append(vb);
    }

    pub fn stroke_path(&mut self, path: &Path, width: f32, color: [f32; 4], cap: LineCap) {
        if color[3] <= 0.0 || width <= 0.0 {
            return;
        }
        let mut vb: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let _ = self.stroke.tessellate_path(
            path,
            &StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(width)
                .with_line_cap(cap)
                .with_line_join(LineJoin::Round),
            &mut BuffersBuilder::new(&mut vb, |v: StrokeVertex| {
                Vertex::solid(v.position().to_array(), color)
            }),
        );
        self.append(vb);
    }

    fn append(&mut self, vb: VertexBuffers<Vertex, u32>) {
        let m = self.buffers();
        let base = m.vertices.len() as u32;
        m.vertices.extend(vb.vertices);
        m.indices.extend(vb.indices.into_iter().map(|i| i + base));
    }

    /// Append pre-transformed triangles (e.g. cached glyphs).
    pub fn triangles(
        &mut self,
        verts: impl Iterator<Item = [f32; 2]>,
        indices: &[u32],
        color: [f32; 4],
    ) {
        let m = self.buffers();
        let base = m.vertices.len() as u32;
        m.vertices
            .extend(verts.map(|pos| Vertex::solid(pos, color)));
        m.indices.extend(indices.iter().map(|i| i + base));
    }

    pub fn rect(&mut self, r: Rect, color: [f32; 4]) {
        if color[3] <= 0.0 {
            return;
        }
        let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32);
        let m = self.buffers();
        let base = m.vertices.len() as u32;
        for pos in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            m.vertices.push(Vertex::solid(pos, color));
        }
        m.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Axis-aligned rectangle outline drawn inside-out with crisp 1px-ish lines.
    pub fn rect_outline(&mut self, r: Rect, width: f64, color: [f32; 4]) {
        let w = width;
        self.rect(Rect::new(r.x - w * 0.5, r.y - w * 0.5, r.w + w, w), color);
        self.rect(
            Rect::new(r.x - w * 0.5, r.bottom() - w * 0.5, r.w + w, w),
            color,
        );
        self.rect(
            Rect::new(r.x - w * 0.5, r.y + w * 0.5, w, (r.h - w).max(0.0)),
            color,
        );
        self.rect(
            Rect::new(r.right() - w * 0.5, r.y + w * 0.5, w, (r.h - w).max(0.0)),
            color,
        );
    }

    pub fn rounded_rect_path(r: Rect, radius: f64) -> Path {
        let mut b = Path::builder();
        let rad = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0) as f32;
        b.add_rounded_rectangle(
            &lyon::math::Box2D::new(p(r.min()), p(r.max())),
            &lyon::path::builder::BorderRadii::new(rad),
            Winding::Positive,
        );
        b.build()
    }

    pub fn ellipse_path(r: Rect) -> Path {
        let mut b = Path::builder();
        b.add_ellipse(
            p(r.center()),
            lyon::math::vector((r.w * 0.5) as f32, (r.h * 0.5) as f32),
            lyon::math::Angle::radians(0.0),
            Winding::Positive,
        );
        b.build()
    }

    pub fn polygon_path(pts: &[V2], closed: bool) -> Path {
        let mut b = Path::builder();
        if let Some(first) = pts.first() {
            b.begin(p(*first));
            for q in &pts[1..] {
                b.line_to(p(*q));
            }
            b.end(closed);
        }
        b.build()
    }

    pub fn circle(&mut self, c: V2, r: f64, color: [f32; 4]) {
        let path = Self::ellipse_path(Rect::new(c.x - r, c.y - r, r * 2.0, r * 2.0));
        self.fill_path(&path, color, FillRule::NonZero);
    }

    pub fn line(&mut self, a: V2, b: V2, width: f64, color: [f32; 4]) {
        self.stroke_path(
            &Self::polygon_path(&[a, b], false),
            width as f32,
            color,
            LineCap::Butt,
        );
    }

    pub fn finish(mut self) -> Vec<Batch> {
        self.batches.retain(|b| !b.mesh.is_empty());
        self.batches
    }
}
