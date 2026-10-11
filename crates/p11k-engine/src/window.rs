//! Writing the prompt window to the real terminal.

use std::io::{self, Write};

use crate::config::Config;
use crate::theme::{self, GitStatus, HeaderInfo};

/// Write bytes through to the terminal and remember the tail of what has been written: the marker
/// match looks at the blank rows in front of it, and on a read boundary those may already be out.
pub fn emit(out: &mut dyn Write, recent: &mut Vec<u8>, bytes: &[u8], cap: usize) -> io::Result<()> {
    out.write_all(bytes)?;
    recent.extend_from_slice(bytes);
    if recent.len() > cap {
        recent.drain(..recent.len() - cap);
    }
    Ok(())
}

/// Paint the header and the input-line prefix over a placeholder the shell has just printed:
/// the cursor sits right after the marker, so moving up `header_rows` rows lands on the blank
/// rows the shell left there for the header.
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
    // The prefix is generated dynamically from the current exit code.
    let text = crate::render::input_prefix(config, current).text;
    theme::render_prompt(out, &text)
}
