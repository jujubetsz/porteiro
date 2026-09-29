use crate::catalog::AccentColor;
use ratatui::style::Color;

pub const BG: Color = Color::Rgb(0x15, 0x17, 0x1c);
pub const TEXT: Color = Color::Rgb(0xcf, 0xd2, 0xd6);
pub const HEADER_BG: Color = Color::Rgb(0x35, 0x33, 0x8a);
pub const HEADER_FG: Color = Color::Rgb(0xe8, 0xe8, 0xf2);
pub const ACCENT: Color = Color::Rgb(0xe2, 0xa5, 0x54);
pub const IDENT: Color = Color::Rgb(0x51, 0xc9, 0xd4);
pub const BAD: Color = Color::Rgb(0xff, 0x6b, 0x8a);
pub const MUTED: Color = Color::Rgb(0x8b, 0x90, 0x9a);
pub const SELECT_BG: Color = Color::Rgb(0x27, 0x4a, 0x44);
pub const TAG_BG: Color = Color::Rgb(0x45, 0x4d, 0x60);
pub const TAG_FG: Color = Color::Rgb(0xe4, 0xe8, 0xf2);

/// Use the terminal's named palette for user accents; never interpret metadata
/// as an ANSI sequence. Text and selection still carry meaning without color.
pub fn accent(color: Option<AccentColor>) -> Color {
    match color {
        Some(AccentColor::Red) => Color::LightRed,
        Some(AccentColor::Yellow) => Color::LightYellow,
        Some(AccentColor::Green) => Color::LightGreen,
        Some(AccentColor::Blue) => Color::LightBlue,
        Some(AccentColor::Magenta) => Color::LightMagenta,
        Some(AccentColor::Cyan) => Color::LightCyan,
        Some(AccentColor::White) => Color::White,
        Some(AccentColor::Gray) => Color::Gray,
        None => IDENT,
    }
}
