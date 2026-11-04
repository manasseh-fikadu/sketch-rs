//! Local-first projects.
//!
//! Every project is a folder under the store root (`~/Documents/sketch-rs/` by default):
//!
//! ```text
//! <root>/<Project name>/
//!     project.json        manifest: id, name, timestamps, ordered page list
//!     pages/<page-id>.json one infinite canvas per page
//!     assets/<hash>.<ext> images used by image fills
//!     thumbnail.png       preview for the home screen (written by the app)
//! <root>/.trash/<Project name>~<timestamp>/   deleted projects, restorable
//! ```
//!
//! All writes go through [`crate::io::atomic`], so a crash never leaves a torn file.

pub mod format;

use crate::io::atomic;
pub use format::{FormatError, Manifest, PageData, PageRef, new_id, now_ms};
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "project.json";
pub const PAGES_DIR: &str = "pages";
pub const THUMBNAIL: &str = "thumbnail.png";
pub const ASSETS_DIR: &str = "assets";
pub const TRASH_DIR: &str = ".trash";
const TRASH_INFO: &str = "trashinfo.json";

/// The folder holding all projects.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

/// A project as listed on the home screen.
#[derive(Debug, Clone)]
pub struct Entry {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub thumbnail: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TrashEntry {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// Unix ms when it was deleted.
    pub deleted: u64,
    /// Folder name it had before deletion.
    pub original: String,
}

/// An open project on disk.
#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

/// Move the projects folder from `old` to `new` unless `new` already exists. If the move fails,
/// keep using `old` so the projects don't disappear from the home screen.
fn adopt_old_root(old: PathBuf, new: PathBuf) -> PathBuf {
    if new.exists() || !old.is_dir() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(e) => {
            log::warn!("could not move {} to {}: {e}", old.display(), new.display());
            old
        }
    }
}

