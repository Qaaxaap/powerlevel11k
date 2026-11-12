//! One function per question: the label, the preview and the edit applied to the config.

use std::io::Write;

use crate::config::{Color, Config, Element, Frame, IconMode, Prop, Segment, Separators};
use crate::i18n::msgid;
use crate::presets::{self, PresetKind};

use super::Answer;
use super::input::Input;
use super::preview::preview_lines;
use super::ui::{ask_apply, ask_choice_preview, ask_yn};
use super::write::{add_to_right, remove_segment, set_prefix};

/// Font detection: diamond (⮀⮂) + lock ()/quotes (❯❮) — three glyphs landing on the three mode tiers.
pub(super) fn ask_font(out: &mut dyn Write, input: &mut dyn Input) -> Answer<IconMode> {
    let diamond = ask_yn(
        out,
        input,
        msgid("Does this look like a diamond (rotated square)?"),
        msgid("Yes."),
        msgid("No."),
        Some("--->  \u{e0b2}\u{e0b0}  <---"),
    )?;
    if diamond {
        match ask_yn(
            out,
            input,
            msgid("Does this look like a lock?"),
            msgid("Yes."),
            msgid("No."),
            Some("--->  \u{f023}  <---"),
        )? {
            true => Ok(IconMode::NerdfontComplete),
            false => Ok(IconMode::Compatible),
        }
    } else {
        match ask_yn(
            out,
            input,
            msgid("Does this look like >< but taller and fatter?"),
            msgid("Yes."),
            msgid("No."),
            Some("--->  \u{276f}\u{276e}  <---"),
        )? {
            true => Ok(IconMode::Compatible),
            false => Ok(IconMode::Ascii),
        }
    }
}

/// Style: each of the four presets comes with a live prompt preview.
/// Prompt preview for one preset and icon mode. The style preview has to use the icon mode the
/// user just chose, otherwise it shows Nerd Font glyphs after the user picked ASCII.
pub(super) fn style_preview(kind: PresetKind, mode: &IconMode) -> Vec<String> {
    let mut cfg = presets::build(kind);
    cfg.mode = mode.clone();
    preview_lines(&cfg)
}

/// Style: each of the four presets comes with a live prompt preview.
pub(super) fn ask_style(
    out: &mut dyn Write,
    input: &mut dyn Input,
    mode: &IconMode,
) -> Answer<PresetKind> {
    let titles = PresetKind::ALL.map(|k| k.title());
    let i = ask_choice_preview(out, input, msgid("Prompt Style"), &titles, |i| {
        style_preview(PresetKind::ALL[i], mode)
    })?;
    Ok(PresetKind::ALL[i])
}

/// Build a config from style + color shade index (preview and final result share one source).
pub(super) fn build_with_color(kind: PresetKind, i: usize) -> Config {
    match kind {
        PresetKind::Lean => presets::lean(i == 1),
        PresetKind::Classic => presets::classic(i + 1),
        PresetKind::Rainbow => presets::rainbow(i + 1),
        PresetKind::Pure => presets::pure(i == 1),
    }
}

/// Color variant (branched by style): lean 256/8, classic four shades, rainbow four frame shades, pure two palettes.
pub(super) fn ask_color(
    out: &mut dyn Write,
    input: &mut dyn Input,
    kind: PresetKind,
    mode: &IconMode,
) -> Answer<Config> {
    // p10k's four shade names (`color_name`).
    const FOUR: [&str; 4] = [
        msgid("Lightest."),
        msgid("Light."),
        msgid("Dark."),
        msgid("Darkest."),
    ];
    let (title, options): (&str, &[&str]) = match kind {
        PresetKind::Lean => (
            msgid("Prompt Colors"),
            &[msgid("256 colors."), msgid("8 colors.")],
        ),
        PresetKind::Classic => (msgid("Prompt Color"), &FOUR),
        PresetKind::Rainbow => (msgid("Frame Color"), &FOUR),
        PresetKind::Pure => (
            msgid("Prompt Colors"),
            &[msgid("Original."), msgid("Snazzy.")],
        ),
    };
    let i = ask_choice_preview(out, input, title, options, |i| {
        let mut c = build_with_color(kind, i);
        c.mode = mode.clone();
        preview_lines(&c)
    })?;
    let mut c = build_with_color(kind, i);
    c.mode = mode.clone();
    Ok(c)
}

/// Character set: Unicode / ASCII (the preview reflects the icon/separator substitutions under ASCII).
pub(super) fn ask_charset(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &Config,
    mode: &IconMode,
) -> Answer<bool> {
    let i = ask_choice_preview(
        out,
        input,
        msgid("Character Set"),
        &[msgid("Unicode."), msgid("ASCII.")],
        |i| {
            let mut c = cfg.clone();
            c.mode = if i == 1 {
                IconMode::Ascii
            } else {
                mode.clone()
            };
            preview_lines(&c)
        },
    )?;
    Ok(i == 1)
}

