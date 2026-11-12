//! `p11k configure` — the interactive configuration wizard.
//!
//! Aligned with the question set p10k `p10k configure` exposes: font detection (three glyphs
//! confirmed by eye), style, character set, color variant, time, separators/ends, height,
//! connection, frame, spacing, icons, prefixes, transient. Answers edit [`Config`] fields
//! directly and the result is serialized to KDL on disk. Keys: `q` quits (writing nothing),
//! `r` restarts, digits/letters select.

mod input;
mod preview;
mod questions;
#[cfg(test)]
mod tests;
mod ui;
mod write;

use std::io::{self, Write};

use crate::config::IconMode;
use crate::i18n::{msgid, t};
use crate::presets::PresetKind;

use self::input::{Input, StdinInput};
use self::preview::header_on;
use self::questions::{
    ask_charset, ask_color, ask_empty_line, ask_extra_icons, ask_font, ask_frame, ask_gap_char,
    ask_heads, ask_num_lines, ask_prefixes, ask_separators, ask_style, ask_tails, ask_time,
    ask_transient, ask_use_rprompt,
};
use self::ui::{ask_yn, line, preview};
use self::write::write_config;

/// Why a question stopped: the user asked to restart or quit, or a write failed.
#[derive(Debug)]
enum Control {
    Restart,
    Quit,
    Io(io::Error),
}

impl From<io::Error> for Control {
    fn from(e: io::Error) -> Control {
        Control::Io(e)
    }
}

/// One answer; an error carries the control flow out of the wizard.
type Answer<T> = Result<T, Control>;

pub fn run() -> anyhow::Result<()> {
    // The wizard reads single keystrokes; without a terminal there is nothing to read and the
    // raw-mode switch cannot apply. Fail instead of reading EOF as an answer.
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 0 {
        anyhow::bail!("{}", t("p11k configure needs an interactive terminal"));
    }
    let _raw = crate::state::RawTerminal::enter(libc::STDIN_FILENO)?;
    let mut out = io::stdout();
    let mut input = StdinInput;
    loop {
        match flow(&mut out, &mut input) {
            Ok(()) => return Ok(()),
            Err(Control::Restart) => continue,
            Err(Control::Quit) => return goodbye(&mut out),
            Err(Control::Io(e)) => return Err(e.into()),
        }
    }
}

fn flow(out: &mut dyn Write, input: &mut dyn Input) -> Answer<()> {
    // 1. Font detection → icon mode.
    let mode = ask_font(out, input)?;
    // 2. Style.
    let kind = ask_style(out, input, &mode)?;
    // 3. Color variant (branched by style) → build a Config with the color arguments.
    let mut cfg = ask_color(out, input, kind, &mode)?;
    // 4. Character set (asked when not ascii; can explicitly switch to ASCII).
    if cfg.mode != IconMode::Ascii && ask_charset(out, input, &cfg, &mode)? {
        cfg.mode = IconMode::Ascii;
    }
    // 5. Location of pure's non-permanent content.
    if kind == PresetKind::Pure {
        ask_use_rprompt(out, input, &mut cfg)?;
    }
    // 6. Time.
    ask_time(out, input, &mut cfg, kind)?;
    // 7. Separators/ends (classic/rainbow only).
    if matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        ask_separators(out, input, &mut cfg)?;
        ask_heads(out, input, &mut cfg)?;
        ask_tails(out, input, &mut cfg)?;
    }
    // 8. Height.
    ask_num_lines(out, input, &mut cfg)?;
    // 9. Connection line (two lines and classic/rainbow only).
    if header_on(&cfg) && matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        ask_gap_char(out, input, &mut cfg)?;
        ask_frame(out, input, &mut cfg)?;
    }
    // 10. Spacing.
    ask_empty_line(out, input, &mut cfg)?;
    // 11. Icons.
    ask_extra_icons(out, input, &mut cfg)?;
    // 12. Prefixes.
    ask_prefixes(out, input, &mut cfg)?;
    // 13. transient.
    ask_transient(out, input, &mut cfg)?;
    // 14. Preview + confirmation.
    preview(out, &cfg)?;
    if !ask_yn(
        out,
        input,
        msgid("Use this theme?"),
        msgid("Yes, write the config."),
        msgid("No."),
        None,
    )? {
        return Err(Control::Quit);
    }
    // 15. Write to disk (asking about overwrite first if it exists).
    write_config(out, input, &cfg)?;
    Ok(())
}

fn goodbye(out: &mut dyn Write) -> anyhow::Result<()> {
    let _ = line(out, "");
    let _ = line(out, &t("p11k configuration wizard has been aborted."));
    let _ = line(out, &t("Nothing was written."));
    Ok(())
}