/// File-system safe folder name for a project name.
pub fn folder_name(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let s = s.trim_start_matches('.').trim().to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

impl Store {
    /// `$SKETCH_RS_HOME` (or `$PAPER_RS_HOME`), else `~/Documents/sketch-rs`. A
    /// `~/Documents/paper-rs` folder from before the rename is moved there the first time.
    pub fn default_root() -> PathBuf {
        if let Some(p) = crate::env_var("HOME") {
            return PathBuf::from(p);
        }
        let docs = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        adopt_old_root(docs.join("paper-rs"), docs.join("sketch-rs"))
    }

    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Store> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Store { root })
    }

    fn trash_root(&self) -> PathBuf {
        self.root.join(TRASH_DIR)
    }

    /// Projects, most recently modified first. Unreadable folders are skipped.
    pub fn list(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| {
                let dir = e.path();
                let manifest = read_manifest(&dir).ok()?;
                let thumb = dir.join(THUMBNAIL);
                Some(Entry {
                    thumbnail: thumb.is_file().then_some(thumb),
                    dir,
                    manifest,
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.manifest
                .modified
                .cmp(&a.manifest.modified)
                .then_with(|| a.manifest.name.cmp(&b.manifest.name))
        });
        out
    }

    /// A folder path for `name` that doesn't exist yet ("Name", "Name 2", ...).
    fn unique_dir(&self, parent: &Path, name: &str) -> PathBuf {
        let base = folder_name(name);
        let mut p = parent.join(&base);
        let mut i = 2;
        while p.exists() {
            p = parent.join(format!("{base} {i}"));
            i += 1;
        }
        p
    }

    /// A display name not used by any listed project ("Untitled", "Untitled 2", ...).
    pub fn unique_name(&self, name: &str) -> String {
        let taken: Vec<String> = self.list().into_iter().map(|e| e.manifest.name).collect();
        let mut n = name.to_string();
        let mut i = 2;
        while taken.contains(&n) || self.root.join(folder_name(&n)).exists() {
            n = format!("{name} {i}");
            i += 1;
        }
        n
    }

    /// Create an empty project with one blank page.
    pub fn create(&self, name: &str) -> Result<(Project, PageData), FormatError> {
        let name = self.unique_name(name);
        let dir = self.unique_dir(&self.root, &name);
        std::fs::create_dir_all(dir.join(PAGES_DIR))?;
        let manifest = Manifest::new(&name);
        let page = PageData {
            id: manifest.pages[0].id.clone(),
            name: manifest.pages[0].name.clone(),
            doc: crate::model::Document {
                name: manifest.pages[0].name.clone(),
                ..Default::default()
            },
            camera: None,
        };
        let p = Project { dir, manifest };
        p.save_page(&page)?;
        p.write_manifest()?;
        Ok((p, page))
    }

    /// Create a project from existing pages (imports, legacy files).
    pub fn create_with_pages(
        &self,
        name: &str,
        pages: Vec<PageData>,
    ) -> Result<Project, FormatError> {
        let name = self.unique_name(name);
        let dir = self.unique_dir(&self.root, &name);
        std::fs::create_dir_all(dir.join(PAGES_DIR))?;
        let mut manifest = Manifest::new(&name);
        manifest.pages = pages
            .iter()
            .map(|p| PageRef {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect();
        manifest.active_page = manifest.pages.first().map(|p| p.id.clone());
        let p = Project { dir, manifest };
        for page in &pages {
            crate::assets::save_to_dir(&p.dir.join(ASSETS_DIR), crate::assets::used_by(&page.doc))?;
            p.save_page(page)?;
        }
        p.write_manifest()?;
        Ok(p)
    }

    /// Rename a project: updates the manifest and renames the folder to match.
    pub fn rename(&self, dir: &Path, new_name: &str) -> Result<PathBuf, FormatError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(FormatError::Invalid("empty name".into()));
        }
        let mut p = Project::open(dir)?;
        p.manifest.name = new_name.to_string();
        p.manifest.modified = now_ms();
        p.write_manifest()?;
        let wanted = self.root.join(folder_name(new_name));
        if wanted == dir {
            return Ok(dir.to_path_buf());
        }
        let target = if wanted.exists() {
            self.unique_dir(&self.root, new_name)
        } else {
            wanted
        };
        std::fs::rename(dir, &target)?;
        Ok(target)
    }

    /// Copy a project (all pages) under a fresh id and "Name copy".
    pub fn duplicate(&self, dir: &Path) -> Result<PathBuf, FormatError> {
        let src = Project::open(dir)?;
        let name = self.unique_name(&format!("{} copy", src.manifest.name));
        let target = self.unique_dir(&self.root, &name);
        copy_dir(dir, &target)?;
        let mut p = Project::open(&target)?;
        p.manifest.id = new_id();
        p.manifest.name = name;
        p.manifest.created = now_ms();
        p.manifest.modified = p.manifest.created;
        p.write_manifest()?;
        Ok(target)
    }

    /// Move a project to the trash folder. Returns its path in the trash.
    pub fn trash(&self, dir: &Path) -> Result<PathBuf, FormatError> {
        let original = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        std::fs::create_dir_all(self.trash_root())?;
        let deleted = now_ms();
        let target = self
            .trash_root()
            .join(format!("{}~{deleted}", folder_name(&original)));
        std::fs::rename(dir, &target)?;
        let info = serde_json::json!({ "deleted": deleted, "original": original });
        atomic::write(&target.join(TRASH_INFO), info.to_string().as_bytes())?;
        Ok(target)
    }

    pub fn list_trash(&self) -> Vec<TrashEntry> {
        let mut out: Vec<TrashEntry> = std::fs::read_dir(self.trash_root())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let dir = e.path();
                let manifest = read_manifest(&dir).ok()?;
                let info: serde_json::Value = std::fs::read_to_string(dir.join(TRASH_INFO))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Some(TrashEntry {
                    deleted: info["deleted"].as_u64().unwrap_or(0),
                    original: info["original"]
                        .as_str()
                        .unwrap_or(&manifest.name)
                        .to_string(),
                    dir,
                    manifest,
                })
            })
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.deleted));
        out
    }

    /// Put a trashed project back (under a unique folder name if the old one is taken).
    pub fn restore(&self, trash_dir: &Path) -> Result<PathBuf, FormatError> {
        let entry = self
            .list_trash()
            .into_iter()
            .find(|e| e.dir == trash_dir)
            .ok_or_else(|| FormatError::Invalid("not in trash".into()))?;
        let target = self.unique_dir(&self.root, &entry.original);
        std::fs::rename(trash_dir, &target)?;
        let _ = std::fs::remove_file(target.join(TRASH_INFO));
        Ok(target)
    }

    pub fn delete_forever(&self, trash_dir: &Path) -> std::io::Result<()> {
        if trash_dir.parent() != Some(self.trash_root().as_path()) {
            return Err(std::io::Error::other(
                "refusing to delete outside the trash",
            ));
        }
        std::fs::remove_dir_all(trash_dir)
    }

    pub fn empty_trash(&self) -> std::io::Result<()> {
        for e in self.list_trash() {
            self.delete_forever(&e.dir)?;
        }
        Ok(())
    }
}

