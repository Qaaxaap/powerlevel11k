//! The announce-file protocol: the shell writes one line per prompt event, the engine reads
//! them in order, plus the byte-level helpers the pass-through stream needs.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;

use crate::theme::HeaderInfo;

/// Announce messages: drawing the prompt window and resize.
pub enum AnnMsg {
    /// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`: precmd announce, draws the header.
    Header(HeaderInfo),
    /// `p`: zle-line-init announce, paints the input-line prefix over the placeholder.
    Prompt,
    /// `r`: resize announce (zsh's TRAPWINCH, bash's trap WINCH, fish's `--on-signal
    /// WINCH`, pwsh compares the window size inside prompt); resize the pty + repaint.
    Resize,
    /// `v\t<keymap>`: zle-keymap-select announce, updates the editing mode and repaints the
    /// header.
    VimMode(String),
}

/// Read new lines from the announce file and parse them into prompt messages.
/// Line format (first field is the message type, the rest are tab-separated):
/// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`, `p`, `r`, `v\t<keymap>`.
/// Missing fields are tolerated: a failed `<exit>` parse is None, a missing `<cwd>` is an
/// empty string, missing or invalid `<jobs>`/`<history>` are 0. Only zsh and pwsh send
/// `<history>`; bash/fish have no such column.
pub fn drain_announce(path: &Path, processed: &mut u64) -> Vec<AnnMsg> {
    let mut out = Vec::new();
    let Ok(mut f) = OpenOptions::new().read(true).open(path) else {
        return out;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return out;
    };
    if len <= *processed {
        return out;
    }
    use std::io::Seek;
    if f.seek(io::SeekFrom::Start(*processed)).is_err() {
        return out;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return out;
    }
    *processed = len;

    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        let mut parts = line.split('\t');
        match parts.next() {
            Some("h") => {
                let code = parts.next().and_then(|s| s.trim().parse::<i32>().ok());
                let cwd = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                let jobs = parts
                    .next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let history = parts
                    .next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                // `$pipestatus` as sent by the shell, space- or comma-separated; absent
                // for shells without one (pwsh) and for shells whose column is empty.
                let pipestatus = parts
                    .next()
                    .map(|s| {
                        s.split(|c: char| c == ',' || c.is_whitespace())
                            .filter(|p| !p.is_empty())
                            .filter_map(|p| p.parse::<i32>().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(AnnMsg::Header(HeaderInfo {
                    exit_code: code,
                    cwd,
                    exec_seconds: 0.0, // filled in by the main loop from last_enter
                    jobs,
                    history,
                    pipestatus,
                }));
            }
            Some("p") => out.push(AnnMsg::Prompt),
            Some("r") => out.push(AnnMsg::Resize),
            Some("v") => {
                let mode = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                out.push(AnnMsg::VimMode(mode));
            }
            _ => continue,
        }
    }
    out
}

/// Terminal replies to program queries do not count as "the user typing on the input line".
///
/// PSReadLine periodically sends DSR (`\x1b[6n`) to query the cursor position and the
/// terminal's `\x1b[<row>;<col>R` reply arrives on stdin: if the engine fails to recognize
/// it, it takes the reply for a keypress and the empty-enter nudge on shrink never fires
/// again. The same goes for focus events `\x1b[I`/`\x1b[O` and device attribute replies
/// `\x1b[?...c`. Arrow keys (`\x1b[A`) and the like are real keypresses and must not count.
pub fn is_terminal_reply(buf: &[u8]) -> bool {
    let Some(rest) = buf.strip_prefix(b"\x1b[") else {
        return false;
    };
    let Some((&last, params)) = rest.split_last() else {
        return false;
    };
    matches!(last, b'R' | b'c' | b'I' | b'O')
        && params
            .iter()
            .all(|b| b.is_ascii_digit() || *b == b';' || *b == b'?')
}

pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Whether the newlines the prompt prints in front of the marker sit right before it: one per
/// header row, ending at `held` (the bytes still buffered) and running back into `behind` (what
/// has already been written). The tty turns each newline into `\r\n` (ONLCR) and shells dress the
/// blank rows up differently — fish erases each line as it opens it (`\r\n\x1b[K`) — so CRs and
/// escape sequences count as noise, while any ordinary byte ends the run. A run of `_` in the
/// middle of a line therefore still fails this.
pub fn blank_rows_before(behind: &[u8], held: &[u8], rows: usize) -> bool {
    let mut bytes = Vec::with_capacity(behind.len() + held.len());
    bytes.extend_from_slice(behind);
    bytes.extend_from_slice(held);
    let mut newlines = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                newlines += 1;
                i += 1;
            }
            b'\r' => i += 1,
            0x1b => i = skip_escape(&bytes, i),
            _ => {
                newlines = 0;
                i += 1;
            }
        }
    }
    newlines >= rows
}

/// Index just past the escape sequence starting at `start` (which holds `ESC`): CSI up to its
/// final byte, OSC up to `BEL` or `ST`, anything else two bytes. Clamped to the buffer.
pub fn skip_escape(buf: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    if i >= buf.len() {
        return i;
    }
    match buf[i] {
        b'[' => {
            i += 1;
            while i < buf.len() && !(0x40..=0x7e).contains(&buf[i]) {
                i += 1;
            }
            (i + 1).min(buf.len())
        }
        b']' => {
            i += 1;
            while i < buf.len() {
                if buf[i] == 0x07 {
                    return i + 1;
                }
                if buf[i] == 0x1b && buf.get(i + 1) == Some(&b'\\') {
                    return i + 2;
                }
                i += 1;
            }
            i
        }
        _ => i + 1,
    }
}

/// Length of the tail of `held` that could still grow into the marker: a run of `_` shorter than
/// the marker and sitting directly behind a newline — the blank row the prompt prints in front of
/// it. Underscores anywhere else are ordinary output and go out right away.
pub fn marker_prefix_tail(held: &[u8], behind: &[u8], marker: &[u8]) -> usize {
    let mut n = 0;
    while n + 1 < marker.len()
        && n < held.len()
        && held[held.len() - 1 - n] == marker[marker.len() - 1 - n]
    {
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    let start = held.len() - n;
    let behind_newline = if start > 0 {
        held[start - 1] == b'\n'
    } else {
        behind.last() == Some(&b'\n')
    };
    if behind_newline { n } else { 0 }
}
