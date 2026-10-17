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

/// The zero-width prompt-ready signal a shell appends to its prompt string:
/// `ESC ] 1337 ; p11k ; p BEL`. It occupies no columns, and terminals ignore unknown OSC
/// sequences, so it passes through the terminal untouched while the engine sees it in the
/// byte stream.
pub const PROMPT_SIGNAL: &[u8] = b"\x1b]1337;p11k;p\x07";

/// Length of the longest suffix of `hay` that is a proper prefix of `needle`.
///
/// A read can split the signal anywhere. This is how much of the tail has to wait for the
/// next read before it can be forwarded as ordinary output; only real prefixes are held
/// back, so plain output is never delayed.
fn signal_prefix_tail(hay: &[u8], needle: &[u8]) -> usize {
    let max = hay.len().min(needle.len() - 1);
    (1..=max)
        .rev()
        .find(|&n| hay[hay.len() - n..] == needle[..n])
        .unwrap_or(0)
}

/// Strips prompt signals out of the inner shell's byte stream.
///
/// The engine forwards everything the shell writes, so a signal has to be removed before it
/// is forwarded; one split across two reads is held back until the next read confirms it.
#[derive(Default)]
pub struct SignalScanner {
    pending: Vec<u8>,
}

/// What one read contributed: the bytes before the last signal, how many signals it held, and
/// the bytes after the last signal.
pub struct Scan {
    /// Everything up to the last complete signal (the blank rows the shell printed for the
    /// header). Forward this first.
    pub before: Vec<u8>,
    /// Complete signals seen in this chunk.
    pub signals: usize,
    /// What follows the last signal (the input-line prefix). Forward this after painting.
    pub after: Vec<u8>,
}

impl SignalScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one read's worth of bytes. A signal split across two reads stays in `pending`
    /// until the next read completes it.
    pub fn push(&mut self, chunk: &[u8]) -> Scan {
        self.pending.extend_from_slice(chunk);
        let mut before = Vec::new();
        let mut after: Vec<u8> = Vec::new();
        let mut signals = 0;
        let mut from = 0;
        while let Some(rel) = find_bytes(&self.pending[from..], PROMPT_SIGNAL) {
            let at = from + rel;
            let target = if signals == 0 {
                &mut before
            } else {
                &mut after
            };
            target.extend_from_slice(&self.pending[from..at]);
            signals += 1;
            from = at + PROMPT_SIGNAL.len();
        }
        let rest = &self.pending[from..];
        let keep = signal_prefix_tail(rest, PROMPT_SIGNAL);
        let tail = &rest[..rest.len() - keep];
        if signals == 0 {
            before.extend_from_slice(tail);
        } else {
            after.extend_from_slice(tail);
        }
        self.pending = rest[rest.len() - keep..].to_vec();
        Scan {
            before,
            signals,
            after,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_scanner_splits_before_and_after_the_last_signal() {
        let mut s = SignalScanner::new();
        let scan = s.push(b"abc");
        assert_eq!(
            (scan.before.as_slice(), scan.signals, scan.after.as_slice()),
            (b"abc".as_slice(), 0, b"".as_slice())
        );

        // A signal cut in the middle must not leak its head as output.
        let scan = s.push(b"\x1b]1337;p11");
        assert!(
            scan.before.is_empty() && scan.after.is_empty(),
            "head of a split signal leaked"
        );
        assert_eq!(scan.signals, 0);
        let scan = s.push(b"k;p\x07tail");
        assert_eq!(
            (scan.before.as_slice(), scan.signals, scan.after.as_slice()),
            (b"".as_slice(), 1, b"tail".as_slice())
        );

        // Blank rows ahead of the signal go to `before`, the prefix behind it to `after`.
        let scan = s.push(b"\r\n\x1b]1337;p11k;p\x07\xe2\x9d\xaf ");
        assert_eq!(
            (scan.before.as_slice(), scan.signals, scan.after.as_slice()),
            (b"\r\n".as_slice(), 1, "\u{276f} ".as_bytes())
        );
    }

    #[test]
    fn signal_scanner_does_not_delay_plain_output() {
        let mut s = SignalScanner::new();
        // A lone ESC is not a prefix of the signal, so it goes out immediately.
        let scan = s.push(b"\x1b[31m");
        assert_eq!(
            (scan.before.as_slice(), scan.signals, scan.after.as_slice()),
            (b"\x1b[31m".as_slice(), 0, b"".as_slice())
        );
    }

    #[test]
    fn signal_prefix_tail_matches_only_real_prefixes() {
        assert_eq!(signal_prefix_tail(b"", PROMPT_SIGNAL), 0);
        assert_eq!(signal_prefix_tail(b"xyz", PROMPT_SIGNAL), 0);
        assert_eq!(signal_prefix_tail(b"abc\x1b]13", PROMPT_SIGNAL), 4);
        // A complete signal is not a proper prefix; the caller has already consumed it.
        assert_eq!(signal_prefix_tail(PROMPT_SIGNAL, PROMPT_SIGNAL), 0);
    }
}
