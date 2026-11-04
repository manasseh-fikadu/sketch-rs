//! Importing files into projects.

use crate::project::{PageData, Project, format};
use std::path::Path;

/// Read a file into pages for a new project. Returns a suggested project name.
pub fn import_as_pages(path: &Path) -> Result<(String, Vec<PageData>), String> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported".into());
    if path.is_dir() {
        let p = Project::open(path).map_err(|e| e.to_string())?;
        let pages = p.load_pages().map_err(|e| e.to_string())?;
        return Ok((p.manifest.name, pages));
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let page = format::page_from_json(&text).map_err(|e| e.to_string())?;
    let name = if page.name.is_empty() || page.name == "Untitled" {
        stem
    } else {
        page.name.clone()
    };
    Ok((name, vec![page]))
}

/// Load the pages of a project folder or of a single document file (CLI export).
pub fn load_any(path: &Path) -> Result<Vec<PageData>, String> {
    import_as_pages(path).map(|(_, pages)| pages)
}
