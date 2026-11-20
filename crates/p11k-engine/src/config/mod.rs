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
use std::path::PathBuf;

use crate::i18n::t_args;

/// Color: a number = 256-color palette, `#rrggbb` = 24-bit, `"default"`/absent = inherit the terminal.
mod locale;
mod parse;
mod parse_helpers;
mod types;
mod write;

pub(crate) use locale::glyph_fits_mode;
use locale::*;
use parse_helpers::*;
pub use types::*;

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
    /// `shell "zsh"`: the shell to proxy. `--shell` overrides it, and `$SHELL` is used when
    /// neither is given.
    pub shell: Option<String>,
    /// `instant-prompt #false`: do not draw the provisional header while the shell loads its
    /// config (p10k `POWERLEVEL9K_INSTANT_PROMPT` off). Default true.
    pub instant_prompt: bool,
    /// `icon-padding "moderate"`: how much space separates an icon from the content next to it.
    /// p10k's `POWERLEVEL9K_ICON_PADDING`, which defaults to `none` (one space).
    pub icon_padding: IconPadding,
}

/// Where the theme file lives: `$XDG_CONFIG_HOME/p11k/p11k.kdl`, falling back to
/// `~/.config/p11k/p11k.kdl`. `p11k configure` writes there, and the engine reads from
/// there when `--config` is not given.
pub fn default_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("p11k").join("p11k.kdl")
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
            shell: None,
            instant_prompt: true,
            icon_padding: IconPadding::default(),
        }
    }
}

impl Config {
    pub fn default_lean() -> Result<Config, String> {
        Ok(crate::presets::build(crate::presets::PresetKind::Lean))
    }

    /// Apply the locale fallback to a parsed config.
    ///
    /// The effective mode is the more restrictive of the declared mode and the mode the locale
    /// can display. A declared glyph the effective mode cannot display falls back to the
    /// built-in default for that field, and a warning names every fallback. When a glyph has no
    /// usable fallback, the config cannot be rendered safely and the caller refuses to start.
    pub fn apply_locale_fallback(&mut self) -> Result<Vec<String>, String> {
        self.apply_locale_fallback_with(locale_capable_mode())
    }

    /// [`Self::apply_locale_fallback`] with an explicit locale capability (tests).
    fn apply_locale_fallback_with(&mut self, capable: IconMode) -> Result<Vec<String>, String> {
        let declared = self.mode.clone();
        let effective = declared.clone().min(capable);
        let mut warnings = Vec::new();
        if effective != declared {
            warnings.push(t_args(
                "config-mode-fallback",
                &[
                    ("declared", declared.name()),
                    ("effective", effective.name()),
                ],
            ));
        }
        self.mode = effective.clone();
        self.rollback_glyphs(&effective, &mut warnings);
        if let Some(error) = self.unrenderable_glyph(&effective) {
            return Err(error);
        }
        Ok(warnings)
    }

