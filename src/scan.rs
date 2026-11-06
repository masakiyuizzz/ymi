//! Finding pictures, filtering them, and completing paths.

use std::path::{Path, PathBuf};

const EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "avif", "jxl",
];

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Every picture under `root`, recursively, sorted for stable listing.
pub fn scan(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && is_image(e.path()))
        .map(|e| e.path().display().to_string())
        .collect();
    out.sort();
    out
}

/// Filter as you type: every whitespace-separated word must appear somewhere.
pub fn filter<'a>(items: &'a [String], query: &str) -> Vec<usize> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    if words.is_empty() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let low = p.to_ascii_lowercase();
            words.iter().all(|w| low.contains(w))
        })
        .map(|(i, _)| i)
        .collect()
}

/// `~` expansion for paths typed by hand.
pub fn expand(input: &str) -> PathBuf {
    let s = input.trim();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(s)
}

pub struct Completion {
    /// the input with the common prefix filled in
    pub filled: String,
    /// candidates to show when there is more than one
    pub options: Vec<String>,
    /// true when `filled` names a directory (trailing slash added)
    pub unique: bool,
}

/// Tab completion over the filesystem: completes the last path component and
/// keeps the directory part untouched. Shell-grade completion is a rabbit hole;
/// this covers "type a folder, press Tab, see what is in it".
pub fn complete(input: &str) -> Completion {
    let expanded = expand(input);
    let typed_name = expanded
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = if input.ends_with('/') {
        expanded.clone()
    } else {
        expanded
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') && !typed_name.starts_with('.') {
                        return None;
                    }
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if !is_dir && !is_image(&e.path()) {
                        return None;
                    }
                    Some(name)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();

    let matches: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(&typed_name))
        .cloned()
        .collect();

    if matches.is_empty() {
        return Completion {
            filled: input.to_string(),
            options: Vec::new(),
            unique: false,
        };
    }

    let prefix = common_prefix(&matches);
    let mut filled = String::new();
    if let Some(parent) = expanded.parent() {
        if !typed_name.is_empty() || input.contains('/') {
            filled.push_str(&parent.display().to_string());
            if !filled.ends_with('/') {
                filled.push('/');
            }
        }
    }
    filled.push_str(&prefix);

    let unique = matches.len() == 1;
    if unique {
        let only = dir.join(&matches[0]);
        if only.is_dir() {
            filled.push('/');
        }
    }

    Completion {
        filled,
        options: matches,
        unique,
    }
}

fn common_prefix(items: &[String]) -> String {
    let mut iter = items.iter();
    let Some(first) = iter.next() else {
        return String::new();
    };
    let mut end = first.chars().count();
    for other in iter {
        let n = first
            .chars()
            .zip(other.chars())
            .take_while(|(a, b)| a == b)
            .count();
        end = end.min(n);
    }
    first.chars().take(end).collect()
}
