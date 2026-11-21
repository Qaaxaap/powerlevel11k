//! Built-in preset themes: constructed from [`Config`] structs, with no hard-coded KDL strings.
//!
//! Visually aligned with the official p10k configs (`config/p10k-{lean,classic,rainbow,pure}.zsh`).
//! One colour table per preset; [`config`] turns a table and a layout into a [`Config`], so a fix
//! to a segment's properties lands in all four. Color variants are parameters: four classic
//! segment backgrounds, four classic/rainbow/lean frame shades, lean 256/8 colors, pure
//! original/snazzy. `--preset` and the engine fallback use [`build`]'s defaults; the wizard calls
//! the constructors with the user's answers and edits fields.

use crate::config::{
    Color, Config, Element, Frame, FramePiece, InputLine, Layout, Prop, Segment, Separators,
    StateSpec, Style,
};

/// Four classic segment backgrounds (Lightest/Light/Dark/Darkest).
pub const BG_COLORS: [u8; 4] = [240, 238, 236, 234];
/// Four classic `sub` thin-line colors (p10k wizard's `sep_color`).
pub const SEP_COLORS: [u8; 4] = [248, 246, 244, 242];
/// Four frame shades (the classic/rainbow frame line color).
pub const FRAME_COLORS: [u8; 4] = [244, 242, 240, 238];

fn x(n: u8) -> Color {
    Color::Xterm(n)
}

// ---- One preset's colours ----

/// Foreground and background of a segment that carries nothing else.
struct PlainColours {
    fg: Color,
    bg: Option<Color>,
}

struct DirColours {
    fg: Color,
    shortened: Color,
    anchor: Color,
    anchor_bold: bool,
    bg: Option<Color>,
}

/// `vcs` colours: either the five count properties, or a foreground for the whole segment.
struct VcsColours {
    fg: Option<Color>,
    /// `clean`, `modified`, `conflicted`, `untracked`, `meta` foregrounds, in that order.
    counts: Option<(u8, u8, u8, u8, u8)>,
    bg: Option<Color>,
}

/// `status` colours: the foregrounds its `ok-foreground` and `error-foreground` properties take.
struct StatusColours {
    ok: u8,
    err: u8,
    bg: Option<Color>,
}

struct PromptCharColours {
    ok: Color,
    err: Color,
}

/// One preset's colours. A `None` segment is not part of the preset.
struct Colours {
    os_icon: Option<PlainColours>,
    dir: DirColours,
    vcs: VcsColours,
    status: Option<StatusColours>,
    exec: PlainColours,
    jobs: Option<PlainColours>,
    prompt_char: PromptCharColours,
}

// ---- Shared construction ----

/// Turn one colour table and its layout into a theme.
fn config(colours: &Colours, layout: Layout, separators: Separators, frame: Frame) -> Config {
    let mut cfg = Config {
        layout,
        separators,
        frame,
        ..base()
    };
    if let Some(c) = &colours.os_icon {
        cfg.segments.insert("os_icon".into(), plain_seg(c));
    }
    cfg.segments.insert("dir".into(), dir_seg(&colours.dir));
    cfg.segments.insert("vcs".into(), vcs_seg(&colours.vcs));
    if let Some(c) = &colours.status {
        cfg.segments.insert("status".into(), status_seg(c));
    }
    cfg.segments
        .insert("command_execution_time".into(), exec_seg(&colours.exec));
    if let Some(c) = &colours.jobs {
        cfg.segments.insert("background_jobs".into(), jobs_seg(c));
    }
    cfg.segments
        .insert("prompt_char".into(), prompt_char_seg(&colours.prompt_char));
    cfg
}

fn plain_seg(c: &PlainColours) -> Segment {
    Segment {
        style: Style {
            fg: Some(c.fg.clone()),
            bg: c.bg.clone(),
            ..Style::default()
        },
        ..Segment::default()
    }
}