/// Location of pure's non-permanent content (exec/context/virtualenv).
pub(super) fn ask_use_rprompt(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Non-permanent content location"),
        &[msgid("Left."), msgid("Right.")],
        cfg,
        |c, i| {
            // First remove exec from both columns, then put it back on the left/right per the choice.
            for row in c.layout.left.iter_mut().chain(c.layout.right.iter_mut()) {
                row.retain(|e| !matches!(e, Element::Seg(s) if s == "command_execution_time"));
            }
            let side = if i == 0 {
                &mut c.layout.left
            } else {
                &mut c.layout.right
            };
            if side.is_empty() {
                side.push(Vec::new());
            }
            side[0].push(Element::Seg("command_execution_time".into()));
        },
    )
}

pub(super) fn ask_time(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
    kind: PresetKind,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Show current time?"),
        &[
            msgid("No."),
            msgid("12-hour format."),
            msgid("24-hour format."),
        ],
        cfg,
        |c, i| match i {
            0 => remove_segment(c, "time"),
            _ => {
                add_to_right(c, "time");
                let fmt = if i == 1 { "12h" } else { "24h" };
                // The background must match the current style, otherwise the new segment turns
                // transparent or black (in p10k, classic relies on the global
                // `POWERLEVEL9K_BACKGROUND`, rainbow uses `TIME_BACKGROUND=7`, lean/pure are
                // transparent). Compute the background before borrowing the entry (to avoid a
                // simultaneous mutable and immutable borrow).
                let bg = time_bg(kind, c);
                let t = c.segments.entry("time".into()).or_insert_with(|| {
                    let mut s = Segment::default();
                    s.style.fg = Color::Xterm(66);
                    if let Some(b) = bg {
                        s.style.bg = Color::Xterm(b);
                    }
                    s
                });
                t.props.insert("time-format".into(), Prop::Str(fmt.into()));
            }
        },
    )
}

/// Background an added segment should use (xterm 256 color number; None = transparent).
///
/// In p10k a segment's background comes from the global `POWERLEVEL9K_BACKGROUND` (classic) or
/// the segment's own `*_BACKGROUND` (rainbow's `TIME_BACKGROUND=7`), and lean/pure are
/// transparent. p11k's classic writes bg per segment, so the same shade is taken here from an
/// existing segment (`dir`).
pub(super) fn time_bg(kind: PresetKind, cfg: &Config) -> Option<u8> {
    match kind {
        PresetKind::Rainbow => Some(7),
        PresetKind::Classic => match cfg.segment("dir").style.bg {
            Color::Xterm(n) => Some(n),
            _ => None,
        },
        PresetKind::Lean | PresetKind::Pure => None,
    }
}

/// Separators (segment/sub): Angled/Vertical/Slanted/Round.
pub(super) fn ask_separators(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Separators"),
        &[
            msgid("Angled."),
            msgid("Vertical."),
            msgid("Slanted."),
            msgid("Round."),
        ],
        cfg,
        |c, i| {
            let (seg, sub, rseg, rsub) = match i {
                0 => ("\u{e0b0}", "\u{e0b1}", "\u{e0b2}", "\u{e0b3}"),
                1 => ("", "\u{2502}", "", "\u{2502}"),
                2 => ("\u{e0bc}", "\u{2571}", "\u{e0ba}", "\u{2571}"),
                _ => ("\u{e0b4}", "\u{e0b5}", "\u{e0b6}", "\u{e0b7}"),
            };
            c.separators.segment = seg.into();
            c.separators.sub = sub.into();
            c.separators.right_segment = rseg.into();
            c.separators.right_sub = rsub.into();
        },
    )
}

/// End symbols, heads (end/right-start): Flat/Blurred/Sharp/Slanted/Round.
pub(super) fn ask_heads(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Heads"),
        &[
            msgid("Flat."),
            msgid("Blurred."),
            msgid("Sharp."),
            msgid("Slanted."),
            msgid("Round."),
        ],
        cfg,
        |c, i| {
            let (end, rstart) = match i {
                0 => ("", ""),
                1 => ("\u{2593}\u{2592}\u{2591}", "\u{2591}\u{2592}\u{2593}"),
                2 => ("\u{e0b0}", "\u{e0b2}"),
                3 => ("\u{e0bc}", "\u{e0ba}"),
                _ => ("\u{e0b4}", "\u{e0b6}"),
            };
            c.separators.end = end.into();
            c.separators.right_start = rstart.into();
        },
    )
}

