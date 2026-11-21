//! Prompt prefix, styles and ANSI painting.
//!
//! The input-line prefix is `frame.last_prefix` plus `prompt_char`, painted for the current
//! state. The header uses the same paint helpers; the prefix helpers also serve the shell-side
//! protocol (`prefix_text_zsh`, `prefix_text_bash`, `transient_prompt_zsh`).

use std::fmt::Write as _;

use crate::config::{AttachText, Color, Config, Style};
use crate::model::{HeaderInfo, ViMode};

/// Input row left part: `frame.last-prefix` + `prompt_char` content, already coloured. The tests
/// pin the shape the theme contributes to the row the user types on.
#[cfg(test)]
pub struct InputPrefix {
    pub text: String,
}

/// How a shell prompt string expresses colour.
///
/// The renderer paints ANSI, which the engine writes straight to the terminal. A prompt string
/// is different: zle and readline count an unmarked escape sequence as visible text, so each
/// shell needs its own zero-width syntax around it. The input row and the prompt prefix travel
/// inside that string, so their ANSI is rewritten before it reaches the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptSyntax {
    /// ANSI as written: fish and pwsh read it directly.
    Ansi,
    /// zsh prompt escapes: `%F{n}` / `%K{n}` / `%B` for style, `%{…%}` for other sequences.
    Zsh,
    /// bash prompt escapes: `\[…\]` around every sequence that takes no columns.
    Bash,
}

impl PromptSyntax {
    /// Rewrite one ANSI-rendered fragment into the shell's prompt syntax.
    pub(crate) fn rewrite(self, text: &str) -> String {
        if self == PromptSyntax::Ansi {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len() + 16);
        let mut visible_from = 0;
        let mut i = 0;
        while i < text.len() {
            let b = text.as_bytes()[i];
            if b != 0x1b {
                // Only ESC is interesting; advance over one whole character so the visible
                // slice below always starts and ends on character boundaries.
                i += text[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            push_prompt_text(&mut out, &text[visible_from..i], self);
            let end = escape_end(text, i);
            push_prompt_escape(&mut out, &text[i..end], self);
            i = end;
            visible_from = end;
        }
        push_prompt_text(&mut out, &text[visible_from..], self);
        out
    }
}

/// End of the escape sequence that starts at `start` (an ESC byte). An unterminated sequence
/// takes the rest of the string: the fragment then loses its style rather than leaking bytes.
fn escape_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    match bytes.get(start + 1) {
        // CSI: ends at the first byte in the final range.
        Some(b'[') => {
            let mut i = start + 2;
            while i < bytes.len() {
                if (0x40..=0x7e).contains(&bytes[i]) {
                    return i + 1;
                }
                i += 1;
            }
            bytes.len()
        }
        // OSC: ends at BEL or ST. SGR never contains one, but an OSC 8 hyperlink wraps text
        // between two of them, so both forms occur in one fragment.
        Some(b']') => {
            let mut i = start + 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    return i + 1;
                }
                if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                    return i + 2;
                }
                i += 1;
            }
            bytes.len()
        }
        Some(_) => (start + 2).min(bytes.len()),
        None => bytes.len(),
    }
}

/// Append visible text, escaping the characters the shell would otherwise interpret.
fn push_prompt_text(out: &mut String, text: &str, syntax: PromptSyntax) {
    match syntax {
        PromptSyntax::Ansi => out.push_str(text),
        // zle expands `%` in the prompt string; a literal percent needs `%%`.
        PromptSyntax::Zsh => out.push_str(&text.replace('%', "%%")),
        // readline expands `\` in PS1; a literal backslash needs `\\`. The template turns
        // `promptvars` off, so a dollar sign and a backtick stay literal.
        PromptSyntax::Bash => out.push_str(&text.replace('\\', "\\\\")),
    }
}

