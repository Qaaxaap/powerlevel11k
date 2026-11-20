use std::collections::BTreeMap;
use std::fmt;

use crate::i18n::{t, t_args};
use kdl::KdlValue;

use super::parse_helpers::{bool_val, merge_style, warn_unknown_key};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Xterm(u8),
    Rgb(u8, u8, u8),
    Named(String),
}

impl Color {
    /// The 16 colour names p10k accepts, in palette order.
    const NAMES: [&'static str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "brightblack",
        "brightred",
        "brightgreen",
        "brightyellow",
        "brightblue",
        "brightmagenta",
        "brightcyan",
        "brightwhite",
    ];

    /// Parse one colour value. `key` names the property in the error.
    fn from_str(s: &str, key: &str) -> Result<Color, String> {
        if let Some(hex) = s.strip_prefix('#') {
            let valid = hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit());
            if valid
                && let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                )
            {
                return Ok(Color::Rgb(r, g, b));
            }
            return Err(t_args(
                "config-invalid-hex-colour",
                &[("key", key), ("value", s)],
            ));
        }
        if s == "default" {
            return Ok(Color::Default);
        }
        if Self::NAMES.contains(&s) {
            return Ok(Color::Named(s.to_string()));
        }
        let help = t("config-colour-help");
        Err(t_args(
            "config-unknown-colour-name",
            &[("key", key), ("name", s), ("help", &help)],
        ))
    }

    /// Parse one colour value; `key` names the property in the error.
    pub(super) fn from_value(v: &KdlValue, key: &str) -> Result<Color, String> {
        match v {
            KdlValue::String(s) => Color::from_str(s, key),
            KdlValue::Integer(n) => u8::try_from(*n).map(Color::Xterm).map_err(|_| {
                let index = n.to_string();
                t_args(
                    "config-colour-index-out-of-range",
                    &[("key", key), ("index", &index)],
                )
            }),
            _ => Err(t_args("config-expected-colour", &[("key", key)])),
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Default => write!(f, "default"),
            Color::Xterm(n) => write!(f, "{n}"),
            Color::Rgb(r, g, b) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            Color::Named(n) => write!(f, "{n}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// `None` = not set here (inherit); `Some(Color::Default)` = explicitly transparent.
    pub fg: Option<Color>,
    /// `None` = not set here (inherit); `Some(Color::Default)` = explicitly transparent.
    pub bg: Option<Color>,
    /// `None` = not set here (inherit); `Some(false)` = explicitly not bold.
    pub bold: Option<bool>,
}

impl Style {
    /// The foreground a renderer uses: unset and explicit default both draw the terminal default.
    pub fn fg_color(&self) -> Color {
        self.fg.clone().unwrap_or(Color::Default)
    }

    /// The background a renderer uses: unset and explicit default are both transparent.
    pub fn bg_color(&self) -> Color {
        self.bg.clone().unwrap_or(Color::Default)
    }

    /// Whether the renderer emits bold.
    pub fn bold_or_false(&self) -> bool {
        self.bold.unwrap_or(false)
    }

    /// Whether a background is actually drawn: unset and explicit default are both transparent.
    pub fn has_bg(&self) -> bool {
        self.bg.as_ref().is_some_and(|c| *c != Color::Default)
    }

    /// Style of a node whose properties are all style properties (`defaults`, `frame` and its
    /// pieces): anything else is a typo, so report it.
    pub(super) fn from_style_entries(entries: &[kdl::KdlEntry]) -> Result<Style, String> {
        Style::parse(entries, true)
    }

    /// Style of a node that also carries non-style properties (a segment, a `state`): only
    /// fg/bg/bold are style here and the remaining names belong to the caller, so they are
    /// picked up by its own loop instead of being reported as unknown.
    pub(super) fn from_mixed_entries(entries: &[kdl::KdlEntry]) -> Result<Style, String> {
        Style::parse(entries, false)
    }

    fn parse(entries: &[kdl::KdlEntry], warn: bool) -> Result<Style, String> {
        let mut s = Style::default();
        for e in entries {
            let Some(name) = e.name() else { continue };
            match name.value() {
                "fg" => s.fg = Some(Color::from_value(e.value(), "fg")?),
                "bg" => s.bg = Some(Color::from_value(e.value(), "bg")?),
                "bold" => s.bold = Some(bool_val(e.value())),
                _ if warn => warn_unknown_key(name.value()),
                _ => {}
            }
        }
        Ok(s)
    }
}

/// One layout element (a segment or static text within a line).
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// Plain segment.
    Seg(String),
    /// Joins the background with the adjacent segment (p10k's `<seg>_joined` suffix).
    Joined(String),
    /// Static text (e.g. `text "some text"`): emitted verbatim, never interpreted as a segment.
    Text(String),
    /// A named node with a string value, e.g. `dir "some text"`. It renders like `text` but
    /// keeps its name, so `to_kdl` writes the same shape back.
    NamedText(String, String),
    /// A commented-out segment (`dir #false`): kept so `to_kdl` does not drop it, never drawn.
    Disabled(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub left: Vec<Vec<Element>>,
    pub right: Vec<Vec<Element>>,
    /// Sparse layout: how many blank lines to leave between consecutive prompts (0 = compact).
    /// Aligns with p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE` + `_COUNT`.
    pub prompt_add_newline: usize,
    /// Transient prompt: after a command is submitted, collapse the multi-line header into a
    /// single `❯` line (p10k `TRANSIENT_PROMPT`). zsh only (it needs zle reset-prompt); bash/fish
    /// ignore it.
    pub transient_prompt: TransientPrompt,
    /// Ruler: draw a full line of the `ruler` character above the header (p10k `SHOW_RULER`, off by default).
    pub show_ruler: bool,
    /// Columns kept between the right column and the right edge (zsh `ZLE_RPROMPT_INDENT`,
    /// default 1; drawing flush against the edge takes the last cell and reports "it fits"
    /// one column earlier than p10k).
    pub right_indent: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            left: Vec::new(),
            right: Vec::new(),
            prompt_add_newline: 0,
            transient_prompt: TransientPrompt::Off,
            show_ruler: false,
            right_indent: 1,
        }
    }
}