fn dir_seg(c: &DirColours) -> Segment {
    let mut s = plain_seg(&PlainColours {
        fg: c.fg.clone(),
        bg: c.bg.clone(),
    });
    s.props.insert(
        "shorten-strategy".into(),
        Prop::Str("truncate_to_unique".into()),
    );
    s.props.insert("shorten-dir-length".into(), Prop::Int(1));
    s.states.insert(
        "SHORTENED".into(),
        StateSpec {
            style: Style {
                fg: Some(c.shortened.clone()),
                ..Style::default()
            },
            char: None,
        },
    );
    s.states.insert(
        "ANCHOR".into(),
        StateSpec {
            style: Style {
                fg: Some(c.anchor.clone()),
                bold: Some(c.anchor_bold),
                ..Style::default()
            },
            char: None,
        },
    );
    s
}

fn vcs_seg(c: &VcsColours) -> Segment {
    let mut s = Segment {
        style: Style {
            fg: c.fg.clone(),
            bg: c.bg.clone(),
            ..Style::default()
        },
        ..Segment::default()
    };
    let Some((clean, modified, conflicted, untracked, meta)) = c.counts else {
        return s;
    };
    s.props
        .insert("clean-foreground".into(), Prop::Int(clean as i64));
    s.props
        .insert("modified-foreground".into(), Prop::Int(modified as i64));
    // In p10k's formatting function conflicted is its own `local conflicted`, a different
    // color from the modified used by staged/unstaged (red 196 for classic/lean).
    s.props
        .insert("conflicted-foreground".into(), Prop::Int(conflicted as i64));
    s.props
        .insert("untracked-foreground".into(), Prop::Int(untracked as i64));
    // meta is the `local meta` of p10k's formatting function: used by detached HEAD's `@`
    // and the tag's `#` (gray 246 for classic/lean, white 7 for rainbow).
    s.props
        .insert("meta-foreground".into(), Prop::Int(meta as i64));
    // Branch names over 32 characters → first 12 + … + last 12 (aligns with the
    // `(( $#branch > 32 )) && branch[13,-13]="…"` hard-coded in the p10k configs).
    s.props.insert("shorten-length".into(), Prop::Int(12));
    s.props.insert("shorten-min-length".into(), Prop::Int(32));
    s.props.insert(
        "shorten-strategy".into(),
        Prop::Str("truncate_middle".into()),
    );
    s
}

fn status_seg(c: &StatusColours) -> Segment {
    let mut s = Segment {
        style: Style {
            bg: c.bg.clone(),
            ..Style::default()
        },
        ..Segment::default()
    };
    s.props
        .insert("ok-foreground".into(), Prop::Int(c.ok as i64));
    s.props
        .insert("error-foreground".into(), Prop::Int(c.err as i64));
    s.props.insert("verbose".into(), Prop::Bool(true));
    s
}

fn exec_seg(c: &PlainColours) -> Segment {
    let mut s = plain_seg(c);
    s.props.insert("threshold-seconds".into(), Prop::Int(3));
    s.props.insert("precision".into(), Prop::Int(0));
    s
}

fn jobs_seg(c: &PlainColours) -> Segment {
    let mut s = plain_seg(c);
    s.props.insert("verbose".into(), Prop::Bool(false));
    s
}

fn prompt_char_seg(c: &PromptCharColours) -> Segment {
    let mut s = Segment {
        style: Style {
            fg: Some(c.ok.clone()),
            ..Style::default()
        },
        ..Segment::default()
    };
    s.states.insert(
        "ERROR".into(),
        StateSpec {
            style: Style {
                fg: Some(c.err.clone()),
                ..Style::default()
            },
            char: None,
        },
    );
    s
}

// ---- Layout, frame and separators ----

/// The one-row layout: the segments share the line with the prompt character.
fn one_row(left: Vec<Element>, right: Vec<Element>) -> Layout {
    Layout {
        input: Some(InputLine { left, right }),
        ..Layout::default()
    }
}

