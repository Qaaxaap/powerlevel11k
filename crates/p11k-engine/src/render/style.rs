//! Prompt prefix, styles and ANSI painting.
//!
//! The input-line prefix is `frame.last_prefix` plus `prompt_char`, painted for the current
//! state. The header uses the same paint helpers; the prefix helpers also serve the shell-side
//! protocol (`prefix_text_zsh`, `prefix_text_bash`, `transient_prompt_zsh`).

use std::fmt::Write as _;

use crate::config::{AttachText, Color, Config, Style};
use crate::theme::HeaderInfo;

/// Current zsh edit mode (reported by the zle hook, updated live by the engine); empty when
/// not zsh or unreported.
pub static CURRENT_VI_MODE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Input line prefix: `frame.last_prefix` + `prompt_char` content, already coloured.
pub struct InputPrefix {
    pub text: String,
}

/// vi state for the current zsh edit mode (p10k's `VIINS`/`VICMD`/`VIVIS`/`VIOWR`).
pub(super) fn vi_state() -> Option<&'static str> {
    let mode = CURRENT_VI_MODE
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    match mode.as_str() {
        "viins" => Some("VIINS"),
        "vicmd" => Some("VICMD"),
        "vis" | "viopp" => Some("VIVIS"),
        "viowr" => Some("VIOWR"),
        _ => None,
    }
}

/// prompt_char state. vi mode wins (p10k
/// `PROMPT_CHAR_{OK,ERROR}_{VIINS,VICMD,VIVIS,VIOWR}`)—but only when a char is really
/// configured for that state; otherwise the exit code decides: non-zero → ERROR, else normal.
/// State for the input-line prefix: the vi mode's own state when it has one, otherwise the
/// status state. Extended states mirror p10k's `STATUS_EXTENDED_STATES`: a failing pipeline
/// is `ERROR_PIPE`, death by signal is `ERROR_SIGNAL`, and a pipeline whose last stage
/// succeeded while an earlier one failed is `OK_PIPE`.
pub fn prompt_state(config: &Config, info: Option<&HeaderInfo>) -> Option<&'static str> {
    if let Some(st) = vi_state() {
        let configured = config
            .segment("prompt_char")
            .states
            .get(st)
            .map(|s| s.char.is_some())
            .unwrap_or(false);
        if configured {
            return Some(st);
        }
    }
    let info = info?;
    let Some(code) = info.exit_code else {
        return None; // first prompt: nothing has run yet
    };
    let pipes = &info.pipestatus;
    let state = if code == 0 {
        if pipes.len() > 1 && pipes.iter().any(|&c| c != 0) {
            "OK_PIPE"
        } else {
            return None;
        }
    } else if pipes.len() > 1 {
        "ERROR_PIPE"
    } else if code > 128 {
        "ERROR_SIGNAL"
    } else {
        "ERROR"
    };
    // p10k gives every state its own toggle (`STATUS_ERROR_PIPE` and friends). Here a state
    // the config defines wins; otherwise fall back to the plain failure state.
    if segment_has_state(config, state) {
        Some(state)
    } else if state.starts_with("ERROR") {
        Some("ERROR")
    } else {
        None
    }
}

/// Whether the `prompt_char` segment declares this state at all.
pub(super) fn segment_has_state(config: &Config, state: &str) -> bool {
    config.segment("prompt_char").states.contains_key(state)
}

/// Builds the input-line prefix (frame last-prefix piece + prompt char + trailing space) with
/// the given colouring function. Each shell needs its own syntax for marking non-printing
/// sequences, so the shape lives here and the colouring is passed in.
pub(super) fn prefix_text_with(
    config: &Config,
    state: Option<&str>,
    colour: impl Fn(&str, &Style) -> String,
) -> String {
    let mut text = String::new();
    if !config.frame.last_prefix.text.is_empty() {
        let piece = &config.frame.last_prefix;
        text.push_str(&colour(&piece.text, &config.frame_piece_style(piece)));
    }
    let pc = config.segment("prompt_char");
    let st = pc.effective_style(state, &config.defaults);
    text.push_str(&colour(pc.char_for(state, "❯"), &st));
    text.push(' '); // space after the prefix (uncolored), matching the original `❯ ` geometry
    text
}

/// Input line prefix text for a state, in plain ANSI (fish and pwsh read ANSI directly).
pub fn prefix_text(config: &Config, state: Option<&str>) -> String {
    prefix_text_with(config, state, paint)
}