/// When to fold the header into a single line after a command is submitted (p10k
/// `POWERLEVEL9K_TRANSIENT_PROMPT`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransientPrompt {
    /// Never fold (`off`, the default).
    #[default]
    Off,
    /// Fold after every command (`always`).
    Always,
    /// Fold only when the command left the shell in the same directory (`same-dir`).
    SameDir,
}

impl TransientPrompt {
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "always" => Self::Always,
            "same-dir" => Self::SameDir,
            _ => Self::Off,
        }
    }
    /// Whether the fold can ever happen; false lets the engine skip the whole handshake.
    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

/// Separator/end family (configurable characters, powerline style).
///
/// - `segment`: arrow between segments with different backgrounds (e.g. powerline ``). Empty = draw nothing (plain space).
/// - `sub`: thin line between segments sharing a background (e.g. ``).
/// - `end`: end symbol at the end of the left column (right triangle ``, pointing forward/to the end of the line).
/// - `left_tail`: start symbol before the first segment of the left column (p10k `LEFT_PROMPT_FIRST_SEGMENT_START_SYMBOL`),
///   left triangle ``, drawn before the leftmost segment.
/// - `right_tail`: end symbol after the last segment of the right column (p10k `RIGHT_PROMPT_LAST_SEGMENT_END_SYMBOL`),
///   right triangle ``, drawn after the rightmost segment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Separators {
    pub segment: String,
    pub sub: String,
    pub end: String,
    /// Start symbol at the head of a right-column line (left triangle, e.g. ``).
    pub right_start: String,
    /// Arrow between right-column segments with **different** colors (left triangle, e.g. ``).
    pub right_segment: String,
    /// Inner thin line between right-column segments sharing a color (e.g. ``).
    pub right_sub: String,
    pub left_tail: String,
    pub right_tail: String,
    /// Character filling the gap between the left and right columns on a line (e.g. `·`; empty = a space).
    pub gap: String,
    /// Foreground of the gap filler (`separators gap-foreground=240`; absent = terminal default).
    /// Aligns with p10k `POWERLEVEL9K_MULTILINE_FIRST_PROMPT_GAP_FOREGROUND`.
    pub gap_foreground: Option<Color>,
    /// Foreground of the thin line between same-background segments (`sub-foreground=246`;
    /// absent = follow the following segment's style). Aligns with the color embedded in p10k's
    /// `LEFT_SUBSEGMENT_SEPARATOR` (p10k's `sep_color`).
    pub sub_foreground: Option<Color>,
    /// Foreground of the right-column same-background thin line (aligns with p10k `RIGHT_SUBSEGMENT_SEPARATOR`).
    pub right_sub_foreground: Option<Color>,
}

