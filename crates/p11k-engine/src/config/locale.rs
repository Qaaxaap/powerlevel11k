use super::*;

pub(super) fn locale_is_utf8() -> bool {
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return locale_str_is_utf8(v);
            }
        }
    }
    false // nothing set → C locale → not UTF-8
}

/// Whether one locale string's codeset (e.g. `en_US.UTF-8`, `C.UTF-8`, `C`) is UTF-8.
pub(super) fn locale_str_is_utf8(locale: &str) -> bool {
    let code = locale
        .rsplit('.')
        .next()
        .unwrap_or("")
        .split('@') // strip the modifier, e.g. UTF-8@euro
        .next()
        .unwrap_or("");
    code.eq_ignore_ascii_case("utf-8") || code.eq_ignore_ascii_case("utf8")
}

/// The best icon mode the current locale can display: a UTF-8 locale can display every glyph
/// the built-in tables use, anything else (C/POSIX, an unset or invalid locale) is ASCII only.
pub(super) fn locale_capable_mode() -> IconMode {
    if locale_is_utf8() {
        IconMode::NerdfontComplete
    } else {
        IconMode::Ascii
    }
}

/// The lowest icon mode that can display `c`: ASCII needs ascii, the private-use areas hold the
/// Nerd Font and Powerline glyphs, and every other non-ASCII character is standard Unicode.
pub(super) fn glyph_required_mode(c: char) -> IconMode {
    if c.is_ascii() {
        return IconMode::Ascii;
    }
    let cp = c as u32;
    // Powerline symbols sit in the BMP private-use area, but the compatible tier is documented
    // to carry them and its built-in icons use them, so they stay compatible.
    if (0xE0A0..=0xE0B3).contains(&cp) {
        return IconMode::Compatible;
    }
    // Private Use Area: BMP, plane 15 and plane 16. Nerd Font glyphs live here.
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFD).contains(&cp)
        || (0x100000..=0x10FFFD).contains(&cp)
    {
        return IconMode::NerdfontFontconfig;
    }
    IconMode::Compatible
}

/// The lowest icon mode that can display every character of `s`.
pub(super) fn text_required_mode(s: &str) -> IconMode {
    s.chars()
        .map(glyph_required_mode)
        .max_by_key(|m| m.rank())
        .unwrap_or(IconMode::Ascii)
}

/// Whether `s` contains a character the mode cannot display.
pub(super) fn text_overflows_mode(s: &str, mode: &IconMode) -> bool {
    text_required_mode(s).rank() > mode.rank()
}

/// Whether the mode can display `c` (used by the `all` icon auto-detection).
pub(crate) fn glyph_fits_mode(c: char, mode: &IconMode) -> bool {
    glyph_required_mode(c).rank() <= mode.rank()
}

/// A warning for one declared glyph that fell back to the built-in default.
pub(super) fn glyph_fallback(field: &str, glyph: &str) -> String {
    t_args(
        "config-glyph-fallback",
        &[("field", field), ("glyph", glyph)],
    )
}
