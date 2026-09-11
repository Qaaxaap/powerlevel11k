//! Theme configuration: KDL parsing + the built-in lean theme.
//!
//! Parsed with the [`kdl`](kdl-rs, the official KDL reference implementation) crate, using KDL v2.
//!
//! Layout expresses multiple lines through line structure:
//! - Layout: `layout { left { line {…} line {…} } right { line {…} } }`.
//!   Each `line` node under `left`/`right` is one line; inside a line are segment nodes
//!   (`dir #true` etc., booleans native; `#false`/omitted means disabled, order = children
//!   order); line breaks sit between the lines.
//! - Segments: `segments { dir fg=39 bold=#true shorten-strategy="t" … }`.
//!   Segment nodes carry the properties `fg`, `bg`, `bold`,
//!   `content`/`icon`/`prefix`/`suffix`, `disabled`; the rest go into
//!   [`Segment::props`] (behavior, read on demand by the segment render function).
//! - state overrides: `state <NAME> fg=…` child nodes under a segment node; their style
//!   overrides the segment default, i.e. the p10k `SEG[_STATE]_ATTR` three-way fallback
//!   (segment STATE → segment → [`Config::defaults`] global fallback).
//! - Defaults: the top-level `defaults { … }` is the global fallback style.
//!
//! Segment behavior is implemented in code; appearance/layout is entirely config-driven, so
//! swapping the config swaps the theme, with no hard-coded visuals.

use std::collections::BTreeMap;
use std::fmt;

use crate::i18n::t;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

/// Color: a number = 256-color palette, `#rrggbb` = 24-bit, `"default"`/absent = inherit the terminal.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Xterm(u8),
    Rgb(u8, u8, u8),
    Named(String),
}