/// End symbols, tails (left-tail/right-tail).
pub(super) fn ask_tails(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Tails"),
        &[
            msgid("Flat."),
            msgid("Blurred."),
            msgid("Sharp."),
            msgid("Slanted."),
            msgid("Round."),
        ],
        cfg,
        |c, i| {
            let (ltail, rtail) = match i {
                0 => ("", ""),
                1 => ("\u{2591}\u{2592}\u{2593}", "\u{2593}\u{2592}\u{2591}"),
                2 => ("\u{e0b2}", "\u{e0b0}"),
                3 => ("\u{e0ba}", "\u{e0bc}"),
                _ => ("\u{e0b6}", "\u{e0b4}"),
            };
            c.separators.left_tail = ltail.into();
            c.separators.right_tail = rtail.into();
        },
    )
}

/// Height: one line (no header) / two lines.
pub(super) fn ask_num_lines(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Height"),
        &[msgid("One line."), msgid("Two lines.")],
        cfg,
        |c, i| {
            if i == 0 {
                c.layout.left.clear();
                c.layout.right.clear();
                c.frame = Frame::default();
                c.separators = Separators::default();
            }
        },
    )
}

pub(super) fn ask_gap_char(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Connection"),
        &[msgid("Disconnected."), msgid("Dotted."), msgid("Solid.")],
        cfg,
        |c, i| {
            c.separators.gap = match i {
                0 => " ",
                1 => "·",
                _ => "─",
            }
            .into();
            // With a filler character, use the gray foreground (aligns with p10k's
            // MULTILINE_FIRST_PROMPT_GAP_FOREGROUND=240); cleared for Disconnected.
            c.separators.gap_foreground = if i == 0 {
                None
            } else {
                Some(Color::Xterm(240))
            };
        },
    )
}

pub(super) fn ask_frame(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    use crate::config::FramePiece;
    ask_apply(
        out,
        input,
        msgid("Prompt Frame"),
        &[
            msgid("No frame."),
            msgid("Left."),
            msgid("Right."),
            msgid("Full."),
        ],
        cfg,
        |c, i| {
            let f = &mut c.frame;
            let clear_prefix = |f: &mut Frame| {
                f.first_prefix = FramePiece::default();
                f.newline_prefix = FramePiece::default();
                f.last_prefix = FramePiece::default();
            };
            let clear_suffix = |f: &mut Frame| {
                f.first_suffix = FramePiece::default();
                f.newline_suffix = FramePiece::default();
                f.last_suffix = FramePiece::default();
            };
            match i {
                0 => {
                    clear_prefix(f);
                    clear_suffix(f);
                }
                1 => clear_suffix(f),
                2 => clear_prefix(f),
                _ => {}
            }
        },
    )
}

pub(super) fn ask_empty_line(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Spacing"),
        &[msgid("Compact."), msgid("Sparse.")],
        cfg,
        |c, i| c.layout.prompt_add_newline = usize::from(i == 1),
    )
}

pub(super) fn ask_extra_icons(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Icons"),
        &[msgid("Few icons."), msgid("Many icons.")],
        cfg,
        |c, i| {
            if i == 0 {
                // Aligns with p10k's few: the os badge is not rendered at all, and the
                // dir/vcs/branch/exec/time icons are emptied. vcs remote icons
                // (github/gitlab) come from `vcs-remote-icons`, which takes precedence over
                // `icon{ git }`, so they must be cleared too, otherwise remote repos would
                // still show the github icon under Few (p10k empties
                // VCS_VISUAL_IDENTIFIER_EXPANSION entirely). The exec icon corresponds to
                // p10k's COMMAND_EXECUTION_TIME_VISUAL_IDENTIFIER_EXPANSION, which p10k
                // empties under Few as well.
                remove_segment(c, "os_icon");
                c.vcs_remote_icons.clear();
                for key in ["folder", "git", "branch", "time", "execution-time"] {
                    c.icon_overrides.insert(
                        key.into(),
                        crate::config::IconOverride {
                            all: Some(String::new()),
                            ..Default::default()
                        },
                    );
                }
            }
        },
    )
}

/// Prefix connectives: Concise (bare value) / Fluent (with on/took/at).
pub(super) fn ask_prefixes(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Flow"),
        &[msgid("Concise."), msgid("Fluent.")],
        cfg,
        |c, i| {
            let names = ["vcs", "command_execution_time", "time"];
            let prefixes = ["", "on ", "took ", "at "];
            for name in names {
                if let Some(s) = c.segments.get_mut(name) {
                    s.prefix = None;
                }
            }
            if i == 1 {
                set_prefix(c, "vcs", prefixes[1]);
                set_prefix(c, "command_execution_time", prefixes[2]);
                set_prefix(c, "time", prefixes[3]);
            }
        },
    )
}

pub(super) fn ask_transient(
    out: &mut dyn Write,
    input: &mut dyn Input,
    cfg: &mut Config,
) -> Answer<()> {
    let b = ask_yn(
        out,
        input,
        msgid("Enable Transient Prompt?"),
        msgid("Yes."),
        msgid("No."),
        None,
    )?;
    cfg.layout.transient_prompt = if b {
        crate::config::TransientPrompt::Always
    } else {
        crate::config::TransientPrompt::Off
    };
    Ok(())
}
