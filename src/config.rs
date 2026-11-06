//! Preferences that outlive a run.
//!
//! A handful of settings are worth remembering, but not worth a dependency on
//! a config crate. The file is `~/.config/<name>/config` (or under
//! `$XDG_CONFIG_HOME`), one `key = value` per line, `#` starts a comment.
//! Unknown keys are ignored, so an older binary never chokes on a newer file.

use std::io::Write;
use std::path::PathBuf;

use crate::i18n::Lang;

/// `$XDG_CONFIG_HOME/<name>/config`, falling back to `~/.config/<name>/config`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join(crate::cli::NAME).join("config"))
}

/// The language stored on disk, if the file exists and says something valid.
pub fn read_lang() -> Option<Lang> {
    let text = std::fs::read_to_string(path()?).ok()?;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("lang") {
            return Lang::parse(value);
        }
    }
    None
}

/// Remember the language. Creates the directory when missing and writes through
/// a temporary file, so a crash cannot leave a half-written config behind.
pub fn write_lang(lang: Lang) -> std::io::Result<PathBuf> {
    let path = path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no HOME or XDG_CONFIG_HOME")
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // Keep any other keys the file already has; only `lang` is rewritten.
    let mut out = String::new();
    let mut seen = false;
    if let Ok(old) = std::fs::read_to_string(&path) {
        for line in old.lines() {
            let bare = line.split('#').next().unwrap_or("").trim();
            if let Some((key, _)) = bare.split_once('=') {
                if key.trim().eq_ignore_ascii_case("lang") {
                    out.push_str(&format!("lang = {}\n", lang.code()));
                    seen = true;
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    if !seen {
        if out.is_empty() {
            out.push_str(&format!("# {} preferences\n", crate::cli::NAME));
        }
        out.push_str(&format!("lang = {}\n", lang.code()));
    }

    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.flush()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}