impl Color {
    fn from_str(s: &str) -> Color {
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() == 6
                && let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                )
            {
                return Color::Rgb(r, g, b);
            }
            Color::Named(s.to_string())
        } else if s == "default" {
            Color::Default
        } else {
            Color::Named(s.to_string())
        }
    }

    fn from_value(v: &KdlValue) -> Color {
        match v {
            KdlValue::String(s) => Color::from_str(s),
            KdlValue::Integer(n) => Color::Xterm((*n).clamp(0, 255) as u8),
            _ => Color::Default,
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
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Style {
    fn from_entries(entries: &[kdl::KdlEntry]) -> Style {
        let mut s = Style::default();
        for e in entries {
            let Some(name) = e.name() else { continue };
            match name.value() {
                "fg" => s.fg = Color::from_value(e.value()),
                "bg" => s.bg = Color::from_value(e.value()),
                "bold" => s.bold = bool_val(e.value()),
                _ => warn_unknown_key(name.value()),
            }
        }
        s
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub left: Vec<Vec<Element>>,
    pub right: Vec<Vec<Element>>,
    /// Sparse layout: how many blank lines to leave between consecutive prompts (0 = compact).
    /// Aligns with p10k `POWERLEVEL9K_PROMPT_ADD_NEWLINE` + `_COUNT`.
    pub prompt_add_newline: usize,
    /// Transient prompt: after a command is submitted, collapse the multi-line header into a
    /// single `❯` line (p10k `TRANSIENT_PROMPT`). zsh only (it needs zle reset-prompt); bash/fish ignore it.
    pub transient_prompt: bool,
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
            transient_prompt: false,
            show_ruler: false,
            right_indent: 1,
        }
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
    fn from_str(s: &str) -> IconMode {
        match s {
            "ascii" => IconMode::Ascii,
            "compatible" => IconMode::Compatible,
            "nerdfont-fontconfig" => IconMode::NerdfontFontconfig,
            _ => IconMode::NerdfontComplete, // unknown or unset is treated as nerdfont-complete
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

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub layout: Layout,
    pub segments: BTreeMap<String, Segment>,
    /// Global fallback style.
    pub defaults: Style,
    /// Separator/end family.
    pub separators: Separators,
    /// Multi-line frame.
    pub frame: Frame,
    /// vcs icon selection by remote domain (domain substring → icon character); matched in order, git's default when nothing matches.
    pub vcs_remote_icons: Vec<(String, String)>,
    /// Icon/font mode.
    pub mode: IconMode,
    /// Top-level `icon{}` override table (icon name → override; looked up by the icon names a segment references).
    pub icon_overrides: BTreeMap<String, IconOverride>,
    /// `dir-classes { class "pattern" state="WORK" icon="…" }`:
    /// switch the dir segment's state / icon by path pattern (aligns with p10k `DIR_CLASSES`).
    pub dir_classes: Vec<DirClass>,
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

impl Default for Config {
    /// Empty config: `vcs_remote_icons` uses the built-in default table (matching the `parse` default).
    fn default() -> Self {
        Config {
            layout: Layout::default(),
            segments: BTreeMap::new(),
            defaults: Style::default(),
            separators: Separators::default(),
            frame: Frame::default(),
            vcs_remote_icons: default_remote_icons(),
            mode: IconMode::NerdfontComplete,
            icon_overrides: BTreeMap::new(),
            dir_classes: Vec::new(),
        }
    }
}

impl Config {
    /// Parse KDL text into a config.
    pub fn parse(src: &str) -> Result<Config, String> {
        let doc = KdlDocument::parse(src).map_err(|e| format!("{}{e}", t("KDL parse error: ")))?;
        Self::parse_doc(&doc)
    }

    /// Built-in lean theme.
    pub fn default_lean() -> Result<Config, String> {
        Ok(crate::presets::build(crate::presets::PresetKind::Lean))
    }

    /// Serialize to KDL text; `parse(to_kdl(cfg))` must yield an equivalent config (round-trip).
    /// Lay it out with kdl-rs autoformat (indentation/spacing), otherwise the constructed
    /// document is packed tight; then switch string values to their quoted representation
    /// (see [`quote_string_values`]).
    pub fn to_kdl(&self) -> String {
        let mut doc = self.to_document();
        doc.autoformat();
        quote_string_values(&mut doc);
        doc.to_string()
    }

    /// Build the equivalent KDL document. Node order is mode/layout/defaults/separators/frame/
    /// segments/vcs-remote-icons/icon; fields equal to the `parse` default are omitted.
    pub fn to_document(&self) -> KdlDocument {
        let mut doc = KdlDocument::new();
        // mode must be emitted explicitly: the parse default follows the locale, so omitting it breaks round-trip.
        doc.nodes_mut().push(leaf("mode", mode_str(&self.mode)));
        doc.nodes_mut().push(self.layout_node());
        if self.defaults != Style::default() {
            let mut n = KdlNode::new("defaults");
            n.entries_mut().extend(style_entries(&self.defaults));
            doc.nodes_mut().push(n);
        }
        if self.separators != Separators::default() {
            doc.nodes_mut().push(self.separators_node());
        }
        if self.frame != Frame::default() {
            doc.nodes_mut().push(self.frame_node());
        }
        if !self.segments.is_empty() {
            let mut n = KdlNode::new("segments");
            let ch = n.ensure_children();
            for (name, seg) in &self.segments {
                ch.nodes_mut().push(segment_node(name, seg));
            }
            doc.nodes_mut().push(n);
        }
        // When vcs-remote-icons is omitted, parse uses the built-in default table, equivalent to the default value.
        if self.vcs_remote_icons != default_remote_icons() {
            doc.nodes_mut().push(self.remote_icons_node());
        }
        if !self.icon_overrides.is_empty() {
            doc.nodes_mut().push(self.icon_node());
        }
        if !self.dir_classes.is_empty() {
            let mut n = KdlNode::new("dir-classes");
            let ch = n.ensure_children();
            for c in &self.dir_classes {
                let mut cn = KdlNode::new("class");
                cn.entries_mut()
                    .push(KdlEntry::new(KdlValue::String(c.pattern.clone())));
                if !c.state.is_empty() {
                    cn.entries_mut().push(KdlEntry::new_prop(
                        "state",
                        KdlValue::String(c.state.clone()),
                    ));
                }
                if !c.icon.is_empty() {
                    cn.entries_mut()
                        .push(KdlEntry::new_prop("icon", KdlValue::String(c.icon.clone())));
                }
                ch.nodes_mut().push(cn);
            }
            doc.nodes_mut().push(n);
        }
        doc
    }

    fn layout_node(&self) -> KdlNode {
        let mut n = KdlNode::new("layout");
        let ch = n.ensure_children();
        let mut left = KdlNode::new("left");
        for row in &self.layout.left {
            left.ensure_children().nodes_mut().push(line_node(row));
        }
        ch.nodes_mut().push(left);
        let mut right = KdlNode::new("right");
        for row in &self.layout.right {
            right.ensure_children().nodes_mut().push(line_node(row));
        }
        ch.nodes_mut().push(right);
        if self.layout.prompt_add_newline == 1 {
            ch.nodes_mut().push(leaf("prompt-add-newline", true));
        } else if self.layout.prompt_add_newline > 1 {
            ch.nodes_mut().push(leaf(
                "prompt-add-newline",
                self.layout.prompt_add_newline as i128,
            ));
        }
        if self.layout.transient_prompt {
            ch.nodes_mut().push(leaf("transient-prompt", true));
        }
        if self.layout.show_ruler {
            ch.nodes_mut().push(leaf("show-ruler", true));
        }
        if self.layout.right_indent != 1 {
            ch.nodes_mut()
                .push(leaf("right-indent", self.layout.right_indent as i128));
        }
        n
    }

    fn separators_node(&self) -> KdlNode {
        let s = &self.separators;
        let mut n = KdlNode::new("separators");
        for (k, c) in [
            ("gap-foreground", &s.gap_foreground),
            ("sub-foreground", &s.sub_foreground),
            ("right-sub-foreground", &s.right_sub_foreground),
        ] {
            if let Some(c) = c {
                n.entries_mut().push(KdlEntry::new_prop(k, color_value(c)));
            }
        }
        let ch = n.ensure_children();
        for (name, val) in [
            ("segment", &s.segment),
            ("sub", &s.sub),
            ("end", &s.end),
            ("right-start", &s.right_start),
            ("right-segment", &s.right_segment),
            ("right-sub", &s.right_sub),
            ("left-tail", &s.left_tail),
            ("right-tail", &s.right_tail),
            ("gap", &s.gap),
        ] {
            if !val.is_empty() {
                ch.nodes_mut().push(leaf(name, val.clone()));
            }
        }
        n
    }

    fn frame_node(&self) -> KdlNode {
        let mut n = KdlNode::new("frame");
        n.entries_mut().extend(style_entries(&self.frame.style));
        let ch = n.ensure_children();
        for (name, piece) in [
            ("first-prefix", &self.frame.first_prefix),
            ("first-suffix", &self.frame.first_suffix),
            ("newline-prefix", &self.frame.newline_prefix),
            ("newline-suffix", &self.frame.newline_suffix),
            ("last-prefix", &self.frame.last_prefix),
            ("last-suffix", &self.frame.last_suffix),
        ] {
            if piece.text.is_empty() && piece.style.is_none() {
                continue;
            }
            let mut pn = KdlNode::new(name);
            pn.entries_mut().push(KdlEntry::new(piece.text.clone()));
            if let Some(st) = &piece.style {
                pn.entries_mut().extend(style_entries(st));
            }
            ch.nodes_mut().push(pn);
        }
        n
    }

    fn remote_icons_node(&self) -> KdlNode {
        let mut n = KdlNode::new("vcs-remote-icons");
        let ch = n.ensure_children();
        for (domain, icon) in &self.vcs_remote_icons {
            ch.nodes_mut().push(leaf(domain.as_str(), icon.clone()));
        }
        n
    }

    fn icon_node(&self) -> KdlNode {
        let mut n = KdlNode::new("icon");
        let ch = n.ensure_children();
        for (name, ov) in &self.icon_overrides {
            let mut inode = KdlNode::new(name.as_str());
            let ich = inode.ensure_children();
            for (k, v) in [
                ("all", &ov.all),
                ("nf", &ov.nf),
                ("compat", &ov.compat),
                ("ascii", &ov.ascii),
            ] {
                if let Some(val) = v {
                    ich.nodes_mut().push(leaf(k, val.clone()));
                }
            }
            ch.nodes_mut().push(inode);
        }
        n
    }

    /// Config for one segment (the default empty segment when absent).
    pub fn segment(&self, name: &str) -> &Segment {
        self.segments.get(name).unwrap_or(&EMPTY_SEG)
    }

    /// Actual style of a frame piece: piece-level properties → frame level → `defaults`.
    /// **Only `defaults`' foreground/bold are inherited, never its `bg`** — `defaults.bg` is the
    /// fallback for segment backgrounds (p10k's global `POWERLEVEL9K_BACKGROUND` for segments),
    /// while frame characters (`╭─`/`╰─`) are always transparent in p10k (foreground only).
    pub fn frame_piece_style(&self, piece: &FramePiece) -> Style {
        let base = merge_style(&self.frame.style, &self.defaults);
        let base = Style {
            bg: self.frame.style.bg.clone(),
            ..base
        };
        match &piece.style {
            Some(s) => merge_style(s, &base),
            None => base,
        }
    }

    fn parse_doc(doc: &KdlDocument) -> Result<Config, String> {
        let mut layout = Layout::default();
        let mut segments: BTreeMap<String, Segment> = BTreeMap::new();
        let mut defaults = Style::default();
        let mut separators = Separators::default();
        let mut frame = Frame::default();
        let mut vcs_remote_icons = default_remote_icons();
        // The default mode follows the locale: a non-UTF-8 terminal cannot display Unicode,
        // so it degrades to ascii; an explicit `mode` in the config wins.
        let mut mode = if locale_is_utf8() {
            IconMode::NerdfontComplete
        } else {
            IconMode::Ascii
        };
        let mut icon_overrides: BTreeMap<String, IconOverride> = BTreeMap::new();
        let mut dir_classes: Vec<DirClass> = Vec::new();

        for node in doc.nodes() {
            match node.name().value() {
                "layout" => layout = parse_layout(node)?,
                "segments" => {
                    if let Some(ch) = node.children() {
                        for seg in ch.nodes() {
                            let name = seg.name().value().to_string();
                            let parsed = parse_segment(seg)?;
                            // Multiple nodes with the same segment name (e.g. `os icon=…` and
                            // `os fg=…` on two lines) merge: the later one overrides fields,
                            // while states/props accumulate.
                            match segments.get_mut(&name) {
                                Some(existing) => merge_segment(existing, parsed),
                                None => {
                                    segments.insert(name, parsed);
                                }
                            }
                        }
                    }
                }
                "defaults" => defaults = Style::from_entries(node.entries()),
                "separators" => separators = parse_separators(node),
                "frame" => frame = parse_frame(node),
                "vcs-remote-icons" => vcs_remote_icons = parse_remote_icons(node),
                "mode" => {
                    if let Some(s) = first_value(node).and_then(str_val) {
                        mode = IconMode::from_str(&s);
                    }
                }
                "icon" => icon_overrides = parse_icon_table(node),
                "dir-classes" => dir_classes = parse_dir_classes(node),
                _ => warn_unknown_key(node.name().value()),
            }
        }
        Ok(Config {
            layout,
            segments,
            defaults,
            separators,
            frame,
            vcs_remote_icons,
            mode,
            icon_overrides,
            dir_classes,
        })
    }
}

// ---- Serialization helpers (Config → KDL) ----

/// Give every string value a quoted representation. kdl-rs leaves strings that can be bare
/// identifiers unquoted
/// (e.g. `mode nerdfont-complete`, `segment <U+E0B0>`, which looks like an empty value),
/// while p11k's presets/docs are always quoted, so this unifies them. `autoformat_keep` stops
/// autoformat from overwriting that representation (it keeps only value_repr and leading and
/// reformats the rest).
fn quote_string_values(doc: &mut KdlDocument) {
    for node in doc.nodes_mut() {
        quote_node_strings(node);
    }
}

fn quote_node_strings(node: &mut KdlNode) {
    for e in node.entries_mut() {
        if let KdlValue::String(s) = e.value() {
            let repr = quote_kdl_string(s);
            let mut fmt = e.format().cloned().unwrap_or(kdl::KdlEntryFormat {
                leading: " ".into(),
                ..Default::default()
            });
            fmt.value_repr = repr;
            fmt.autoformat_keep = true;
            e.set_format(fmt);
        }
    }
    if let Some(children) = node.children_mut() {
        quote_string_values(children);
    }
}

/// KDL v2 string literal (escape rules align with kdl-rs's `write_string`).
fn quote_kdl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Single positional value node (`<name> "value"` / `<name> #true`).
fn leaf(name: &str, value: impl Into<KdlValue>) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(value));
    n
}

fn mode_str(m: &IconMode) -> &'static str {
    match m {
        IconMode::NerdfontComplete => "nerdfont-complete",
        IconMode::NerdfontFontconfig => "nerdfont-fontconfig",
        IconMode::Compatible => "compatible",
        IconMode::Ascii => "ascii",
    }
}

fn color_value(c: &Color) -> KdlValue {
    match c {
        Color::Default => KdlValue::Null,
        Color::Xterm(n) => KdlValue::from(*n as i128),
        Color::Rgb(r, g, b) => KdlValue::from(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Named(n) => KdlValue::from(n.clone()),
    }
}

/// Style → KDL properties (fg/bg/bold, only non-default entries emitted).
fn style_entries(style: &Style) -> Vec<KdlEntry> {
    let mut v = Vec::new();
    if style.fg != Color::Default {
        v.push(KdlEntry::new_prop("fg", color_value(&style.fg)));
    }
    if style.bg != Color::Default {
        v.push(KdlEntry::new_prop("bg", color_value(&style.bg)));
    }
    if style.bold {
        v.push(KdlEntry::new_prop("bold", true));
    }
    v
}

fn prop_value(p: &Prop) -> KdlValue {
    match p {
        Prop::Str(s) => KdlValue::from(s.clone()),
        Prop::Int(n) => KdlValue::from(*n as i128),
        Prop::Float(f) => KdlValue::from(*f),
        Prop::Bool(b) => KdlValue::from(*b),
    }
}

/// One line of layout elements → a `line { … }` node.
fn line_node(row: &[Element]) -> KdlNode {
    let mut n = KdlNode::new("line");
    let ch = n.ensure_children();
    for el in row {
        match el {
            Element::Seg(name) => {
                ch.nodes_mut().push(KdlNode::new(name.as_str()));
            }
            Element::Joined(name) => {
                ch.nodes_mut().push(KdlNode::new(format!("{name}_joined")));
            }
            Element::Text(t) => {
                ch.nodes_mut().push(leaf("text", t.clone()));
            }
        }
    }
    n
}

/// One segment → a `<name> … { state … text-* … }` node.
fn segment_node(name: &str, seg: &Segment) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().extend(style_entries(&seg.style));
    if let Some(c) = &seg.content {
        n.entries_mut()
            .push(KdlEntry::new_prop("content", c.clone()));
    }
    if let Some(p) = &seg.prefix {
        n.entries_mut()
            .push(KdlEntry::new_prop("prefix", p.clone()));
    }
    if let Some(s) = &seg.suffix {
        n.entries_mut()
            .push(KdlEntry::new_prop("suffix", s.clone()));
    }
    if !seg.shown {
        n.entries_mut().push(KdlEntry::new_prop("disabled", true));
    }
    for (k, v) in &seg.props {
        n.entries_mut()
            .push(KdlEntry::new_prop(k.clone(), prop_value(v)));
    }
    if !seg.states.is_empty()
        || seg.text_left.is_some()
        || seg.text_middle.is_some()
        || seg.text_right.is_some()
    {
        let ch = n.ensure_children();
        for (sname, spec) in &seg.states {
            let mut sn = KdlNode::new("state");
            sn.entries_mut().push(KdlEntry::new(sname.clone()));
            sn.entries_mut().extend(style_entries(&spec.style));
            if let Some(c) = &spec.char {
                sn.entries_mut().push(KdlEntry::new_prop("char", c.clone()));
            }
            ch.nodes_mut().push(sn);
        }
        if let Some(t) = &seg.text_left {
            ch.nodes_mut().push(attach_node("text-left", t));
        }
        if let Some(t) = &seg.text_middle {
            ch.nodes_mut().push(attach_node("text-middle", t));
        }
        if let Some(t) = &seg.text_right {
            ch.nodes_mut().push(attach_node("text-right", t));
        }
    }
    n
}

/// Attached text slot → a `text-<side> "…" [fg=…]` node.
fn attach_node(name: &str, a: &AttachText) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(a.text.clone()));
    if let Some(fg) = &a.fg {
        n.entries_mut()
            .push(KdlEntry::new_prop("fg", color_value(fg)));
    }
    n
}

