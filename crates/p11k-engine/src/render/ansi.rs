//! ANSI escape handling: strip sequences and measure the visible width.
//!
//! The renderer writes SGR (CSI), OSC 8 hyperlinks and, through the prompt protocol, other
//! escape sequences. Width and stripping share one scanner so a sequence can never count as
//! text: an OSC 8 URL such as `file:///tmp/proj` contains `m` characters, and a scanner that
//! stops at the first `m` would both miscount the URL and cut the following text.

/// Visible width: escape sequences skipped, wide characters count 2, combining characters 0.
pub(super) fn width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    if !s.contains('\x1b') {
        return s.width();
    }
    strip(s).width()
}

/// The text with every escape sequence removed.
pub(super) fn strip(s: &str) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(s)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csi_sequences_are_not_text() {
        assert_eq!(width("\x1b[38;5;196mred\x1b[0m"), 3);
        assert_eq!(strip("\x1b[1;38;5;196mred\x1b[0m"), "red");
    }

    #[test]
    fn osc8_hyperlinks_are_not_text() {
        let link = "\x1b]8;;file:///tmp/mnt/proj\x07txt\x1b]8;;\x07";
        assert_eq!(width(link), 3);
        assert_eq!(strip(link), "txt");
    }

    #[test]
    fn unterminated_sequences_are_skipped_to_the_end() {
        assert_eq!(width("abc\x1b[1;"), 3);
        assert_eq!(strip("abc\x1b]8;;file:///x"), "abc");
    }
}