/// One frame piece (a line-head/line-tail decoration character): text + optional standalone style.
/// With no style it falls back to frame level → defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FramePiece {
    pub text: String,
    pub style: Option<Style>,
}

/// Multi-line frame (line-head/line-tail decoration with configurable characters and colors).
/// See p10k MULTILINE_*_PROMPT_PREFIX/SUFFIX.
/// - `first_*`: first header line (`╭─`/`─╮`); `newline_*`: middle header lines (`├─`/`─┤`);
///   `last_*`: input line (`╰─`/`─╯`). Empty = draw nothing.
/// - Colors: `style` (frame node properties `fg`/`bg`/`bold`) is the whole-frame default; an
///   individual piece may override it with its own style properties. Colors configured nowhere
///   fall back to defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Frame-level default style.
    pub style: Style,
    pub first_prefix: FramePiece,
    pub first_suffix: FramePiece,
    pub newline_prefix: FramePiece,
    pub newline_suffix: FramePiece,
    pub last_prefix: FramePiece,
    pub last_suffix: FramePiece,
}

/// Behavior property value (typed, not a string).
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Override for a single state: style + optional character (used by character-rendered segments such as prompt_char).
#[derive(Clone, Debug, PartialEq)]
pub struct StateSpec {
    pub style: Style,
    /// Character the segment shows in this state (e.g. prompt_char's ERROR state); None = keep the segment default.
    pub char: Option<String>,
}

/// Attached text slot on a segment (left/middle/right): concatenated for display only, never a block of its own.
/// With `fg` as `None` the segment style is kept; when set it overrides the foreground
/// (background/bold still follow the segment style).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttachText {
    pub text: String,
    pub fg: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Segment default style.
    pub style: Style,
    /// state overrides (name -> override, e.g. dir's `SHORTENED`/`ANCHOR`).
    pub states: BTreeMap<String, StateSpec>,
    /// Content text (optional; generated by the segment render function when absent).
    pub content: Option<String>,
    /// Text attached to the segment's left edge (before the icon).
    pub text_left: Option<AttachText>,
    /// Text attached in the middle of the segment (between icon and content; rendered only when the segment has both an icon and text).
    pub text_middle: Option<AttachText>,
    /// Text attached to the segment's right edge (after the content).
    pub text_right: Option<AttachText>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// Whether the segment is shown (`disabled` sets it false).
    pub shown: bool,
    /// Other behavior properties (e.g. `shorten-dir-length`, `threshold-seconds`), read on demand by the segment render function.
    pub props: BTreeMap<String, Prop>,
}

impl Default for Segment {
    /// Segments are shown by default (`shown=true`), matching the `parse` default.
    fn default() -> Self {
        Segment {
            style: Style::default(),
            states: BTreeMap::new(),
            content: None,
            text_left: None,
            text_middle: None,
            text_right: None,
            prefix: None,
            suffix: None,
            shown: true,
            props: BTreeMap::new(),
        }
    }
}

impl Segment {
    /// Style for a state (or the segment default), via the segment STATE → segment → global three-way fallback.
    pub fn effective_style(&self, state: Option<&str>, globals: &Style) -> Style {
        // Previously a state hit merged only (state, segment), dropping `defaults`, so
        // state-carrying pieces (dir's ANCHOR/SHORTENED etc.) never got the global background.
        let base = merge_style(&self.style, globals);
        match state.and_then(|st| self.states.get(st)) {
            Some(spec) => merge_style(&spec.style, &base),
            None => base,
        }
    }