/// Parse the top-level `icon` node: each child is the override for one icon name, and the
/// override fields are its children (e.g. `ok { all "✔" ascii "V" }`, one each for all/nf/compat/ascii).
fn parse_icon_table(node: &KdlNode) -> BTreeMap<String, IconOverride> {
    let mut out = BTreeMap::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let name = child.name().value().to_string();
            let mut ov = IconOverride::default();
            if let Some(fields) = child.children() {
                for f in fields.nodes() {
                    let Some(v) = first_value(f).and_then(str_val) else {
                        continue;
                    };
                    match f.name().value() {
                        "all" => ov.all = Some(v),
                        "nf" => ov.nf = Some(v),
                        "compat" => ov.compat = Some(v),
                        "ascii" => ov.ascii = Some(v),
                        _ => warn_unknown_key(f.name().value()),
                    }
                }
            }
            out.insert(name, ov);
        }
    }
    out
}

fn default_remote_icons() -> Vec<(String, String)> {
    vec![
        ("github".into(), "\u{f113}".into()),            // 
        ("gitlab".into(), "\u{f296}".into()),            // 
        ("bitbucket".into(), "\u{f171}".into()),         // 
        ("aur.archlinux.org".into(), "\u{f303}".into()), // 
        ("archlinux.org".into(), "\u{f303}".into()),     // 
    ]
}

