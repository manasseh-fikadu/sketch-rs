//! The open project: loading it into the editor, autosave and thumbnails.
//!
//! This layer has no windowing code so it can be driven by tests with a fake clock.

use crate::editor::Editor;
use crate::project::{FormatError, PageRef, Project, Store, now_ms};
use crate::render::gpu::Headless;
use std::path::Path;
use std::time::{Duration, Instant};

/// Save this long after the first unsaved change (bounded latency, even while editing).
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(1000);
/// Re-render the home-screen thumbnail at most this often while editing.
pub const THUMBNAIL_INTERVAL: Duration = Duration::from_secs(20);
pub const THUMB_SIZE: (f64, f64) = (464.0, 296.0);

pub struct Session {
    pub store: Store,
    pub project: Option<Project>,
    dirty_since: Option<Instant>,
    thumb_stale: bool,
    last_thumb: Option<Instant>,
}

impl Session {
    pub fn new(store: Store) -> Session {
        Session {
            store,
            project: None,
            dirty_since: None,
            thumb_stale: false,
            last_thumb: None,
        }
    }

    pub fn project_name(&self) -> String {
        self.project
            .as_ref()
            .map(|p| p.manifest.name.clone())
            .unwrap_or_default()
    }

    /// Open a project folder into the editor.
    pub fn open(&mut self, ed: &mut Editor, dir: &Path) -> Result<(), FormatError> {
        let p = Project::open(dir)?;
        crate::assets::load_dir(&dir.join(crate::project::ASSETS_DIR));
        let pages = p.load_pages()?;
        ed.open_pages(pages, p.manifest.active_page.as_deref());
        // Freshly loaded pages are saved by definition.
        for i in 0..ed.pages.len() {
            ed.mark_page_saved(i);
        }
        self.project = Some(p);
        self.dirty_since = None;
        self.thumb_stale = false;
        self.last_thumb = None;
        Ok(())
    }

    /// Create a new project and open it.
    pub fn create(&mut self, ed: &mut Editor, name: &str) -> Result<(), FormatError> {
        let (p, _) = self.store.create(name)?;
        self.open(ed, &p.dir)
    }

    /// Note that time passed; returns true when a save is due now.
    pub fn autosave_due(&mut self, ed: &Editor, now: Instant) -> bool {
        if self.project.is_none() || !ed.project_dirty() {
            self.dirty_since = None;
            return false;
        }
        let since = *self.dirty_since.get_or_insert(now);
        now.duration_since(since) >= AUTOSAVE_DELAY && !ed.is_busy()
    }

    /// Retry later (after a failed save) instead of spinning on an overdue deadline.
    pub fn defer(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    /// When the next autosave check should happen (for the event loop's wake-up timer).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.dirty_since.map(|t| t + AUTOSAVE_DELAY)
    }

