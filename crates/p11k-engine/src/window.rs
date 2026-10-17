//! Writing the prompt window to the real terminal.

use std::io::{self, Write};

use crate::config::Config;
use crate::theme::{self, GitStatus, HeaderInfo};

/// Paint the header after the shell's prompt-ready signal.
///
/// The cursor sits at the start of the input line (the shell printed the blank rows, the
/// signal and the prefix; zle/readline draw the user's buffer right after the prefix), so the
/// header is filled by moving up `header_rows` rows and covering them. The prefix is not drawn
/// here: it belongs to the shell's prompt string.
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
    theme::redraw_header_cfg(out, rows, config, info, vcs)
}