/// The visible text of a string that may carry SGR sequences. Used where the same text has to be
/// matched after a terminal layer stripped the colours, such as PSReadLine repainting the prompt.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Input-line prefix in zsh's prompt syntax: `%F{...}` is zero width for zle, so the prefix can
/// carry its colour without shifting the buffer. Used when the prompt string is built by the
/// shell (the engine cannot paint the prefix itself: zle draws the buffer immediately after the
/// prompt and a later paint would overwrite it).
pub fn prefix_text_zsh(config: &Config, state: Option<&str>) -> String {
    prefix_text_with(config, state, paint_zsh)
}

/// Input-line prefix in bash's prompt syntax: the SGR runs are wrapped in `\[...\]` so readline
/// counts them as zero width.
pub fn prefix_text_bash(config: &Config, state: Option<&str>) -> String {
    prefix_text_with(config, state, paint_bash)
}

/// zsh prompt colouring: `%F{...}` sets the foreground and `%f` resets it, both zero width.
pub(super) fn paint_zsh(text: &str, style: &Style) -> String {
    if text.is_empty() {
        return String::new();
    }
    format!("{}{}%f", zsh_fg(&style.fg), text)
}

/// Computes the input line prefix (e.g. `╰─❯`). `last_prefix` (frame style) and the prompt
/// char (prompt_char style, entering ERROR state from `exit_code`) are colored; `width` is
/// the ANSI-stripped display width.
pub fn input_prefix(config: &Config, info: Option<&HeaderInfo>) -> InputPrefix {
    InputPrefix {
        text: prefix_text(config, prompt_state(config, info)),
    }
}

/// Single-line prompt for transient folding (zsh format). The engine precomputes the zsh
/// prompt escapes; zsh swaps PROMPT in zle-line-finish and folds in sync via reset-prompt.
/// Not easily portable to bash/fish, hence the zsh-only approach.
/// Uses native `%F{...}` escapes;
/// the conditional `%(?\x01OK\x01ERR)` picks a color from the last command's exit code.
pub fn transient_prompt_zsh(config: &Config) -> String {
    let pc = config.segment("prompt_char");
    let ch = pc.char_for(None, "❯");
    let ok = pc.effective_style(None, &config.defaults);
    let err = pc.effective_style(Some("ERROR"), &config.defaults);
    let ok_prompt = format!("{}{} ", zsh_fg(&ok.fg), ch);
    let err_prompt = format!("{}{} ", zsh_fg(&err.fg), ch);
    format!("%(?\u{1}{}\u{1}{})%f", ok_prompt, err_prompt)
}

/// Color → zsh `%F{...}` foreground escape (for the transient prompt).
pub(super) fn zsh_fg(c: &Color) -> String {
    match c {
        Color::Default => "%f".into(),
        Color::Xterm(n) => format!("%F{{{n}}}"),
        Color::Rgb(r, g, b) => format!("%F{{#{r:02x}{g:02x}{b:02x}}}"),
        Color::Named(n) => format!("%F{{{}}}", named_256(n)),
    }
}

/// Colors attach text: fg defaults to the segment style, when set it overrides the foreground
/// (bg/bold still inherit the segment style).
pub(super) fn paint_attach(style: &Style, a: &AttachText) -> String {
    let mut st = style.clone();
    if let Some(fg) = &a.fg {
        st.fg = fg.clone();
    }
    paint(&a.text, &st)
}

/// Colors text with a style (foreground + background block + bold; plain text, no separators).
pub(super) fn paint(text: &str, style: &Style) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = ansi_style(style);
    s.push_str(text);
    s.push_str("\x1b[0m");
    s
}

/// bash prompt colouring: the SGR run and the reset are wrapped in `\[...\]` so readline
/// counts them as zero width instead of shifting the buffer.
pub(super) fn paint_bash(text: &str, style: &Style) -> String {
    if text.is_empty() {
        return String::new();
    }
    let sgr = ansi_style(style);
    if sgr.is_empty() {
        return text.to_string();
    }
    format!("\\[{sgr}\\]{text}\\[\\e[0m\\]")
}

/// Maps the standard 16 color names to 256-color indexes.
pub(super) fn named_256(name: &str) -> u8 {
    match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "brightblack" => 8,
        "brightred" => 9,
        "brightgreen" => 10,
        "brightyellow" => 11,
        "brightblue" => 12,
        "brightmagenta" => 13,
        "brightcyan" => 14,
        "brightwhite" => 15,
        _ => 7,
    }
}

/// ANSI SGR sequence that turns on the given style; empty when the style asks for nothing.
pub(super) fn ansi_style(style: &Style) -> String {
    let mut s = String::new();
    match &style.fg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(n)).unwrap(),
    }
    match &style.bg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(n)).unwrap(),
    }
    if style.bold {
        s.push_str("\x1b[1m");
    }
    s
}