/// The framed header: one row per side, the segments classic and rainbow use.
fn framed_header() -> Layout {
    Layout {
        left: vec![vec![
            Element::Seg("os_icon".into()),
            Element::Seg("dir".into()),
            Element::Seg("vcs".into()),
        ]],
        right: vec![vec![
            Element::Seg("status".into()),
            Element::Seg("command_execution_time".into()),
            Element::Seg("background_jobs".into()),
        ]],
        ..Layout::default()
    }
}

/// powerline arrow frame (classic/rainbow): first line `╭─`, middle `├─`, input line `╰─`.
/// The frame-level fg colors every piece (aligns with p10k `%<frame_color>F╭─`).
fn powerline_frame(color: u8) -> Frame {
    let mut f = Frame::default();
    f.style.fg = Some(x(color));
    f.first_prefix = FramePiece {
        text: "╭─".into(),
        style: None,
    };
    f.first_suffix = FramePiece {
        text: "─╮".into(),
        style: None,
    };
    f.newline_prefix = FramePiece {
        text: "├─".into(),
        style: None,
    };
    f.newline_suffix = FramePiece {
        text: "─┤".into(),
        style: None,
    };
    f.last_prefix = FramePiece {
        text: "╰─".into(),
        style: None,
    };
    f
}

/// Foreground of the `sub` thin line (p10k wizard's `sep_color`). Not set for rainbow (p10k's
/// rainbow subsep embeds no color and follows the segment background gradient).
fn powerline_separators(sub_foreground: Option<u8>) -> Separators {
    Separators {
        segment: "\u{e0b0}".into(),
        sub: "\u{e0b1}".into(),
        end: String::new(),
        right_start: "\u{e0b2}".into(),
        right_segment: "\u{e0b2}".into(),
        right_sub: "\u{e0b3}".into(),
        left_tail: String::new(),
        right_tail: String::new(),
        gap: " ".into(),
        gap_foreground: None,
        sub_foreground: sub_foreground.map(x),
        right_sub_foreground: sub_foreground.map(x),
    }
}

/// Empty config skeleton (mode defaults to nerdfont-complete, vcs remote icons use the built-in table).
fn base() -> Config {
    Config::default()
}

// ---- The four presets ----

/// lean: one row with no frame or arrows and a transparent background, like p10k lean, whose
/// segments share the line with the `❯`. `colors_8` requests the 8-color downgrade (aligns with
/// p10k lean-8colors).
pub fn lean(colors_8: bool) -> Config {
    let colours = if colors_8 {
        // p10k lean-8colors: clean 2 / modified 3 / conflicted 1 / untracked 4 (blue), with meta
        // on the default foreground (0 here). The tuple order is the one on `VcsColours::counts`.
        Colours {
            os_icon: None,
            dir: DirColours {
                fg: x(4),
                shortened: x(4),
                anchor: x(4),
                anchor_bold: false,
                bg: None,
            },
            vcs: VcsColours {
                fg: None,
                counts: Some((2, 3, 1, 4, 0)),
                bg: None,
            },
            status: Some(StatusColours {
                ok: 2,
                err: 1,
                bg: None,
            }),
            exec: PlainColours { fg: x(3), bg: None },
            jobs: Some(PlainColours { fg: x(1), bg: None }),
            prompt_char: PromptCharColours {
                ok: x(2),
                err: x(1),
            },
        }
    } else {
        // p10k lean: clean 76 / modified 178 / untracked 39 (blue) / conflicted 196,
        // meta 246 (gray).
        Colours {
            os_icon: None,
            dir: DirColours {
                fg: x(31),
                shortened: x(103),
                anchor: x(39),
                anchor_bold: true,
                bg: None,
            },
            vcs: VcsColours {
                fg: None,
                counts: Some((76, 178, 196, 39, 246)),
                bg: None,
            },
            status: Some(StatusColours {
                ok: 70,
                err: 160,
                bg: None,
            }),
            exec: PlainColours {
                fg: x(101),
                bg: None,
            },
            jobs: Some(PlainColours {
                fg: x(70),
                bg: None,
            }),
            prompt_char: PromptCharColours {
                ok: x(76),
                err: x(196),
            },
        }
    };
    config(
        &colours,
        one_row(
            vec![Element::Seg("dir".into()), Element::Seg("vcs".into())],
            vec![
                Element::Seg("status".into()),
                Element::Seg("command_execution_time".into()),
                Element::Seg("background_jobs".into()),
            ],
        ),
        Separators::default(),
        Frame::default(),
    )
}

