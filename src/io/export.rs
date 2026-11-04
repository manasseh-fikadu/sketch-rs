//! Export of selections, frames and pages to PNG, SVG and PDF (shared by the app, the CLI and
//! tests).

use crate::model::{Color, Document, NodeId};
use crate::render::gpu::Headless;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Png,
    Svg,
    Pdf,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Pdf => "pdf",
        }
    }
    pub const ALL: [Format; 3] = [Format::Png, Format::Svg, Format::Pdf];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// The selected layers (or the whole page when nothing is selected).
    #[default]
    Selection,
    /// Each top-level frame (or each selected frame) separately.
    Frames,
    /// Everything on the current page.
    Page,
    /// Every page of the project.
    AllPages,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Selection => "Selection",
            Scope::Frames => "Each frame",
            Scope::Page => "Page",
            Scope::AllPages => "All pages",
        }
    }
    pub const ALL: [Scope; 4] = [
        Scope::Selection,
        Scope::Frames,
        Scope::Page,
        Scope::AllPages,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub format: Format,
    pub scope: Scope,
    /// PNG pixel scale.
    pub scale: f64,
    pub transparent: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            format: Format::Png,
            scope: Scope::Selection,
            scale: 2.0,
            transparent: false,
        }
    }
}

impl Spec {
    /// One output file, or a folder of files?
    pub fn single_file(&self, items: usize) -> bool {
        self.format == Format::Pdf || items <= 1
    }
}

/// One thing to export: a document and the roots to draw, with a file-friendly name.
pub struct Item<'a> {
    pub name: String,
    pub doc: &'a Document,
    pub roots: Vec<NodeId>,
}

/// Resolve what a spec exports, given the pages (`(name, doc)`), the active page index and
/// the selection on it.
pub fn items<'a>(
    spec: &Spec,
    pages: &[(&'a str, &'a Document)],
    active: usize,
    selection: &[NodeId],
) -> Vec<Item<'a>> {
    let (pname, doc) = pages[active];
    let frames_of = |doc: &'a Document, among: &[NodeId]| -> Vec<Item<'a>> {
        among
            .iter()
            .filter(|id| doc.get(**id).is_some_and(|n| n.is_frame() && n.visible))
            .map(|id| Item {
                name: doc.get(*id).unwrap().name.clone(),
                doc,
                roots: vec![*id],
            })
            .collect()
    };
    match spec.scope {
        Scope::Selection if !selection.is_empty() => vec![Item {
            name: if selection.len() == 1 {
                doc.get(selection[0])
                    .map_or(pname.to_string(), |n| n.name.clone())
            } else {
                pname.to_string()
            },
            doc,
            roots: selection.to_vec(),
        }],
        Scope::Selection | Scope::Page => vec![Item {
            name: pname.to_string(),
            doc,
            roots: doc.root.clone(),
        }],
        Scope::Frames => {
            let sel_frames = frames_of(doc, selection);
            if !sel_frames.is_empty() {
                sel_frames
            } else {
                frames_of(doc, &doc.root)
            }
        }
        Scope::AllPages => pages
            .iter()
            .map(|(n, d)| Item {
                name: n.to_string(),
                doc: d,
                roots: d.root.clone(),
            })
            .collect(),
    }
}

fn safe(name: &str) -> String {
    let s = crate::project::folder_name(name);
    if s.is_empty() { "export".into() } else { s }
}

/// Write the export. `out` is a file (single output) or a folder (several). Returns the
/// files written.
pub fn write(
    spec: &Spec,
    items: &[Item],
    out: &Path,
    gpu: Option<&Headless>,
) -> Result<Vec<PathBuf>, String> {
    let bg = (!spec.transparent).then_some(Color::WHITE);
    if items.is_empty() {
        return Err("nothing to export".into());
    }
    if spec.format == Format::Pdf {
        let mut pages = Vec::new();
        for it in items {
            let frames = spec.scope == Scope::Frames;
            pages.extend(crate::io::pdf::pages_for(
                it.doc,
                Some(&it.roots),
                frames,
                bg,
            ));
        }
        if pages.is_empty() {
            return Err("nothing to export".into());
        }
        let bytes = crate::io::pdf::export(&pages);
        crate::io::atomic::write(out, &bytes).map_err(|e| e.to_string())?;
        return Ok(vec![out.to_path_buf()]);
    }
    let single = spec.single_file(items.len());
    let mut written = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for it in items {
        let path = if single {
            out.to_path_buf()
        } else {
            let mut name = safe(&it.name);
            let mut i = 2;
            while used.contains(&name) {
                name = format!("{} {i}", safe(&it.name));
                i += 1;
            }
            used.push(name.clone());
            std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
            out.join(format!("{name}.{}", spec.format.ext()))
        };
        match spec.format {
            Format::Svg => {
                let s = crate::io::svg::export(it.doc, Some(&it.roots), bg)
                    .ok_or("nothing to export")?;
                crate::io::atomic::write(&path, s.as_bytes()).map_err(|e| e.to_string())?;
            }
            Format::Png => {
                let owned;
                let g = match gpu {
                    Some(g) => g,
                    None => {
                        owned = Headless::new()?;
                        &owned
                    }
                };
                let img = crate::io::png::render(g, it.doc, Some(&it.roots), spec.scale, bg)?;
                let mut buf = std::io::Cursor::new(Vec::new());
                img.write_to(&mut buf, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                crate::io::atomic::write(&path, &buf.into_inner()).map_err(|e| e.to_string())?;
            }
            Format::Pdf => unreachable!(),
        }
        written.push(path);
    }
    Ok(written)
}