    /// Replace every declared glyph the effective mode cannot display by the built-in default.
    fn rollback_glyphs(&mut self, mode: &IconMode, warnings: &mut Vec<String>) {
        fn rollback_text(
            text: &mut String,
            field: &str,
            mode: &IconMode,
            warnings: &mut Vec<String>,
        ) {
            if text_overflows_mode(text, mode) {
                warnings.push(glyph_fallback(field, &text.escape_default().to_string()));
                text.clear();
            }
        }

        for (field, piece) in [
            ("frame first-prefix", &mut self.frame.first_prefix),
            ("frame first-suffix", &mut self.frame.first_suffix),
            ("frame newline-prefix", &mut self.frame.newline_prefix),
            ("frame newline-suffix", &mut self.frame.newline_suffix),
            ("frame last-prefix", &mut self.frame.last_prefix),
            ("frame last-suffix", &mut self.frame.last_suffix),
        ] {
            rollback_text(&mut piece.text, field, mode, warnings);
        }
        for (field, text) in [
            ("separators segment", &mut self.separators.segment),
            ("separators sub", &mut self.separators.sub),
            ("separators end", &mut self.separators.end),
            ("separators right-start", &mut self.separators.right_start),
            (
                "separators right-segment",
                &mut self.separators.right_segment,
            ),
            ("separators right-sub", &mut self.separators.right_sub),
            ("separators left-tail", &mut self.separators.left_tail),
            ("separators right-tail", &mut self.separators.right_tail),
            ("separators gap", &mut self.separators.gap),
        ] {
            rollback_text(text, field, mode, warnings);
        }
        for (name, segment) in self.segments.iter_mut() {
            for (field, text) in [
                ("content", &mut segment.content),
                ("prefix", &mut segment.prefix),
                ("suffix", &mut segment.suffix),
            ] {
                let glyph = text
                    .as_deref()
                    .filter(|value| text_overflows_mode(value, mode))
                    .map(|value| value.escape_default().to_string());
                if let Some(glyph) = glyph {
                    warnings.push(glyph_fallback(&format!("segment {name} {field}"), &glyph));
                    *text = None;
                }
            }
            for (field, attach) in [
                ("text-left", &mut segment.text_left),
                ("text-middle", &mut segment.text_middle),
                ("text-right", &mut segment.text_right),
            ] {
                let glyph = attach
                    .as_ref()
                    .filter(|value| text_overflows_mode(&value.text, mode))
                    .map(|value| value.text.escape_default().to_string());
                if let Some(glyph) = glyph {
                    warnings.push(glyph_fallback(&format!("segment {name} {field}"), &glyph));
                    *attach = None;
                }
            }
            for (state, spec) in segment.states.iter_mut() {
                let glyph = spec
                    .char
                    .as_deref()
                    .filter(|value| text_overflows_mode(value, mode))
                    .map(|value| value.escape_default().to_string());
                if let Some(glyph) = glyph {
                    warnings.push(glyph_fallback(
                        &format!("segment {name} state {state}"),
                        &glyph,
                    ));
                    spec.char = None;
                }
            }
            let char_glyph = segment
                .props
                .get("char")
                .and_then(|prop| match prop {
                    crate::config::Prop::Str(value) => Some(value),
                    _ => None,
                })
                .filter(|value| text_overflows_mode(value, mode))
                .map(|value| value.escape_default().to_string());
            if let Some(glyph) = char_glyph {
                warnings.push(glyph_fallback(&format!("segment {name} char"), &glyph));
                segment.props.remove("char");
            }
        }
        let mode_slot: fn(&mut crate::config::IconOverride) -> &mut Option<String> = match mode {
            IconMode::NerdfontComplete | IconMode::NerdfontFontconfig => |ov| &mut ov.nf,
            IconMode::Compatible => |ov| &mut ov.compat,
            IconMode::Ascii => |ov| &mut ov.ascii,
        };
        for (name, ov) in self.icon_overrides.iter_mut() {
            let all_glyph = ov
                .all
                .as_deref()
                .filter(|value| text_overflows_mode(value, mode))
                .map(|value| value.escape_default().to_string());
            if let Some(glyph) = all_glyph {
                warnings.push(glyph_fallback(&format!("icon {name} all"), &glyph));
                ov.all = None;
            }
            let slot = mode_slot(ov);
            let mode_glyph = slot
                .as_deref()
                .filter(|value| text_overflows_mode(value, mode))
                .map(|value| value.escape_default().to_string());
            if let Some(glyph) = mode_glyph {
                warnings.push(glyph_fallback(&format!("icon {name} mode"), &glyph));
                *slot = None;
            }
        }
        for class in &mut self.dir_classes {
            rollback_text(&mut class.icon, "dir-classes icon", mode, warnings);
        }
        // The built-in remote table is Nerd Font only; rolling it back in a non-UTF-8 locale is
        // normal degradation, not a user error, so only a configured table warns.
        let configured_remotes = self.vcs_remote_icons != default_remote_icons();
        self.vcs_remote_icons.retain(|(domain, icon)| {
            if text_overflows_mode(icon, mode) {
                if configured_remotes {
                    warnings.push(glyph_fallback(
                        &format!("vcs-remote-icons {domain}"),
                        &icon.escape_default().to_string(),
                    ));
                }
                false
            } else {
                true
            }
        });
    }