fn locale_is_utf8() -> bool {
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return locale_str_is_utf8(v);
            }
        }
    }
    false // nothing set → C locale → not UTF-8
}

/// Whether one locale string's codeset (e.g. `en_US.UTF-8`, `C.UTF-8`, `C`) is UTF-8.
fn locale_str_is_utf8(locale: &str) -> bool {
    let code = locale
        .rsplit('.')
        .next()
        .unwrap_or("")
        .split('@') // strip the modifier, e.g. UTF-8@euro
        .next()
        .unwrap_or("");
    code.eq_ignore_ascii_case("utf-8") || code.eq_ignore_ascii_case("utf8")
}

/// Merge multiple nodes with the same segment name: non-Default styles override, text/icons take the later value, states/props accumulate.
fn merge_segment(a: &mut Segment, b: Segment) {
    if b.style.fg != Color::Default {
        a.style.fg = b.style.fg;
    }
    if b.style.bg != Color::Default {
        a.style.bg = b.style.bg;
    }
    if b.style.bold {
        a.style.bold = true;
    }
    if b.content.is_some() {
        a.content = b.content;
    }
    if b.text_left.is_some() {
        a.text_left = b.text_left;
    }
    if b.text_middle.is_some() {
        a.text_middle = b.text_middle;
    }
    if b.text_right.is_some() {
        a.text_right = b.text_right;
    }
    if b.prefix.is_some() {
        a.prefix = b.prefix;
    }
    if b.suffix.is_some() {
        a.suffix = b.suffix;
    }
    if !b.shown {
        a.shown = false;
    }
    for (k, v) in b.states {
        a.states.insert(k, v);
    }
    for (k, v) in b.props {
        a.props.insert(k, v);
    }
}

