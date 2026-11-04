//! Crash-safe file writes.
//!
//! Data goes to a uniquely named temp file in the destination directory, is flushed to disk,
//! and is then renamed over the target. Rename within one directory is atomic on every
//! platform we target, so readers see either the old file or the new one, never a torn write.
//! On Unix the directory is fsynced too, so the rename itself survives a power loss.

use std::io::Write;
use std::path::{Path, PathBuf};

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    path.with_file_name(format!(
        ".{name}.{}-{:08x}.tmp",
        std::process::id(),
        fastrand::u32(..)
    ))
}

/// Atomically replace `path` with `data`.
pub fn write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = temp_path(path);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        sync_dir(path.parent().unwrap_or(Path::new(".")));
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Remove temp files left behind by a crash mid-write (best effort).
pub fn clean_stale_temps(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("sketch-rs-atomic-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.json");
        write(&p, b"one").unwrap();
        write(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(names.len(), 1);
        // A crashed write leaves a temp file; the target is untouched and cleanup removes it.
        std::fs::write(temp_path(&p), b"partial").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        clean_stale_temps(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