    /// Return an error when the rolled-back config still holds a glyph the mode cannot display.
    fn unrenderable_glyph(&self, mode: &IconMode) -> Option<String> {
        let mut bad = Vec::new();
        let mut check = |field: &str, text: &str| {
            if text_overflows_mode(text, mode) {
                bad.push(format!("{field} \"{}\"", text.escape_default()));
            }
        };
        for (field, piece) in [
            ("frame first-prefix", &self.frame.first_prefix),
            ("frame first-suffix", &self.frame.first_suffix),
            ("frame newline-prefix", &self.frame.newline_prefix),
            ("frame newline-suffix", &self.frame.newline_suffix),
            ("frame last-prefix", &self.frame.last_prefix),
            ("frame last-suffix", &self.frame.last_suffix),
        ] {
            check(field, &piece.text);
        }
        for (field, text) in [
            ("separators segment", &self.separators.segment),
            ("separators sub", &self.separators.sub),
            ("separators end", &self.separators.end),
            ("separators right-start", &self.separators.right_start),
            ("separators right-segment", &self.separators.right_segment),
            ("separators right-sub", &self.separators.right_sub),
            ("separators left-tail", &self.separators.left_tail),
            ("separators right-tail", &self.separators.right_tail),
            ("separators gap", &self.separators.gap),
        ] {
            check(field, text);
        }
        for (name, segment) in &self.segments {
            for (field, text) in [
                ("content", &segment.content),
                ("prefix", &segment.prefix),
                ("suffix", &segment.suffix),
            ] {
                if let Some(text) = text {
                    check(&format!("segment {name} {field}"), text);
                }
            }
            for (field, attach) in [
                ("text-left", &segment.text_left),
                ("text-middle", &segment.text_middle),
                ("text-right", &segment.text_right),
            ] {
                if let Some(attach) = attach {
                    check(&format!("segment {name} {field}"), &attach.text);
                }
            }
            for (state, spec) in &segment.states {
                if let Some(text) = &spec.char {
                    check(&format!("segment {name} state {state}"), text);
                }
            }
            if let Some(crate::config::Prop::Str(text)) = segment.props.get("char") {
                check(&format!("segment {name} char"), text);
            }
        }
        for (name, ov) in &self.icon_overrides {
            // Only the `all` slot and the effective mode's slot are rendered; a slot for an
            // inactive mode is kept for a future UTF-8 session and is not validated here.
            let mode_slot = match mode {
                IconMode::NerdfontComplete | IconMode::NerdfontFontconfig => &ov.nf,
                IconMode::Compatible => &ov.compat,
                IconMode::Ascii => &ov.ascii,
            };
            for (field, slot) in [("all", &ov.all), ("mode", mode_slot)] {
                if let Some(text) = slot {
                    check(&format!("icon {name} {field}"), text);
                }
            }
        }
        for class in &self.dir_classes {
            check("dir-classes icon", &class.icon);
        }
        for (domain, icon) in &self.vcs_remote_icons {
            check(&format!("vcs-remote-icons {domain}"), icon);
        }
        if bad.is_empty() {
            None
        } else {
            let glyphs = bad.join(", ");
            Some(t_args(
                "config-unrenderable-glyphs",
                &[("mode", mode.name()), ("glyphs", &glyphs)],
            ))
        }
    }

    /// Serialize to KDL text; `parse(to_kdl(cfg))` must yield an equivalent config (round-trip).
    /// Lay it out with kdl-rs autoformat (indentation/spacing), otherwise the constructed
    /// document is packed tight; then switch string values to their quoted representation
    /// (see [`quote_string_values`]).
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
}

#[cfg(test)]
mod tests;