fn read_manifest(dir: &Path) -> Result<Manifest, FormatError> {
    Manifest::from_json(&std::fs::read_to_string(dir.join(MANIFEST))?)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

impl Project {
    pub fn open(dir: &Path) -> Result<Project, FormatError> {
        let manifest = read_manifest(dir)?;
        atomic::clean_stale_temps(dir);
        atomic::clean_stale_temps(&dir.join(PAGES_DIR));
        Ok(Project {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    pub fn page_path(&self, id: &str) -> PathBuf {
        self.dir.join(PAGES_DIR).join(format!("{id}.json"))
    }

    pub fn load_page(&self, id: &str) -> Result<PageData, FormatError> {
        let mut p = format::page_from_json(&std::fs::read_to_string(self.page_path(id))?)?;
        if let Some(r) = self.manifest.pages.iter().find(|r| r.id == id) {
            // The manifest owns page names (renames touch only the manifest).
            p.name = r.name.clone();
            p.doc.name = r.name.clone();
        }
        Ok(p)
    }

    /// Load every page listed in the manifest. A page whose file is missing comes back empty
    /// (e.g. after a crash between creating it and its first save).
    pub fn load_pages(&self) -> Result<Vec<PageData>, FormatError> {
        self.manifest
            .pages
            .iter()
            .map(|r| match self.load_page(&r.id) {
                Err(FormatError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(PageData {
                        id: r.id.clone(),
                        name: r.name.clone(),
                        doc: crate::model::Document {
                            name: r.name.clone(),
                            ..Default::default()
                        },
                        camera: None,
                    })
                }
                other => other,
            })
            .collect()
    }

    pub fn save_page(&self, page: &PageData) -> Result<(), FormatError> {
        std::fs::create_dir_all(self.dir.join(PAGES_DIR))?;
        atomic::write(
            &self.page_path(&page.id),
            format::page_to_json(page).as_bytes(),
        )?;
        Ok(())
    }

    pub fn delete_page_file(&self, id: &str) {
        let _ = std::fs::remove_file(self.page_path(id));
    }

    pub fn write_manifest(&self) -> Result<(), FormatError> {
        atomic::write(&self.dir.join(MANIFEST), self.manifest.to_json().as_bytes())?;
        Ok(())
    }

    pub fn thumbnail_path(&self) -> PathBuf {
        self.dir.join(THUMBNAIL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_store() -> Store {
        let root = std::env::temp_dir().join(format!("sketch-rs-store-{:016x}", fastrand::u64(..)));
        Store::open(root).unwrap()
    }

    #[test]
    fn create_list_rename_duplicate_trash_restore() {
        let s = temp_store();
        let (a, page) = s.create("Alpha").unwrap();
        assert!(a.dir.join(MANIFEST).is_file());
        assert!(a.page_path(&page.id).is_file());
        std::thread::sleep(std::time::Duration::from_millis(5));
        let (b, _) = s.create("Alpha").unwrap();
        assert_eq!(b.manifest.name, "Alpha 2");
        let names: Vec<String> = s.list().into_iter().map(|e| e.manifest.name).collect();
        assert_eq!(names, ["Alpha 2", "Alpha"], "most recent first");

        let renamed = s.rename(&a.dir, "Beta: plan").unwrap();
        assert_eq!(renamed.file_name().unwrap(), "Beta- plan");
        assert_eq!(Project::open(&renamed).unwrap().manifest.name, "Beta: plan");

        let dup = s.duplicate(&renamed).unwrap();
        let d = Project::open(&dup).unwrap();
        assert_eq!(d.manifest.name, "Beta: plan copy");
        assert_ne!(d.manifest.id, Project::open(&renamed).unwrap().manifest.id);
        assert_eq!(d.load_pages().unwrap().len(), 1);

        let t = s.trash(&renamed).unwrap();
        assert!(!renamed.exists());
        assert_eq!(s.list().len(), 2);
        let trash = s.list_trash();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].manifest.name, "Beta: plan");
        let back = s.restore(&t).unwrap();
        assert_eq!(back, renamed);
        assert_eq!(s.list().len(), 3);

        s.trash(&dup).unwrap();
        s.empty_trash().unwrap();
        assert!(s.list_trash().is_empty());
        assert!(s.delete_forever(&s.root.join("Alpha")).is_err());
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn pages_roundtrip_and_missing_page_is_empty() {
        let s = temp_store();
        let (mut p, mut page) = s.create("P").unwrap();
        page.doc.name = "Page 1".into();
        let id = page.doc.alloc_id();
        let n = crate::model::Node::new(
            id,
            crate::model::NodeKind::Rect,
            crate::geom::Rect::new(1.0, 2.0, 3.0, 4.0),
        );
        page.doc.nodes.insert(id, n);
        page.doc.root.push(id);
        p.save_page(&page).unwrap();
        p.manifest.pages.push(PageRef {
            id: "ghost".into(),
            name: "Ghost".into(),
        });
        p.write_manifest().unwrap();
        let p = Project::open(&p.dir).unwrap();
        let pages = p.load_pages().unwrap();
        assert_eq!(pages[0], page);
        assert!(pages[1].doc.nodes.is_empty());
        assert_eq!(pages[1].name, "Ghost");
        std::fs::remove_dir_all(&s.root).ok();
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("a/b\\c"), "a-b-c");
        assert_eq!(folder_name("  ..hidden "), "hidden");
        assert_eq!(folder_name(""), "Untitled");
    }

    #[test]
    fn projects_folder_from_before_the_rename_moves_once() {
        let base = std::env::temp_dir().join(format!("sketch-rs-root-{:016x}", fastrand::u64(..)));
        let (old, new) = (base.join("paper-rs"), base.join("sketch-rs"));
        std::fs::create_dir_all(old.join("Plan")).unwrap();
        assert_eq!(adopt_old_root(old.clone(), new.clone()), new);
        assert!(new.join("Plan").is_dir());
        assert!(!old.exists());

        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(adopt_old_root(old.clone(), new.clone()), new);
        assert!(
            old.exists(),
            "an existing new folder wins and the old one is left alone"
        );
        std::fs::remove_dir_all(base).ok();
    }
}
