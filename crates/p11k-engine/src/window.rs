//! Writing the prompt window to the real terminal.

use std::io::{self, Write};

use crate::config::Config;
use crate::theme::{self, GitStatus, HeaderInfo};

/// Paint the header and the input-line prefix after the shell's prompt-ready signal.
///
/// The cursor is at the start of the input line (the shell printed the blank rows and the
/// signal, nothing else), so the header is filled by moving up `header_rows` rows, and the
/// prefix is drawn with a plain `\r`: it ends up exactly where the user's input starts.
pub fn paint_prompt(
    out: &mut dyn Write,
    config: &Config,
    rows: usize,
    current: Option<&HeaderInfo>,
    instant: &HeaderInfo,
    last_vcs: Option<&(String, Option<GitStatus>)>,
) -> io::Result<()> {
    let info = current.unwrap_or(instant);
    let vcs = last_vcs
        .filter(|(cwd, _)| cwd == &info.cwd)
        .and_then(|(_, s)| s.as_ref());
    theme::redraw_header_cfg(out, rows, config, info, vcs)?;
    // The prefix is generated dynamically from the current exit code, and its width is not
    // constrained: the cursor simply ends up after it.
    let text = crate::render::input_prefix(config, current).text;
    write!(out, "\r{text}")?;
    // OSC 133 B: tell the terminal the cursor now rests at the input position.
    write!(out, "\x1b]133;B\x07")
}
