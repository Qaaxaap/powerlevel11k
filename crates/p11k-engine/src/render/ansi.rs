//! ANSI escape handling: strip sequences and measure the visible width.
//!
//! The renderer writes SGR (CSI), OSC 8 hyperlinks and, through the prompt protocol, other
//! escape sequences. Width and stripping share one scanner so a sequence can never count as
//! text: an OSC 8 URL such as `file:///tmp/proj` contains `m` characters, and a scanner that
//! stops at the first `m` would both miscount the URL and cut the following text.

/// Byte length of the escape sequence starting at `bytes[i]` (an ESC byte). An unterminated
/// sequence runs to the end of the string.
fn escape_len(bytes: &[u8], i: usize) -> usize {
    let Some(&kind) = bytes.get(i + 1) else {
        return 1;
    };
    match kind {
        // CSI: parameter/intermediate bytes, then a final byte in 0x40..=0x7e.
        b'[' => {
            let mut j = i + 2;
            while j < bytes.len() {
                if (0x40..=0x7e).contains(&bytes[j]) {
                    return j - i + 1;
                }
                j += 1;
            }
            bytes.len() - i
        }
        // OSC: terminated by BEL or ST (ESC `\`). OSC 8 hyperlinks use BEL here.
        b']' => {
            let mut j = i + 2;
            while j < bytes.len() {
                if bytes[j] == 0x07 {
                    return j - i + 1;
                }
                if bytes[j] == 0x1b && bytes.get(j + 1) == Some(&b'\\') {
                    return j - i + 2;
                }
                j += 1;
            }
            bytes.len() - i
        }
        // Charset/single-byte designators (ESC ( B) are three bytes; other ESC pairs two.
        b'(' | b')' | b'#' | b'%' => 3.min(bytes.len() - i),
        _ => 2.min(bytes.len() - i),
    }
}

/// Visible width: escape sequences skipped, wide characters count 2, combining characters 0.
pub(super) fn width(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut w = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += escape_len(bytes, i);
            continue;
        }
        let ch = s[i..].chars().next().expect("a char starts at this byte");
        w += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        i += ch.len_utf8();
    }
    w
}

/// The text with every escape sequence removed.
pub(super) fn strip(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += escape_len(bytes, i);
            continue;
        }
        let ch = s[i..].chars().next().expect("a char starts at this byte");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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