/// Parse `dir-classes { class "pattern" state="WORK" icon="…" }`.
/// The first argument is the pattern, `state` / `icon` are node properties; matched in declaration order, first match wins.
fn parse_dir_classes(node: &KdlNode) -> Vec<DirClass> {
    let mut out = Vec::new();
    let Some(ch) = node.children() else {
        return out;
    };
    for child in ch.nodes() {
        if child.name().value() != "class" {
            continue;
        }
        let Some(KdlValue::String(pattern)) = first_value(child) else {
            continue;
        };
        let attr = |key: &str| -> String {
            child
                .entries()
                .iter()
                .find(|e| e.name().map(|n| n.value() == key).unwrap_or(false))
                .and_then(|e| e.value().as_string())
                .unwrap_or_default()
                .to_string()
        };
        out.push(DirClass {
            pattern: pattern.clone(),
            state: attr("state"),
            icon: attr("icon"),
        });
    }
    out
}

/// Parse the `vcs-remote-icons` node: each child is a domain→icon string.
fn parse_remote_icons(node: &KdlNode) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            if let Some(KdlValue::String(icon)) = first_value(child) {
                out.push((child.name().value().to_string(), icon.clone()));
            }
        }
    }
    out
}

/// Parse the `frame` node: the frame-level style (node properties fg/bg/bold) + the
/// prefix/suffix of first/newline/last (child nodes, value a string; a child's own fg/bg/bold overrides the frame level).
fn parse_frame(node: &KdlNode) -> Frame {
    let mut f = Frame {
        style: Style::from_entries(node.entries()),
        ..Frame::default()
    };
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let Some(v) = first_value(child) else {
                continue;
            };
            let Some(text) = str_val(v) else { continue };
            let st = Style::from_entries(child.entries());
            // A child with no style properties → None (fall back to frame level); with them → Some (override).
            let style = if st == Style::default() {
                None
            } else {
                Some(st)
            };
            let piece = FramePiece { text, style };
            match child.name().value() {
                "first-prefix" => f.first_prefix = piece,
                "first-suffix" => f.first_suffix = piece,
                "newline-prefix" => f.newline_prefix = piece,
                "newline-suffix" => f.newline_suffix = piece,
                "last-prefix" => f.last_prefix = piece,
                "last-suffix" => f.last_suffix = piece,
                _ => warn_unknown_key(child.name().value()),
            }
        }
    }
    f
}

