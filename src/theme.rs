use ratatui::style::Color;

#[derive(Clone)]
pub struct Theme {
    pub accent: Color,
    pub accent2: Color,
    /// secondary text: artist, album, stats
    pub muted: Color,
    /// tertiary text: numbers, durations, hints — still has to be readable
    pub dim: Color,
    /// rules, progress track, subtle fills
    pub faint: Color,
    /// outer frame
    pub border: Color,
    pub text: Color,
    pub sel_bg: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            accent: Color::Rgb(0xE9, 0xB4, 0x6A),
            accent2: Color::Rgb(0x7F, 0xD1, 0xC4),
            muted: Color::Rgb(0xBE, 0xC8, 0xD7),
            dim: Color::Rgb(0x93, 0x9E, 0xB0),
            faint: Color::Rgb(0x4E, 0x57, 0x66),
            border: Color::Rgb(0x7C, 0x87, 0x99),
            text: Color::Reset,
            sel_bg: Color::Rgb(0x27, 0x2C, 0x38),
        }
    }

    pub fn light() -> Self {
        Self {
            accent: Color::Rgb(0xA8, 0x62, 0x00),
            accent2: Color::Rgb(0x0E, 0x7A, 0x8C),
            muted: Color::Rgb(0x3F, 0x47, 0x53),
            dim: Color::Rgb(0x5C, 0x64, 0x71),
            faint: Color::Rgb(0xC2, 0xC8, 0xD1),
            border: Color::Rgb(0x9A, 0xA2, 0xAE),
            text: Color::Reset,
            sel_bg: Color::Rgb(0xE8, 0xEA, 0xEF),
        }
    }

    pub fn from_light(light: bool) -> Self {
        if light {
            Self::light()
        } else {
            Self::dark()
        }
    }
}