/// classic: multi-line frame + powerline arrows, one dark background for all segments. `color` is
/// one of the four shades (1..=4).
pub fn classic(color: usize) -> Config {
    let i = color.clamp(1, 4) - 1;
    let bg = Some(x(BG_COLORS[i]));
    let colours = Colours {
        os_icon: Some(PlainColours {
            fg: x(255),
            bg: bg.clone(),
        }),
        dir: DirColours {
            fg: x(31),
            shortened: x(103),
            anchor: x(39),
            anchor_bold: true,
            bg: bg.clone(),
        },
        // untracked=39 (blue): p10k's classic/lean vcs formatting function uses
        // `local untracked='%39F'`, which is not the same as
        // POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND (76, used only by the vcs_info fallback path) —
        // the actual rendering is blue.
        // conflicted=196 (red): p10k classic `local conflicted='%196F'`.
        vcs: VcsColours {
            fg: None,
            counts: Some((76, 178, 196, 39, 246)),
            bg: bg.clone(),
        },
        status: Some(StatusColours {
            ok: 70,
            err: 160,
            bg: bg.clone(),
        }),
        exec: PlainColours {
            fg: x(248),
            bg: bg.clone(),
        },
        jobs: Some(PlainColours {
            fg: x(37),
            bg: bg.clone(),
        }),
        prompt_char: PromptCharColours {
            ok: x(76),
            err: x(196),
        },
    };
    config(
        &colours,
        framed_header(),
        powerline_separators(Some(SEP_COLORS[i])),
        powerline_frame(FRAME_COLORS[i]),
    )
}

/// rainbow: classic structure but a colored background per segment (aligns with p10k rainbow).
/// `color` is one of the four frame shades.
pub fn rainbow(color: usize) -> Config {
    let i = color.clamp(1, 4) - 1;
    // rainbow's thin lines embed no color (same as p10k, following the segment background gradient).
    let colours = Colours {
        // Rainbow backgrounds: os white, dir blue, vcs green, status black, exec yellow, jobs black.
        os_icon: Some(PlainColours {
            fg: x(232),
            bg: Some(x(7)),
        }),
        dir: DirColours {
            fg: x(254),
            shortened: x(250),
            anchor: x(255),
            anchor_bold: true,
            bg: Some(x(4)),
        },
        // p10k rainbow's vcs formatting function: clean/modified/untracked are all `%0F` (black, on
        // the green 2 segment background) and conflicted is `%1F` (red). Previously 2/3/2 was passed,
        // and green on green made the branch name and ?N invisible.
        vcs: VcsColours {
            fg: None,
            counts: Some((0, 0, 1, 0, 7)),
            bg: Some(x(2)),
        },
        status: Some(StatusColours {
            ok: 2,
            err: 3,
            bg: Some(x(0)),
        }),
        exec: PlainColours {
            fg: x(0),
            bg: Some(x(3)),
        },
        jobs: Some(PlainColours {
            fg: x(6),
            bg: Some(x(0)),
        }),
        prompt_char: PromptCharColours {
            ok: x(76),
            err: x(196),
        },
    };
    config(
        &colours,
        framed_header(),
        powerline_separators(None),
        powerline_frame(FRAME_COLORS[i]),
    )
}