/// Append one escape sequence in the shell's syntax.
fn push_prompt_escape(out: &mut String, seq: &str, syntax: PromptSyntax) {
    match syntax {
        PromptSyntax::Ansi => out.push_str(seq),
        PromptSyntax::Zsh => match sgr_params(seq) {
            Some(params) => out.push_str(&zsh_sgr(&params)),
            // Not SGR: mark it zero width and keep its bytes. The prompt-ready signal must
            // stay byte-identical, and an OSC 8 hyperlink is opened by one sequence and
            // closed by another. A percent inside `%{…%}` is still expanded by zle, so it is
            // doubled and comes out single.
            None => {
                out.push_str("%{");
                out.push_str(&seq.replace('%', "%%"));
                out.push_str("%}");
            }
        },
        PromptSyntax::Bash => {
            out.push_str("\\[");
            out.push_str(seq);
            out.push_str("\\]");
        }
    }
}

/// The parameters of an SGR sequence (`ESC [ … m`), or `None` for any other sequence.
fn sgr_params(seq: &str) -> Option<Vec<&str>> {
    let body = seq.strip_prefix("\x1b[")?.strip_suffix('m')?;
    Some(body.split(';').collect())
}

/// Translate SGR parameters into zsh prompt escapes. Parameters p11k does not emit are kept as
/// zero-width sequences, so an unknown style degrades instead of disappearing.
fn zsh_sgr(params: &[&str]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < params.len() {
        match params[i] {
            "" | "0" => {
                out.push_str("%f%k%b");
                i += 1;
            }
            "1" => {
                out.push_str("%B");
                i += 1;
            }
            "38" | "48" => {
                i += zsh_colour(&params[i..], params[i] == "38", &mut out);
            }
            other => {
                out.push_str(&format!("%{{\x1b[{other}m%}}"));
                i += 1;
            }
        }
    }
    out
}

/// Translate one foreground/background colour parameter block, starting at `38` or `48`.
/// Returns how many parameters it consumed.
fn zsh_colour(params: &[&str], fg: bool, out: &mut String) -> usize {
    let slot = if fg { 'F' } else { 'K' };
    match (params.get(1), params.get(2)) {
        (Some(&"5"), Some(n)) => {
            out.push_str(&format!("%{slot}{{{n}}}"));
            3
        }
        (Some(&"2"), _) => {
            let hex = match (params.get(2), params.get(3), params.get(4)) {
                (Some(r), Some(g), Some(b)) => rgb_hex(r, g, b),
                _ => None,
            };
            if let Some(hex) = hex {
                out.push_str(&format!("%{slot}{{{hex}}}"));
                5
            } else {
                2
            }
        }
        // A colour form the renderer does not emit: drop the block, keep the rest.
        _ => 2.min(params.len()),
    }
}

