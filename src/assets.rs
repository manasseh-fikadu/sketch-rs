//! Image assets, registered process-wide by content hash.
//!
//! Image fills reference an asset id (a hash of the encoded bytes). The registry keeps the
//! decoded pixels for rendering and the original bytes for saving and export. Projects store
//! their assets as `assets/<id>.<ext>`; copy/paste between projects carries only the id, and
//! the destination project writes the file on its next save.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub struct ImageAsset {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub rgba: image::RgbaImage,
    /// The original encoded file.
    pub bytes: Vec<u8>,
    /// "png", "jpg", ...
    pub ext: &'static str,
}

impl ImageAsset {
    pub fn mime(&self) -> &'static str {
        match self.ext {
            "jpg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => "image/png",
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<ImageAsset>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<ImageAsset>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// FNV-1a 64-bit content hash as 16 hex digits.
pub fn hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn ext_of(bytes: &[u8]) -> &'static str {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::Gif) => "gif",
        Ok(image::ImageFormat::WebP) => "webp",
        _ => "png",
    }
}

/// Decode and register encoded image bytes; returns the asset id.
pub fn register(bytes: &[u8]) -> Result<String, String> {
    let id = hash(bytes);
    if registry().lock().unwrap().contains_key(&id) {
        return Ok(id);
    }
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("unsupported image: {e}"))?
        .to_rgba8();
    let a = ImageAsset {
        id: id.clone(),
        width: img.width(),
        height: img.height(),
        rgba: img,
        bytes: bytes.to_vec(),
        ext: ext_of(bytes),
    };
    registry().lock().unwrap().insert(id.clone(), Arc::new(a));
    Ok(id)
}

/// Register raw pixels (generated images); encodes them as PNG.
pub fn register_rgba(img: &image::RgbaImage) -> String {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png encodes");
    register(&out.into_inner()).expect("own png decodes")
}

pub fn get(id: &str) -> Option<Arc<ImageAsset>> {
    registry().lock().unwrap().get(id).cloned()
}

/// Register every image in a project's `assets/` folder.
pub fn load_dir(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if let Ok(bytes) = std::fs::read(e.path())
            && let Err(err) = register(&bytes)
        {
            log::warn!("{}: {err}", e.path().display());
        }
    }
}

/// Write the given assets into `dir` (skipping ones already there).
pub fn save_to_dir(dir: &Path, ids: impl IntoIterator<Item = String>) -> std::io::Result<()> {
    for id in ids {
        let Some(a) = get(&id) else { continue };
        let path = dir.join(format!("{id}.{}", a.ext));
        if !path.exists() {
            std::fs::create_dir_all(dir)?;
            crate::io::atomic::write(&path, &a.bytes)?;
        }
    }
    Ok(())
}

/// Asset ids referenced by a document's image fills.
pub fn used_by(doc: &crate::model::Document) -> Vec<String> {
    let mut out: Vec<String> = doc
        .nodes
        .values()
        .flat_map(|n| n.style.fills.iter())
        .filter_map(|p| match p {
            crate::model::Paint::Image { asset, .. } => Some(asset.clone()),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Base64 (standard alphabet, padded), for data URLs in SVG export.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(base64_decode(&base64(s)).unwrap(), s);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn register_dedups_by_content() {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]));
        let a = register_rgba(&img);
        let b = register_rgba(&img);
        assert_eq!(a, b);
        let asset = get(&a).unwrap();
        assert_eq!((asset.width, asset.height), (3, 2));
        assert!(register(b"not an image").is_err());
    }
}