/// Parse `layout`: the two layout line groups `left`/`right`, plus `prompt-add-newline`, `transient-prompt`, `show-ruler`, `right-indent`.
fn parse_layout(node: &KdlNode) -> Result<Layout, String> {
    let mut layout = Layout::default();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "left" => layout.left = parse_lines(child),
                "right" => layout.right = parse_lines(child),
                // `prompt-add-newline #true` = 1 line; a number is taken as is (0 = off).
                "prompt-add-newline" => {
                    layout.prompt_add_newline = match first_value(child) {
                        Some(KdlValue::Integer(n)) => (*n).clamp(0, 9) as usize,
                        v => usize::from(v.map(bool_val).unwrap_or(false)),
                    };
                }
                "transient-prompt" => {
                    layout.transient_prompt = first_value(child).map(bool_val).unwrap_or(false);
                }
                // `show-ruler #true`: add a full ruler line above the header (p10k SHOW_RULER).
                "show-ruler" => {
                    layout.show_ruler = first_value(child).map(bool_val).unwrap_or(false);
                }
                // `right-indent <N>`: columns kept between the right column and the right edge (default 1, same as zsh).
                "right-indent" => {
                    if let Some(KdlValue::Integer(n)) = first_value(child) {
                        layout.right_indent = (*n).clamp(0, 9) as usize;
                    }
                }
                _ => warn_unknown_key(child.name().value()),
            }
        }
    }
    Ok(layout)
}

/// Parse one side: each child node is one line.
fn parse_lines(node: &KdlNode) -> Vec<Vec<Element>> {
    let mut rows = Vec::new();
    if let Some(ch) = node.children() {
        for row in ch.nodes() {
            rows.push(parse_line(row));
        }
    }
    rows
}

/// Parse one line: each child node is a segment (boolean enable), or `text "…"` static text.
fn parse_line(node: &KdlNode) -> Vec<Element> {
    let mut out = Vec::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            // A string value → static text (e.g. `text "some text"`); otherwise treat it as a segment.
            if let Some(KdlValue::String(s)) = first_value(child) {
                out.push(Element::Text(s.clone()));
                continue;
            }
            let name = child.name().value();
            // An absent value means enabled; an explicit #false disables it.
            let enabled = first_value(child).map(bool_val).unwrap_or(true);
            if !enabled {
                continue;
            }
            if let Some(base) = name.strip_suffix("_joined") {
                out.push(Element::Joined(base.to_string()));
            } else {
                out.push(Element::Seg(name.to_string()));
            }
        }
    }
    out
}

/// Parse one segment node: properties (style/text/visibility/behavior) + `state <NAME> …` child overrides.
fn parse_segment(node: &KdlNode) -> Result<Segment, String> {
    let mut seg = Segment {
        style: Style::from_entries(node.entries()),
        shown: true,
        ..Default::default()
    };
    for e in node.entries() {
        let Some(name) = e.name() else { continue };
        match name.value() {
            "fg" | "bg" | "bold" => {}
            "content" => seg.content = str_val(e.value()),
            "prefix" => seg.prefix = str_val(e.value()),
            "suffix" => seg.suffix = str_val(e.value()),
            "disabled" => seg.shown = !bool_val(e.value()),
            _ => {
                seg.props
                    .insert(name.value().to_string(), prop_val(e.value()));
            }
        }
    }
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "state" => {
                    let Some(nm) = first_state_name(child) else {
                        return Err(t("`state` needs a name (a positional string)"));
                    };
                    let st = Style::from_entries(child.entries());
                    let ch = named_str(child, "char");
                    seg.states.insert(
                        nm,
                        StateSpec {
                            style: st,
                            char: ch,
                        },
                    );
                }
                // Attached text slots: left/middle/right. The positional string is the text, `fg` is optional.
                "text-left" | "text-middle" | "text-right" => {
                    let Some(text) = first_state_name(child) else {
                        return Err(format!(
                            "{}{}",
                            child.name().value(),
                            t(" needs text (a positional string)")
                        ));
                    };
                    let attach = AttachText {
                        text,
                        fg: named_color(child, "fg"),
                    };
                    match child.name().value() {
                        "text-left" => seg.text_left = Some(attach),
                        "text-middle" => seg.text_middle = Some(attach),
                        "text-right" => seg.text_right = Some(attach),
                        _ => warn_unknown_key(child.name().value()),
                    }
                }
                _ => warn_unknown_key(child.name().value()),
            }
        }
    }
    Ok(seg)
}

/// A node's named string property (e.g. a state node's `char="✘"`).
fn named_str(node: &KdlNode, name: &str) -> Option<String> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
        .and_then(|e| str_val(e.value()))
}

/// A node's named color property (e.g. attached text's `fg=196`).
fn named_color(node: &KdlNode, name: &str) -> Option<Color> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
        .map(|e| Color::from_value(e.value()))
}

/// A node's first positional argument value (the entry with no name).
fn first_value(node: &KdlNode) -> Option<&KdlValue> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| e.value())
}