/// `r;g;b` decimal parameters → `#rrggbb`, or `None` when one of them is not a byte.
fn rgb_hex(r: &str, g: &str, b: &str) -> Option<String> {
    let (r, g, b): (u8, u8, u8) = (r.parse().ok()?, g.parse().ok()?, b.parse().ok()?);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// prompt_char state. vi mode wins (p10k
/// `PROMPT_CHAR_{OK,ERROR}_{VIINS,VICMD,VIVIS,VIOWR}`)—but only when a char is really
/// configured for that state; otherwise the exit code decides: non-zero → ERROR, else normal.
/// State for the input-line prefix: the vi mode's own state when it has one, otherwise the
/// status state. Extended states mirror p10k's `STATUS_EXTENDED_STATES`: a failing pipeline
/// is `ERROR_PIPE`, death by signal is `ERROR_SIGNAL`, and a pipeline whose last stage
/// succeeded while an earlier one failed is `OK_PIPE`.
pub fn prompt_state(
    config: &Config,
    info: Option<&HeaderInfo>,
    vi_mode: ViMode,
) -> Option<&'static str> {
    if let Some(st) = vi_mode.state() {
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

/// The left part of the row the user types on: the frame's last prefix, the row's segments and
/// the prompt character, in the syntax of one shell's prompt string.
///
/// The frame piece comes first, because it decorates the row edge the way `frame.first-prefix`
/// does on the first header row. `segments` is the rendered left column as ANSI; pass an empty
/// string when the theme declares no `input` row.
pub(crate) fn input_left(
    config: &Config,
    state: Option<&str>,
    segments: &str,
    syntax: PromptSyntax,
) -> String {
    let mut ansi = String::new();
    let piece = &config.frame.last_prefix;
    if !piece.text.is_empty() {
        ansi.push_str(&paint(&piece.text, &config.frame_piece_style(piece)));
    }
    ansi.push_str(segments);
    let pc = config.segment("prompt_char");
    let st = pc.effective_style(state, &config.defaults);
    // The locale fallback can force the ascii mode, which cannot display `❯`.
    let default_char = match config.mode {
        crate::config::IconMode::Ascii => ">",
        _ => "❯",
    };
    ansi.push_str(&paint(pc.char_for(state, default_char), &st));
    ansi.push(' '); // space after the prompt char, uncoloured
    syntax.rewrite(&ansi)
}

/// The right part of the row the user types on: its right column and the frame's last suffix.
///
/// The shell aligns this against the right edge, so the suffix lands in the rightmost cell of the
/// row, as `frame.first-suffix` does on a header row. A shell without a right prompt draws
/// neither the column nor the suffix.
pub(crate) fn input_right(config: &Config, segments: &str, syntax: PromptSyntax) -> String {
    let mut ansi = String::from(segments);
    let piece = &config.frame.last_suffix;
    if !piece.text.is_empty() {
        ansi.push_str(&paint(&piece.text, &config.frame_piece_style(piece)));
    }
    syntax.rewrite(&ansi)
}

/// The left part of the row the user types on without segments, in plain ANSI: the shape the
/// engine paints before the shell has announced a prompt.
pub fn prefix_text(config: &Config, state: Option<&str>) -> String {
    input_left(config, state, "", PromptSyntax::Ansi)
}

/// The visible text of a string that may carry SGR sequences. Used where the same text has to be
/// matched after a terminal layer stripped the colours, such as PSReadLine repainting the prompt.
pub fn strip_ansi(s: &str) -> String {
    super::ansi::strip(s)
}

/// [`prefix_text`] in zsh's prompt syntax: `%F{...}` is zero width for zle, so the prefix can
/// carry its colour without shifting the buffer. Used when the prompt string is built by the
/// shell (the engine cannot paint the prefix itself: zle draws the buffer immediately after the
/// prompt and a later paint would overwrite it).
pub fn prefix_text_zsh(config: &Config, state: Option<&str>) -> String {
    input_left(config, state, "", PromptSyntax::Zsh)
}

/// [`prefix_text`] in bash's prompt syntax: the SGR runs are wrapped in `\[...\]` so readline
/// counts them as zero width.
pub fn prefix_text_bash(config: &Config, state: Option<&str>) -> String {
    input_left(config, state, "", PromptSyntax::Bash)
}

/// Computes the left part of the row the user types on (e.g. `╰─❯ `) from the prompt state.
/// `last_prefix` (frame style) and the prompt char (prompt_char style, entering ERROR state from
/// `exit_code`) are colored; `text` is the ANSI form.
#[cfg(test)]
pub fn input_prefix(config: &Config, info: Option<&HeaderInfo>, vi_mode: ViMode) -> InputPrefix {
    InputPrefix {
        text: prefix_text(config, prompt_state(config, info, vi_mode)),
    }
}

/// Single-line prompt for transient folding (zsh format). The engine precomputes the zsh
/// prompt escapes; zsh swaps PROMPT in zle-line-finish and folds in sync via reset-prompt.
/// Not easily portable to bash/fish, hence the zsh-only approach.
/// Uses native `%F{...}` escapes;
/// the conditional `%(?\x01OK\x01ERR)` picks a color from the last command's exit code.
pub fn transient_prompt_zsh(config: &Config) -> String {
    let pc = config.segment("prompt_char");
    // The locale fallback can force the ascii mode, which cannot display `❯`.
    let default_char = match config.mode {
        crate::config::IconMode::Ascii => ">",
        _ => "❯",
    };
    let ch = pc.char_for(None, default_char);
    let ok = pc.effective_style(None, &config.defaults);
    let err = pc.effective_style(Some("ERROR"), &config.defaults);
    let ok_prompt = format!("{}{} ", zsh_fg(&ok.fg_color()), ch);
    let err_prompt = format!("{}{} ", zsh_fg(&err.fg_color()), ch);
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
        st.fg = Some(fg.clone());
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
    match &style.fg_color() {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(n)).unwrap(),
    }
    match &style.bg_color() {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(n)).unwrap(),
    }
    if style.bold_or_false() {
        s.push_str("\x1b[1m");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header is painted as ANSI; a prompt string needs the shell's own zero-width syntax,
    /// or zle and readline count the escape bytes as text and the cursor lands in the wrong
    /// column.
    #[test]
    fn zsh_prompt_syntax_replaces_sgr_with_zero_width_escapes() {
        assert_eq!(
            PromptSyntax::Zsh.rewrite("\x1b[38;5;196mred\x1b[0m"),
            "%F{196}red%f%k%b"
        );
        assert_eq!(
            PromptSyntax::Zsh.rewrite("\x1b[48;5;238m\x1b[1mX\x1b[0m"),
            "%K{238}%BX%f%k%b"
        );
        assert_eq!(
            PromptSyntax::Zsh.rewrite("\x1b[38;2;255;92;87mrgb\x1b[0m"),
            "%F{#ff5c57}rgb%f%k%b"
        );
    }

    #[test]
    fn bash_prompt_syntax_brackets_every_sequence() {
        assert_eq!(
            PromptSyntax::Bash.rewrite("\x1b[38;5;196mred\x1b[0m"),
            "\\[\x1b[38;5;196m\\]red\\[\x1b[0m\\]"
        );
    }

    /// fish and pwsh read the ANSI the renderer produced, so nothing changes for them.
    #[test]
    fn ansi_prompt_syntax_is_unchanged() {
        let text = "\x1b[38;5;196mred\x1b[0m";
        assert_eq!(PromptSyntax::Ansi.rewrite(text), text);
    }

    /// Every shell expands one character in its prompt string: `%` in zsh, `\` in bash. Text
    /// from a segment must survive that expansion unchanged.
    #[test]
    fn visible_metacharacters_are_escaped_per_shell() {
        assert_eq!(PromptSyntax::Zsh.rewrite("100%"), "100%%");
        assert_eq!(PromptSyntax::Zsh.rewrite("a\\b"), "a\\b");
        assert_eq!(PromptSyntax::Bash.rewrite("a\\b"), "a\\\\b");
        assert_eq!(PromptSyntax::Bash.rewrite("100%"), "100%");
    }

    /// A hyperlink is an OSC 8 pair around the text. The sequence is escape, the text is not,
    /// and a percent inside the URL is still expanded by zle, so it is doubled.
    #[test]
    fn osc8_hyperlinks_stay_escape_in_prompt_syntax() {
        let link = "\x1b]8;;file:///tmp/a%20b\x07txt\x1b]8;;\x07";
        assert_eq!(
            PromptSyntax::Zsh.rewrite(link),
            "%{\x1b]8;;file:///tmp/a%%20b\x07%}txt%{\x1b]8;;\x07%}"
        );
        assert_eq!(
            PromptSyntax::Bash.rewrite(link),
            "\\[\x1b]8;;file:///tmp/a%20b\x07\\]txt\\[\x1b]8;;\x07\\]"
        );
    }

    /// The prompt-ready signal travels inside the prefix and must keep its exact bytes.
    #[test]
    fn the_prompt_signal_keeps_its_bytes() {
        let signal = "\x1b]1337;p11k;p\x07";
        assert_eq!(PromptSyntax::Zsh.rewrite(signal), format!("%{{{signal}%}}"));
        assert_eq!(
            PromptSyntax::Bash.rewrite(signal),
            format!("\\[{signal}\\]")
        );
    }

    /// An SGR the translator does not know is kept zero width instead of being dropped.
    #[test]
    fn unknown_sgr_stays_zero_width() {
        assert_eq!(
            PromptSyntax::Zsh.rewrite("\x1b[4munder\x1b[0m"),
            "%{\x1b[4m%}under%f%k%b"
        );
    }
}