/// pure: one row with no frame or arrows and a transparent background, like p10k pure, whose
/// segments share the line with the `❯`. `snazzy` selects the Snazzy true colors, otherwise
/// Original.
pub fn pure(snazzy: bool) -> Config {
    // Aligns with the pure_original / pure_snazzy palettes of p10k pure.
    let (grey, red, yellow, blue, magenta) = if snazzy {
        (
            x(242),
            Color::Rgb(255, 92, 87),
            Color::Rgb(243, 249, 157),
            Color::Rgb(87, 199, 255),
            Color::Rgb(255, 106, 193),
        )
    } else {
        (x(242), x(1), x(3), x(4), x(5))
    };
    let colours = Colours {
        os_icon: None,
        dir: DirColours {
            fg: blue.clone(),
            shortened: grey.clone(),
            anchor: blue,
            anchor_bold: false,
            bg: None,
        },
        vcs: VcsColours {
            fg: Some(grey.clone()),
            counts: None,
            bg: None,
        },
        status: None,
        exec: PlainColours {
            fg: yellow,
            bg: None,
        },
        jobs: None,
        prompt_char: PromptCharColours {
            ok: magenta,
            err: red,
        },
    };
    // One row, like p10k pure: the segments share the line with the `❯`, and there is no right
    // column.
    let mut cfg = config(
        &colours,
        one_row(
            vec![
                Element::Seg("context".into()),
                Element::Seg("dir".into()),
                Element::Seg("vcs".into()),
                Element::Seg("command_execution_time".into()),
            ],
            Vec::new(),
        ),
        Separators::default(),
        Frame::default(),
    );
    let mut context = Segment::default();
    context.style.fg = Some(grey);
    cfg.segments.insert("context".into(), context);
    cfg
}

// ---- Preset catalog ----

/// Preset identity (name/title/default-argument construction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    Lean,
    Classic,
    Rainbow,
    Pure,
}

impl PresetKind {
    pub fn name(self) -> &'static str {
        match self {
            PresetKind::Lean => "lean",
            PresetKind::Classic => "classic",
            PresetKind::Rainbow => "rainbow",
            PresetKind::Pure => "pure",
        }
    }

    /// Style name shown in the wizard (aligns with p10k `ask_style`'s wording).
    pub fn title(self) -> &'static str {
        match self {
            PresetKind::Lean => "preset-lean",
            PresetKind::Classic => "preset-classic",
            PresetKind::Rainbow => "preset-rainbow",
            PresetKind::Pure => "preset-pure",
        }
    }

    /// All presets, in wizard display order.
    pub const ALL: [PresetKind; 4] = [
        PresetKind::Lean,
        PresetKind::Classic,
        PresetKind::Rainbow,
        PresetKind::Pure,
    ];
}

pub fn by_name(name: &str) -> Option<PresetKind> {
    PresetKind::ALL.iter().copied().find(|k| k.name() == name)
}

