//! The announce protocol: the shell writes one line per prompt event into a FIFO, the engine
//! reads them in order, plus the byte-level helpers the pass-through stream needs.

use crate::theme::HeaderInfo;

/// Announce messages: drawing the prompt window and resize.
pub enum AnnMsg {
    /// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`: precmd announce, draws the header.
    Header(HeaderInfo),
    /// `r`: resize announce (zsh's TRAPWINCH, bash's trap WINCH, fish's `--on-signal
    /// WINCH`, pwsh compares the window size inside prompt); resize the pty + repaint.
    Resize,
    /// `v\t<keymap>`: zle-keymap-select announce, updates the editing mode and repaints the
    /// header.
    VimMode(String),
}

/// Read pending lines from the announce FIFO and parse them into prompt messages.
/// Line format (first field is the message type, the rest are tab-separated):
/// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`, `r`, `v\t<keymap>`.
/// Missing fields are tolerated: a failed `<exit>` parse is None, a missing `<cwd>` is an
/// empty string, missing or invalid `<jobs>`/`<history>` are 0. Only zsh and pwsh send
/// `<history>`; bash/fish have no such column.
///
/// The caller waits for the FIFO to become readable with poll, so one drain takes everything
/// that is buffered and returns. A read can end in the middle of a line: the partial line stays
/// in `pending` until the next read completes it.
pub fn drain_announce(fd: i32, pending: &mut Vec<u8>) -> Vec<AnnMsg> {
    let mut out = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        // SAFETY: reads into a stack array; n bounds the slice.
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // EAGAIN: nothing buffered. The caller already knows it is readable, so a short
            // read is normal.
            break;
        }
        if n == 0 {
            // Every writer closed. The engine keeps its own write end, so this is not EOF
            // for the read end; it only means this drain saw no data.
            break;
        }
        out.extend(push_announce(pending, &chunk[..n as usize]));
    }
    out
}

/// Append one read's bytes and return the messages it completed.
///
/// A read can end in the middle of a line. Only complete lines are parsed; the tail stays in
/// `pending` for the next read. Without this a line split across two reads would be parsed twice,
/// the first time as a truncated message with a wrong `cwd`.
pub fn push_announce(pending: &mut Vec<u8>, chunk: &[u8]) -> Vec<AnnMsg> {
    pending.extend_from_slice(chunk);
    let Some(end) = pending.iter().rposition(|&b| b == b'\n').map(|i| i + 1) else {
        // No complete line yet. A writer that never finishes one line is broken; do not let a
        // broken writer grow the buffer without bound.
        if pending.len() > MAX_ANNOUNCE_LINE {
            pending.clear();
        }
        return Vec::new();
    };
    let msgs = parse_announce_lines(&pending[..end]);
    pending.drain(..end);
    msgs
}

/// Longest partial announce line kept across reads. Real lines are a few hundred bytes at most.
const MAX_ANNOUNCE_LINE: usize = 64 * 1024;

/// Parse announce lines (see [`drain_announce`]).
fn parse_announce_lines(buf: &[u8]) -> Vec<AnnMsg> {
    let mut out = Vec::new();
    let text = String::from_utf8_lossy(buf);
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

    /// A line split across two reads must wait for the rest. Parsing the first half would produce
    /// a truncated `cwd`; parsing both halves would produce two messages for one announce.
    #[test]
    fn announce_waits_for_the_rest_of_a_split_line() {
        let mut pending = Vec::new();
        assert!(
            push_announce(&mut pending, b"h\t0\t/tmp").is_empty(),
            "a line without its newline is not complete yet"
        );
        assert_eq!(pending.as_slice(), b"h\t0\t/tmp");

        let msgs = push_announce(&mut pending, b"\t0\t1\n");
        assert_eq!(msgs.len(), 1);
        assert!(pending.is_empty());
        let AnnMsg::Header(info) = &msgs[0] else {
            panic!("expected a header announce");
        };
        assert_eq!(info.cwd, "/tmp");
    }

    /// Complete lines are parsed now; a partial tail waits for the next read.
    #[test]
    fn announce_parses_complete_lines_and_keeps_the_partial_tail() {
        let mut pending = Vec::new();
        let msgs = push_announce(&mut pending, b"r\nh\t0\t/a");
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0], AnnMsg::Resize));
        assert_eq!(pending.as_slice(), b"h\t0\t/a");

        let msgs = push_announce(&mut pending, b"\t0\t2\n");
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0], AnnMsg::Header(_)));
        assert!(pending.is_empty());
    }

    /// A writer that never finishes a line must not grow the buffer without bound.
    #[test]
    fn announce_drops_a_line_that_never_finishes() {
        let mut pending = Vec::new();
        push_announce(&mut pending, &vec![b'x'; MAX_ANNOUNCE_LINE + 1]);
        assert!(
            pending.is_empty(),
            "a runaway line must not grow the buffer"
        );
    }
}