    /// Write dirty pages and the manifest. Returns how many page files were written.
    pub fn save(&mut self, ed: &mut Editor) -> Result<usize, FormatError> {
        let Some(p) = self.project.as_mut() else {
            return Ok(0);
        };
        ed.commit_pending();
        let dirty = ed.dirty_pages();
        for &i in &dirty {
            let data = ed.page_data(i);
            // Images first, so a page never references a file that isn't there.
            crate::assets::save_to_dir(
                &p.dir.join(crate::project::ASSETS_DIR),
                crate::assets::used_by(&data.doc),
            )?;
            p.save_page(&data)?;
            ed.mark_page_saved(i);
        }
        let pages: Vec<PageRef> = ed
            .pages
            .iter()
            .map(|s| PageRef {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect();
        // Remove files of deleted pages.
        for old in &p.manifest.pages {
            if !pages.iter().any(|n| n.id == old.id) {
                p.delete_page_file(&old.id);
            }
        }
        let wrote = !dirty.is_empty() || ed.meta_dirty;
        p.manifest.pages = pages;
        p.manifest.active_page = Some(ed.active_page_id().to_string());
        if wrote {
            p.manifest.modified = now_ms();
            self.thumb_stale = true;
        }
        p.write_manifest()?;
        ed.meta_dirty = false;
        self.dirty_since = None;
        Ok(dirty.len())
    }

    /// Save the camera too (on close / leaving the project), plus everything dirty.
    pub fn save_all(&mut self, ed: &mut Editor) -> Result<(), FormatError> {
        self.save(ed)?;
        if let Some(p) = &self.project {
            // The camera isn't tracked by history; persist the active page's view.
            p.save_page(&ed.page_data(ed.active_page))?;
        }
        Ok(())
    }

    /// Whether the thumbnail should be refreshed now.
    pub fn thumbnail_due(&self, now: Instant, force: bool) -> bool {
        self.project.is_some()
            && self.thumb_stale
            && (force
                || self
                    .last_thumb
                    .is_none_or(|t| now.duration_since(t) >= THUMBNAIL_INTERVAL))
    }

    /// Render the first page into `thumbnail.png`.
    pub fn write_thumbnail(&mut self, gpu: &Headless, ed: &Editor) -> Result<(), String> {
        let Some(p) = &self.project else {
            return Ok(());
        };
        self.thumb_stale = false;
        self.last_thumb = Some(Instant::now());
        let doc = ed.page_doc(0);
        let path = p.thumbnail_path();
        match render_thumbnail(gpu, doc)? {
            Some(png) => crate::io::atomic::write(&path, &png).map_err(|e| e.to_string()),
            None => {
                let _ = std::fs::remove_file(&path);
                Ok(())
            }
        }
    }

    /// Close the project: final save; the caller then shows the home screen.
    pub fn close(&mut self, ed: &mut Editor) -> Result<(), FormatError> {
        let r = self.save_all(ed);
        self.project = None;
        self.dirty_since = None;
        r
    }
}

/// PNG bytes of a document fitted into the thumbnail box, or None when it's empty.
pub fn render_thumbnail(
    gpu: &Headless,
    doc: &crate::model::Document,
) -> Result<Option<Vec<u8>>, String> {
    let Some(b) = crate::io::svg::export_bounds(doc, &doc.root) else {
        return Ok(None);
    };
    let scale = (THUMB_SIZE.0 / b.w).min(THUMB_SIZE.1 / b.h).min(2.0);
    let img = crate::io::png::render(gpu, doc, None, scale, None)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(Some(out.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::{Node, NodeId, NodeKind};

    fn store() -> Store {
        Store::open(
            std::env::temp_dir().join(format!("sketch-rs-session-{:016x}", fastrand::u64(..))),
        )
        .unwrap()
    }

    fn add_rect(ed: &mut Editor, x: f64) -> NodeId {
        ed.add_node(
            Node::new(NodeId(0), NodeKind::Rect, Rect::new(x, 0.0, 10.0, 10.0)),
            None,
        )
    }

    #[test]
    fn autosave_after_delay_and_reload() {
        let s = store();
        let mut sess = Session::new(s.clone());
        let mut ed = Editor::default();
        sess.create(&mut ed, "Auto").unwrap();
        let t0 = Instant::now();
        assert!(!sess.autosave_due(&ed, t0));
        add_rect(&mut ed, 5.0);
        assert!(!sess.autosave_due(&ed, t0), "not before the delay");
        assert!(sess.autosave_due(&ed, t0 + AUTOSAVE_DELAY));
        assert_eq!(sess.save(&mut ed).unwrap(), 1);
        assert!(!ed.project_dirty());
        assert!(!sess.autosave_due(&ed, t0 + AUTOSAVE_DELAY * 3));

        // Reload from disk in a fresh editor.
        let dir = sess.project.as_ref().unwrap().dir.clone();
        let mut ed2 = Editor::default();
        Session::new(s.clone()).open(&mut ed2, &dir).unwrap();
        assert_eq!(ed2.doc.nodes, ed.doc.nodes);
        assert!(!ed2.project_dirty());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn pages_persist_with_names_order_and_deletions() {
        let s = store();
        let mut sess = Session::new(s.clone());
        let mut ed = Editor::default();
        sess.create(&mut ed, "Pages").unwrap();
        add_rect(&mut ed, 1.0);
        ed.add_page();
        add_rect(&mut ed, 2.0);
        add_rect(&mut ed, 3.0);
        ed.rename_page(1, "Wireframes");
        ed.add_page();
        ed.move_page(2, 0);
        sess.save(&mut ed).unwrap();
        let removed = ed.pages[0].id.clone();
        ed.delete_page(0);
        sess.save(&mut ed).unwrap();
        let p = Project::open(&sess.project.as_ref().unwrap().dir).unwrap();
        assert!(!p.page_path(&removed).exists());
        let names: Vec<_> = p.manifest.pages.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Page 1", "Wireframes"]);
        let pages = p.load_pages().unwrap();
        assert_eq!(pages[0].doc.nodes.len(), 1);
        assert_eq!(pages[1].doc.nodes.len(), 2);
        assert_eq!(
            p.manifest.active_page.as_deref(),
            Some(pages[0].id.as_str())
        );
        std::fs::remove_dir_all(&s.root).ok();
    }
}