/// Build with default arguments (used by `--preset` / engine fallback).
pub fn build(kind: PresetKind) -> Config {
    match kind {
        PresetKind::Lean => lean(false),
        PresetKind::Classic => classic(2),
        PresetKind::Rainbow => rainbow(2),
        PresetKind::Pure => pure(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_builds_and_serializes() {
        for kind in PresetKind::ALL {
            let cfg = build(kind);
            let out = cfg.to_kdl();
            let reparsed = crate::config::Config::parse(&out).unwrap();
            assert_eq!(cfg, reparsed, "{:?} round-trip failed", kind.name());
        }
    }

    #[test]
    fn classic_color_shades_differ() {
        let light = classic(1);
        let dark = classic(4);
        assert_ne!(
            light.segment("dir").style.bg,
            dark.segment("dir").style.bg,
            "segment bg should differ across the four shades"
        );
        assert_ne!(light.frame.style.fg, dark.frame.style.fg);
    }

    #[test]
    fn classic_colors_every_segment_and_keeps_frame_transparent() {
        // p10k classic takes its background from the global `POWERLEVEL9K_BACKGROUND`, which
        // segments inherit while frame characters stay transparent. p11k's `defaults.bg` is also
        // the fallback end for frame/text, so the background must be written per segment:
        // putting it in defaults tints `╭─`/`╰─` as well (verified empirically).
        for (color, want) in [(1usize, 240u8), (2, 238), (3, 236), (4, 234)] {
            let c = classic(color);
            for seg in ["os_icon", "dir", "vcs", "status", "command_execution_time"] {
                assert_eq!(
                    c.segment(seg).style.bg,
                    Some(x(want)),
                    "shade {color}: segment {seg} bg should be {want}"
                );
            }
            assert_eq!(
                c.defaults.bg, None,
                "defaults should have no bg (otherwise frame chars get tinted)"
            );
            // Frame piece style: foreground only, no background.
            let piece = c.frame_piece_style(&c.frame.first_prefix);
            assert_eq!(piece.bg, None, "frame chars must be transparent");
        }
        // lean/pure/rainbow are globally transparent; rainbow gives each segment its own background.
        assert_eq!(lean(false).defaults.bg, None);
        assert_eq!(pure(false).defaults.bg, None);
        assert_eq!(rainbow(1).defaults.bg, None);
        assert_eq!(
            rainbow(1).segment("os_icon").style.bg,
            Some(x(7)),
            "rainbow gives every segment its own bg"
        );
    }

    #[test]
    fn pure_snazzy_differs_from_original() {
        assert_ne!(pure(false), pure(true));
    }

    #[test]
    fn wizard_template_kdl_writes_per_segment_background() {
        // Wizard-generated config: background per segment, no bg in defaults (so the frame is not tinted).
        let kdl = classic(2).to_kdl();
        let dir_line = kdl
            .lines()
            .find(|l| l.trim_start().starts_with("dir "))
            .expect("should have a dir segment");
        assert!(
            dir_line.contains("bg=238"),
            "classic shade 2 segment bg should be 238: {dir_line}"
        );
        assert!(
            !kdl.contains("defaults"),
            "should not write defaults (it would tint frame chars too):\n{kdl}"
        );
        // rainbow is the opposite: every segment has its own background, and the time segment carries 7.
        let rb = rainbow(1).to_kdl();
        assert!(
            rb.contains("bg=7"),
            "rainbow os_icon should have a bg:\n{rb}"
        );
    }

    #[test]
    fn vcs_colors_match_p10k_generated_configs() {
        use crate::config::Prop;
        let int = |c: &Config, seg: &str, key: &str| match c.segment(seg).prop(key) {
            Some(Prop::Int(n)) => Some(*n),
            _ => None,
        };
        // p10k lean/classic's vcs formatting function: clean %76F, modified %178F,
        // untracked %39F (blue), conflicted %196F (red). untracked differs from
        // POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND (76, used only by the vcs_info fallback path).
        for c in [lean(false), classic(1)] {
            assert_eq!(int(&c, "vcs", "clean-foreground"), Some(76));
            assert_eq!(int(&c, "vcs", "modified-foreground"), Some(178));
            assert_eq!(int(&c, "vcs", "untracked-foreground"), Some(39));
            assert_eq!(int(&c, "vcs", "conflicted-foreground"), Some(196));
        }
        // p10k lean-8colors: untracked 4 (blue), conflicted 1 (red).
        let l8 = lean(true);
        assert_eq!(int(&l8, "vcs", "untracked-foreground"), Some(4));
        assert_eq!(int(&l8, "vcs", "conflicted-foreground"), Some(1));
        // p10k rainbow: the vcs segment background is green 2 and all its text is black 0 (it must not match the background, or the text becomes invisible).
        let rb = rainbow(1);
        let vcs = rb.segment("vcs");
        assert_eq!(vcs.style.bg, Some(x(2)));
        assert_eq!(int(&rb, "vcs", "clean-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "modified-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "untracked-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "conflicted-foreground"), Some(1));
    }

    #[test]
    fn vcs_branch_shortening_matches_p10k_hardcoded_rule() {
        // `(( $#branch > 32 )) && branch[13,-13]="…"` is hard-coded in the p10k configs.
        use crate::config::Prop;
        let c = classic(1);
        let p = |k: &str| c.segment("vcs").prop(k).cloned();
        assert_eq!(p("shorten-length"), Some(Prop::Int(12)));
        assert_eq!(p("shorten-min-length"), Some(Prop::Int(32)));
        assert_eq!(
            p("shorten-strategy"),
            Some(Prop::Str("truncate_middle".into()))
        );
    }
}