/// A state node's name = its first positional argument.
fn first_state_name(node: &KdlNode) -> Option<String> {
    match first_value(node) {
        Some(KdlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Warn when the config contains a key the engine does not know.
fn warn_unknown_key(name: &str) {
    eprintln!("p11k: {}{name}", t("unknown config key: "));
}

/// Parse the `separators` node: character child nodes (`segment`/`sub`/`end`/`right-start`/`right-segment`/`right-sub`/`left-tail`/`right-tail`/`gap`), values read as whole strings.
fn parse_separators(node: &KdlNode) -> Separators {
    let mut s = Separators::default();
    // `gap-foreground` / `sub-foreground` / `right-sub-foreground` are
    // properties of the separators node (not child nodes).
    if let Some(c) = named_color(node, "gap-foreground") {
        s.gap_foreground = Some(c);
    }
    if let Some(c) = named_color(node, "sub-foreground") {
        s.sub_foreground = Some(c);
    }
    if let Some(c) = named_color(node, "right-sub-foreground") {
        s.right_sub_foreground = Some(c);
    }
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let Some(v) = first_value(child) else {
                continue;
            };
            let Some(ch) = str_val(v) else { continue };
            match child.name().value() {
                "segment" => s.segment = ch,
                "sub" => s.sub = ch,
                "end" => s.end = ch,
                "right-start" => s.right_start = ch,
                "right-segment" => s.right_segment = ch,
                "right-sub" => s.right_sub = ch,
                "left-tail" => s.left_tail = ch,
                "right-tail" => s.right_tail = ch,
                "gap" => s.gap = ch,
                _ => warn_unknown_key(child.name().value()),
            }
        }
    }
    s
}

static EMPTY_SEG: Segment = Segment {
    style: Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
    },
    states: BTreeMap::new(),
    content: None,
    text_left: None,
    text_middle: None,
    text_right: None,
    prefix: None,
    suffix: None,
    shown: true,
    props: BTreeMap::new(),
};

/// Merge: the upper layer's non-Default fields override the lower layer (the tail end of the three-way fallback).
fn merge_style(over: &Style, base: &Style) -> Style {
    Style {
        fg: if over.fg == Color::Default {
            base.fg.clone()
        } else {
            over.fg.clone()
        },
        bg: if over.bg == Color::Default {
            base.bg.clone()
        } else {
            over.bg.clone()
        },
        bold: over.bold || base.bold,
    }
}

