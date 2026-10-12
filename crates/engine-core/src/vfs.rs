//! Case-insensitive access to the user's game files.
//!
//! The shipped data mixes upper- and lower-case names (`LIPS00.XRB`, `hand01.pic`) and was
//! written for a case-insensitive file system, so every lookup folds case.

use std::collections::HashMap;
use std::io;

/// A read-only set of game files keyed by their path relative to the game directory.
#[derive(Default)]
pub struct Vfs {
    files: HashMap<String, Vec<u8>>,
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file, e.g. from a browser file picker. `path` uses `/` separators.
    pub fn insert(&mut self, path: &str, data: Vec<u8>) {
        self.files.insert(normalize(path), data);
    }

    pub fn read(&self, path: &str) -> io::Result<&[u8]> {
        self.files
            .get(&normalize(path))
            .map(Vec::as_slice)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("game file not found: {path}")))
    }

    /// All file paths (normalised: lowercase, `/` separators).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    /// Overlay every file below `dir` on top of the current set (mods replace game files).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn overlay_dir(&mut self, dir: &std::path::Path) -> io::Result<usize> {
        let overlay = Self::from_dir(dir)?;
        let count = overlay.files.len();
        self.files.extend(overlay.files);
        Ok(count)
    }

    /// Load every file below `dir` (native builds only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dir(dir: &std::path::Path) -> io::Result<Self> {
        let mut vfs = Self::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current)? {
                let path = entry?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(rel) = path.strip_prefix(dir) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    vfs.insert(&rel, std::fs::read(&path)?);
                }
            }
        }
        Ok(vfs)
    }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_ascii_lowercase()
}
