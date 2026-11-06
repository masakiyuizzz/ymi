//! Two languages, picked once at startup from `--lang`.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Cn,
    En,
}

static LANG: AtomicU8 = AtomicU8::new(0);

impl Lang {
    pub fn parse(s: &str) -> Option<Lang> {
        // `--lang <en>` and `--lang "en"` are what people actually type when the
        // help shows `<cn|en>`, so the brackets and quotes are not an error.
        let s = s
            .trim()
            .trim_matches(|c: char| c == '<' || c == '>' || c == '"' || c == '\'')
            .trim()
            .to_ascii_lowercase();
        match s.as_str() {
            "cn" | "zh" | "zh-cn" | "zh_cn" | "chinese" | "中文" => Some(Lang::Cn),
            "en" | "eng" | "english" => Some(Lang::En),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Cn => "cn",
            Lang::En => "en",
        }
    }
}

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 0 {
        Lang::Cn
    } else {
        Lang::En
    }
}

pub fn is_en() -> bool {
    lang() == Lang::En
}

/// Pick the string for the current language.
pub fn t(cn: &'static str, en: &'static str) -> &'static str {
    if is_en() {
        en
    } else {
        cn
    }
}

/// Pick between two owned strings (for messages built with `format!`).
pub fn p(cn: String, en: String) -> String {
    if is_en() {
        en
    } else {
        cn
    }
}

/// Display width of a string: CJK glyphs occupy two cells, which is what makes
/// the help columns line up in both languages.
pub fn width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let c = c as u32;
            let wide = matches!(c,
                0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 |
                0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 |
                0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}