fn bool_val(v: &KdlValue) -> bool {
    matches!(v, KdlValue::Bool(true))
}
fn str_val(v: &KdlValue) -> Option<String> {
    match v {
        KdlValue::String(s) => Some(s.clone()),
        KdlValue::Integer(n) => Some(n.to_string()),
        KdlValue::Float(f) => Some(f.to_string()),
        KdlValue::Bool(b) => Some(b.to_string()),
        KdlValue::Null => None,
    }
}
fn prop_val(v: &KdlValue) -> Prop {
    match v {
        KdlValue::String(s) => Prop::Str(s.clone()),
        KdlValue::Integer(n) => Prop::Int((*n) as i64),
        KdlValue::Float(f) => Prop::Float(*f),
        KdlValue::Bool(b) => Prop::Bool(*b),
        KdlValue::Null => Prop::Str(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_kdl_roundtrips_presets() {
        for kind in crate::presets::PresetKind::ALL {
            let cfg = crate::presets::build(kind);
            let out = cfg.to_kdl();
            let cfg2 = Config::parse(&out).unwrap();
            assert_eq!(
                cfg,
                cfg2,
                "{} round-trip failed, output:\n{out}",
                kind.name()
            );
        }
    }

    #[test]
    fn to_kdl_roundtrips_rich_config() {
        let src = r#"
mode "ascii"
layout {
    left {
        line { os_icon; dir_joined; text "|" }
        line { prompt_char }
    }
    right {
        line { status }
    }
    prompt-add-newline #true
    transient-prompt #true
}
defaults fg=200 bold=#true
separators gap-foreground=240 {
    segment "\u{e0b0}"
    sub "\u{e0b1}"
    end ""
    right-start "\u{e0b2}"
    right-segment "\u{e0b2}"
    right-sub "\u{e0b3}"
    left-tail "\u{e0b2}"
    right-tail "\u{e0b0}"
    gap "·"
}
frame fg=240 {
    first-prefix "╭─"
    first-suffix "─╮" fg=241
    newline-prefix "├─"
    newline-suffix "─┤"
    last-prefix "╰─"
    last-suffix ""
}
segments {
    dir fg=39 shorten-strategy="truncate_to_unique" shorten-dir-length=3 {
        state SHORTENED fg=103
        state ANCHOR fg=39 bold=#true char="~"
        text-left "L" fg=100
        text-right "R"
    }
    vcs clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status ok-foreground=70 error-foreground=160 verbose=#true
    prompt_char fg=76 {
        state ERROR fg=196 char="✘"
    }
}
vcs-remote-icons {
    github "\u{f113}"
    gitlab "\u{f296}"
}
icon {
    ok { all "✔" ascii "V" }
    folder { nf "\u{f07c}" compat "" }
}
"#;
        let cfg = Config::parse(src).unwrap();
        let out = cfg.to_kdl();
        let cfg2 = Config::parse(&out).unwrap();
        assert_eq!(cfg, cfg2, "rich round-trip failed, output:\n{out}");
    }

    #[test]
    fn parses_layout_lines() {
        let c = Config::parse(
            "layout {\n  left {\n    line { dir #true; vcs #true }\n    line { prompt_char #true }\n  }\n  right {\n    line { status #true }\n  }\n}",
        )
        .unwrap();
        assert_eq!(
            c.layout.left,
            vec![
                vec![Element::Seg("dir".into()), Element::Seg("vcs".into())],
                vec![Element::Seg("prompt_char".into())],
            ]
        );
        assert_eq!(c.layout.right, vec![vec![Element::Seg("status".into())]]);
    }

    #[test]
    fn parses_icon_mode() {
        // The top-level mode node: ascii/compatible/nerdfont-fontconfig map to their modes.
        let c = Config::parse("mode \"ascii\"\nlayout { left { line { dir #true } } }").unwrap();
        assert_eq!(c.mode, IconMode::Ascii);
        let c =
            Config::parse("mode \"compatible\"\nlayout { left { line { dir #true } } }").unwrap();
        assert_eq!(c.mode, IconMode::Compatible);
        let c =
            Config::parse("mode \"nerdfont-fontconfig\"\nlayout { left { line { dir #true } } }")
                .unwrap();
        assert_eq!(c.mode, IconMode::NerdfontFontconfig);
        // No mode written → follow the locale: UTF-8 → nerdfont, otherwise (console/C locale) → ascii.
        let c = Config::parse("layout { left { line { dir #true } } }").unwrap();
        let expect = if locale_is_utf8() {
            IconMode::NerdfontComplete
        } else {
            IconMode::Ascii
        };
        assert_eq!(c.mode, expect);
    }

    #[test]
    fn locale_codeset_utf8_detection() {
        // The locale string's codeset check (aligns with p10k langinfo[CODESET]): the UTF-8 family yes, the rest no.
        assert!(locale_str_is_utf8("en_US.UTF-8"));
        assert!(locale_str_is_utf8("C.UTF-8"));
        assert!(locale_str_is_utf8("zh_CN.utf8"));
        assert!(locale_str_is_utf8("en_US.UTF-8@euro"));
        assert!(!locale_str_is_utf8("C"));
        assert!(!locale_str_is_utf8("POSIX"));
        assert!(!locale_str_is_utf8("ja_JP.EUC-JP"));
        assert!(!locale_str_is_utf8("en_US"));
    }

    #[test]
    fn parses_joined_element_and_disabled() {
        // "dir_joined" shares the background; segments explicitly set to #false are skipped.
        let c = Config::parse(
            "layout {\n  left {\n    line { dir_joined #true; vcs #true; time #false }\n  }\n}",
        )
        .unwrap();
        assert_eq!(c.layout.left[0][0], Element::Joined("dir".into()));
        assert_eq!(c.layout.left[0].len(), 2, "#false time should be skipped");
    }

    #[test]
    fn parses_segment_attrs_states_and_props() {
        let c = Config::parse(
            r#"layout {}
               segments { dir fg=39 shorten-strategy="truncate_to_unique" {
                   state SHORTENED fg=103
                   state ANCHOR fg=39 bold=#true
               } }"#,
        )
        .unwrap();
        let d = c.segment("dir");
        assert_eq!(d.style.fg, Color::Xterm(39));
        assert_eq!(
            d.props["shorten-strategy"],
            Prop::Str("truncate_to_unique".into())
        );
        assert_eq!(d.states["SHORTENED"].style.fg, Color::Xterm(103));
        assert!(d.states["ANCHOR"].style.bold);
    }

    #[test]
    fn effective_style_three_way_fallback() {
        let c = Config::parse(
            r#"layout {}
               defaults fg=200
               segments { dir fg=39 { state SHORTENED fg=103 } }"#,
        )
        .unwrap();
        assert_eq!(
            c.segment("dir")
                .effective_style(Some("SHORTENED"), &c.defaults)
                .fg,
            Color::Xterm(103)
        );
        assert_eq!(
            c.segment("dir").effective_style(None, &c.defaults).fg,
            Color::Xterm(39)
        );
        assert_eq!(
            c.segment("nope").effective_style(None, &c.defaults).fg,
            Color::Xterm(200)
        );
    }

    #[test]
    fn default_lean_parses() {
        let c = Config::default_lean().unwrap();
        assert!(!c.segments.is_empty());
        assert_eq!(c.layout.left.len(), 1, "lean left header is a single line");
        assert_eq!(
            c.layout.left[0],
            vec![Element::Seg("dir".into()), Element::Seg("vcs".into())]
        );
        assert_eq!(
            c.layout.right[0],
            vec![
                Element::Seg("status".into()),
                Element::Seg("command_execution_time".into()),
                Element::Seg("background_jobs".into())
            ]
        );
        assert!(c.segment("dir").props.contains_key("shorten-strategy"));
        // The prompt_char segment is still there (to color the input line prefix) but is not in the header layout.
        assert_eq!(c.segment("prompt_char").style.fg, Color::Xterm(76));
        assert!(
            !c.layout
                .left
                .iter()
                .flatten()
                .any(|e| matches!(e, Element::Seg(s) if s == "prompt_char"))
        );
        assert!(
            c.layout.prompt_add_newline == 0,
            "lean is compact, no blank lines"
        );
    }

    #[test]
    fn bool_and_hex() {
        let c = Config::parse(
            r##"layout {}
defaults fg="#ffffff"
segments {
  a bold=#true x=3
}"##,
        )
        .unwrap();
        assert_eq!(c.defaults.fg, Color::Rgb(255, 255, 255));
        assert!(c.segment("a").style.bold);
        assert_eq!(c.segment("a").props["x"], Prop::Int(3));
    }
}