    /// Character the segment renders in the current state: state.char → segment `char` property → `default_char`.
    pub fn char_for<'a>(&'a self, state: Option<&str>, default_char: &'a str) -> &'a str {
        if let Some(st) = state
            && let Some(spec) = self.states.get(st)
            && let Some(c) = &spec.char
        {
            return c;
        }
        match &self.props.get("char") {
            Some(Prop::Str(c)) => c,
            _ => default_char,
        }
    }

    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.get(name)
    }
}

/// Icon/font mode. Selects the character set used by built-in segment icons.
/// `nerdfont-complete` and `nerdfont-fontconfig` share the same Nerd Font glyphs;
/// `compatible` uses standard Unicode + Powerline fonts and needs no Nerd Font;
/// `ascii` is pure ASCII.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum IconMode {
    /// Nerd Font (complete set). Default.
    #[default]
    NerdfontComplete,
    /// Nerd Font (fontconfig variant, same glyph code points as complete).
    NerdfontFontconfig,
    /// Compatible mode: standard Unicode + Powerline fonts, no Nerd Font needed.
    Compatible,
    /// Pure ASCII.
    Ascii,
}

impl IconMode {
    pub(super) fn from_str(s: &str) -> IconMode {
        match s {
            "ascii" => IconMode::Ascii,
            "compatible" => IconMode::Compatible,
            "nerdfont-fontconfig" => IconMode::NerdfontFontconfig,
            _ => IconMode::NerdfontComplete, // unknown or unset is treated as nerdfont-complete
        }
    }

    /// The mode name used in the config file and in diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            IconMode::NerdfontComplete => "nerdfont-complete",
            IconMode::NerdfontFontconfig => "nerdfont-fontconfig",
            IconMode::Compatible => "compatible",
            IconMode::Ascii => "ascii",
        }
    }

    /// How much of the Unicode range the mode can display. The two Nerd Font variants share
    /// the same glyph code points, so they rank equal.
    pub(super) fn rank(&self) -> u8 {
        match self {
            IconMode::Ascii => 0,
            IconMode::Compatible => 1,
            IconMode::NerdfontFontconfig | IconMode::NerdfontComplete => 2,
        }
    }

    /// The more restrictive of two modes. Equal ranks keep `self`, so a declared mode is not
    /// replaced by an equally capable one.
    pub(super) fn min(self, other: IconMode) -> IconMode {
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }
}

/// Override for one icon name in the top-level `icon {}`.
///
/// Field semantics:
/// - `all`: once the engine classifies the character, it enters every mode that can display
///   it — NF private-use characters enter only nf, standard Unicode enters nf + compat, and
///   pure ASCII enters all three.
/// - `nf` / `compat` / `ascii`: override the corresponding mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconOverride {
    pub all: Option<String>,
    pub nf: Option<String>,
    pub compat: Option<String>,
    pub ascii: Option<String>,
}
/// Space between an icon and the content beside it (p10k `POWERLEVEL9K_ICON_PADDING`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconPadding {
    /// One space. p10k `none`, and its default.
    #[default]
    None,
    /// Two spaces. p10k `moderate`.
    Moderate,
}

impl IconPadding {
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "moderate" => Self::Moderate,
            _ => Self::None,
        }
    }
    /// The whitespace that goes between the icon and the content in the left column; the right
    /// column mirrors it.
    pub fn spaces(self) -> &'static str {
        match self {
            Self::None => " ",
            Self::Moderate => "  ",
        }
    }
}

/// One directory classification rule (the triple of p10k `DIR_CLASSES`).
#[derive(Clone, Debug, PartialEq)]
pub struct DirClass {
    /// glob matched against `$PWD`: `~` expands to $HOME, `*`/`?`/`[…]` do not cross `/`, `**` does.
    pub pattern: String,
    /// state name the dir segment uses on a match (colored by `state <name>`).
    pub state: String,
    /// Icon character for the dir segment on a match; empty = use the default folder icon.
    pub icon: String,
}
