//! Renderer: drives a KDL config into multi-line header text.
//!
//! - Geometry: rows follow [`Config::layout`], each line being left segments + right-aligned
//!   right segments.
//! - Segments: built-ins (dir/vcs/status/time/command_execution_time/background_jobs/
//!   prompt_char/os/text) render from config, with optional left/middle/right attach text.
//! - Color: configured `fg`/`bg` (three-level fallback) plus inter-segment powerline
//!   separators and background blocks.
//!
//! Input: config plus the current state. Output: the header ANSI string.

use crate::config::{Color, Config, Element, Segment, Style};
use crate::theme::{GitStatus, HeaderInfo};
use std::cell::RefCell;
mod ansi;
mod glob;
mod segment;
mod special;
pub(crate) use self::glob::workdir_is_disabled;
mod env;
mod icons;
mod registry;
mod segments;
mod style;
use self::icons::{icon_str, resolve_icon_choice};
use self::segment::{Ctx, IconStyle, SegmentValue};
pub use self::segments::display_width_of;
use self::segments::*;
pub use self::style::CURRENT_VI_MODE;
pub use self::style::{
    input_prefix, prefix_text, prefix_text_bash, prefix_text_zsh, prompt_state, strip_ansi,
    transient_prompt_zsh,
};
use self::style::{named_256, paint, paint_attach};
use std::collections::HashMap;

/// Renders the header as per-line content (each line = left segments + right-aligned right
/// segments, without cursor/clear/newline). The theme layer draws it line by line as
/// `\r\e[K` + content + `\r\n`.
#[derive(Clone)]
struct SegmentText {
    text: String,
    style: Style,
    /// Whether the text already carries the segment's trailing padding; segments without a
    /// background get no extra space between them.
    padded: bool,
    /// The segment was declared with the `_joined` suffix: it shares a block with the segment
    /// to its left, so no separator is drawn between them even when their backgrounds differ
    /// (p10k `_p9k_left_join` / `_p9k_right_join`; `p10k.zsh:681-691`).
    joined_left: bool,
}

/// The renderer for one prompt. It holds the config, the prompt state and the terminal width;
/// one instance renders one header. [`render_header_lines`] is the free-function entry point.
struct Renderer<'a> {
    config: &'a Config,
    info: &'a HeaderInfo,
    vcs: Option<&'a GitStatus>,
    cols: usize,
}

impl<'a> Renderer<'a> {
    fn new(
        config: &'a Config,
        info: &'a HeaderInfo,
        vcs: Option<&'a GitStatus>,
        cols: usize,
    ) -> Self {
        Renderer {
            config,
            info,
            vcs,
            cols,
        }
    }

    fn header_lines(&self) -> Vec<String> {
        let config = self.config;
        let cols = self.cols;
        let layout = &config.layout;
        let left = &layout.left;
        let right = &layout.right;
        let lines = left.len().max(right.len());
        let frame = &config.frame;
        let mut out: Vec<String> = Vec::with_capacity(lines + 1);
        // p10k `SHOW_RULER`: lay a full row of ruler chars above the header.
        if layout.show_ruler {
            let ch = icon_str(config, "ruler").unwrap_or_else(|| "\u{2500}".to_string());
            let style = config
                .segment("ruler")
                .effective_style(None, &config.defaults);
            let width = display_width(&ch).max(1);
            let n = cols / width;
            out.push(paint(&ch.repeat(n), &style));
        }
        out.extend(
            (0..lines)
                .map(|i| {
                    let left_row = left.get(i).map(Vec::as_slice);
                    let right_row = right.get(i).map(Vec::as_slice);
                    let render_side = |budget: Option<usize>| {
                        let l = left_row
                            .map(|seg| self.row(seg, false, budget))
                            .unwrap_or_default();
                        let r = right_row
                            .map(|seg| self.row(seg, true, budget))
                            .unwrap_or_default();
                        (l, r)
                    };
                    let (prefix, suffix) = if i == 0 {
                        (&frame.first_prefix, &frame.first_suffix)
                    } else {
                        (&frame.newline_prefix, &frame.newline_suffix)
                    };
                    let pre_w = display_width(&prefix.text);
                    let suf_w = display_width(&suffix.text);
                    let mut row = String::new();
                    if !prefix.text.is_empty() {
                        row.push_str(&paint(&prefix.text, &config.frame_piece_style(prefix)));
                    }
                    // Right-align budget = cols - prefix width - suffix width, or the frame
                    // widens the row and pushes the suffix to the next line.
                    let body_budget = cols.saturating_sub(pre_w + suf_w);
                    let seps = &config.separators;
                    let indent = config.layout.right_indent;
                    // Width of each column (the right one including its start separator), used
                    // to decide whether the whole row fits. p10k's test is stricter than "just
                    // fits": content + indent + 1 must fit (measured: the same content is
                    // hidden at 33 cols and shown at 34, while the content is only 32 cols
                    // wide). Passing cols 0 for the right column alone skips the gap
                    // calculation.
                    let left_w = |l: &[SegmentText]| {
                        display_width(&assemble_row(l, &[], body_budget, seps, 0))
                    };
                    let right_w =
                        |r: &[SegmentText]| display_width(&assemble_row(&[], r, 0, seps, 0));
                    let fits = |lw: usize, rw: usize| lw + rw + indent < body_budget;
                    // Pass 1: no shortening at all (p10k leaves it as-is when it fits).
                    // Shortening budgets always use the **unshortened** width, otherwise
                    // shortening from an already shortened width would shorten less than
                    // needed.
                    let (l0, r0) = render_side(None);
                    let (lw0, rw0) = (left_w(&l0), right_w(&r0));
                    let body = if fits(lw0, rw0) {
                        assemble_row(&l0, &r0, body_budget, seps, indent)
                    } else {
                        // Pass 2: shorten dir by the number of overflowing columns (narrow
                        // terminals shorten, wide terminals do not). Only `dir` depends on the
                        // budget, so the other segments reuse the first pass result.
                        let overflow = (lw0 + rw0 + indent + 1).saturating_sub(body_budget);
                        let shorten =
                            |row: Option<&[Element]>, parts: &[SegmentText], right: bool| {
                                row.map(|elements| {
                                    self.shorten_row(elements, right, parts.to_vec(), overflow)
                                })
                                .unwrap_or_default()
                            };
                        let l1 = shorten(left_row, &l0, false);
                        let r1 = shorten(right_row, &r0, true);
                        let (lw1, rw1) = (left_w(&l1), right_w(&r1));
                        if fits(lw1, rw1) {
                            assemble_row(&l1, &r1, body_budget, seps, indent)
                        } else {
                            // Pass 3: still does not fit after shortening → drop the whole
                            // right column (gap included). p10k omits the right column when the
                            // width is insufficient instead of letting it overflow. The left
                            // column is shortened again by its overflow over the unshortened
                            // width.
                            let budget = lw0.saturating_sub(body_budget);
                            let l2 = left_row
                                .map(|elements| self.shorten_row(elements, false, l0, budget))
                                .unwrap_or_default();
                            assemble_row(&l2, &[], body_budget, seps, indent)
                        }
                    };
                    row.push_str(&body);
                    if !suffix.text.is_empty() {
                        row.push_str(&paint(&suffix.text, &config.frame_piece_style(suffix)));
                    }
                    row
                })
                .collect::<Vec<String>>(),
        );
        out
    }

    /// Render one row (one side of one header line).
    fn row(
        &self,
        elements: &[Element],
        right: bool,
        dir_budget: Option<usize>,
    ) -> Vec<SegmentText> {
        let config = self.config;
        elements
            .iter()
            .map(|el| match el {
                Element::Seg(name) => self.segment(name, right, dir_budget, false),
                Element::Joined(name) => self.segment(name, right, dir_budget, true),
                Element::Text(t) => {
                    let text = config.segment("text");
                    let mut style = text.effective_style(None, &config.defaults);
                    // `text` inherits fg and bold from defaults, never the background: the
                    // terminal's own background shows through unless the `text` segment sets a
                    // bg.
                    if text.style.bg == Color::Default {
                        style.bg = Color::Default;
                    }
                    SegmentText {
                        text: paint(&expand_env(t), &style),
                        style,
                        padded: false,
                        joined_left: false,
                    }
                }
            })
            .collect()
    }

    /// Re-render only the segments that depend on the shortening budget (`dir`). Every other
    /// segment's text is independent of the budget, so the shortening passes reuse the first
    /// pass result instead of rendering the whole row again.
    fn shorten_row(
        &self,
        elements: &[Element],
        right: bool,
        mut parts: Vec<SegmentText>,
        dir_budget: usize,
    ) -> Vec<SegmentText> {
        for (el, part) in elements.iter().zip(parts.iter_mut()) {
            let (name, joined) = match el {
                Element::Seg(name) => (name.as_str(), false),
                Element::Joined(name) => (name.as_str(), true),
                Element::Text(_) => continue,
            };
            if registry::find(name).is_some_and(|meta| meta.render.width_dependent()) {
                *part = self.segment(name, right, Some(dir_budget), joined);
            }
        }
        parts
    }

    /// Render one segment: evaluate the segment value, then compose it into the row part.
    fn segment(
        &self,
        name: &str,
        right: bool,
        dir_budget: Option<usize>,
        joined_left: bool,
    ) -> SegmentText {
        let seg = self.config.segment(name);
        match self.evaluate(name, seg, dir_budget) {
            Some(value) => self.compose(seg, value, right, joined_left),
            None => SegmentText {
                text: String::new(),
                style: seg.effective_style(None, &self.config.defaults),
                padded: false,
                joined_left,
            },
        }
    }

    /// The segment's contribution: painted text and the icon decision.
    fn evaluate(
        &self,
        name: &str,
        seg: &'a Segment,
        dir_budget: Option<usize>,
    ) -> Option<SegmentValue> {
        let meta = registry::find(name)?;
        let ctx = Ctx {
            config: self.config,
            segment: seg,
            info: self.info,
            vcs: self.vcs,
            dir_budget,
        };
        Some(meta.render.render(&ctx))
    }

    /// Compose the value into the row part: icon, attach slots, prefix/suffix, padding and the
    /// leading space.
    fn compose(
        &self,
        seg: &'a Segment,
        value: SegmentValue,
        right: bool,
        joined_left: bool,
    ) -> SegmentText {
        let config = self.config;
        let info = self.info;
        let vcs = self.vcs;
        let style = seg.effective_style(None, &config.defaults);
        // Leading space is added when the effective background is non-transparent (inheritance
        // chain below); transparent segments get none—matching p10k lean, no space at line
        // start. Effective segment background: the segment's own → global `defaults.bg`
        // (p10k's `POWERLEVEL9K_BACKGROUND`) → transparent when neither (the terminal's own
        // color). The `effective_style` inheritance already does this layer; the bg here is the
        // result.
        let has_real_bg = style.bg != Color::Default;
        let text = value.text;
        // Segment icon: config `icon` (auto-overrides the applicable mode by char class) →
        // otherwise the built-in default table.
        // The space after the icon (p10k `LEFT_MIDDLE_WHITESPACE`) is added only when the
        // segment has both an icon and content—an icon-only segment (e.g. os_icon) gets none,
        // or it would stack with the trailing padding into two spaces.
        // Segments with empty text: only "icon is content" segments (env indicators, os badges)
        // draw the icon; the rest hide entirely—matching p10k: no vcs icon outside a git repo,
        // no gear when jobs=0.
        // Icon: the segment decides (key, vcs remote, dir class, os badge); the renderer
        // resolves the glyph for the mode.
        let icon = resolve_icon_choice(config, &value.icon, info, vcs);
        // Icon color: `visual-identifier-color` (p10k `SEG_VISUAL_IDENTIFIER_COLOR`) wins; the
        // vcs segment falls back to `clean-foreground` when unset—the icon indicates the repo
        // and p10k also defaults it to green (icons do not inherit the segment default, or the
        // default theme would render them in the terminal default color). `status` falls back
        // to the foreground of its state.
        let icon_style = if seg.prop("visual-identifier-color").is_some() {
            prop_style(seg, &style, "visual-identifier-color")
        } else {
            match value.icon_style {
                IconStyle::Segment => style.clone(),
                IconStyle::VcsClean => prop_style(seg, &style, "clean-foreground"),
                IconStyle::Status => prop_style(seg, &style, status_foreground(info)),
            }
        };
        // Icon placement: the column decides by default (left column puts the icon first, the
        // right column puts it last, so both read outward from the line).
        // `icon-before-content` overrides that per segment, which is p10k's
        // `POWERLEVEL9K_<SEG>_ICON_BEFORE_CONTENT`.
        let icon_first = match seg.prop("icon-before-content") {
            Some(crate::config::Prop::Bool(b)) => *b,
            _ => !right,
        };
        let icon_text = match icon {
            Some(ic) if text.is_empty() => {
                if value.icon.is_content() {
                    paint(&ic, &icon_style)
                } else {
                    String::new()
                }
            }
            // The space between icon and content (p10k `LEFT_MIDDLE_WHITESPACE` /
            // `RIGHT_MIDDLE_WHITESPACE`) sits between them. Its width comes from
            // `icon-padding` (p10k `POWERLEVEL9K_ICON_PADDING`): one space by default, two with
            // `moderate`. It goes on the content side, so the icon stays flush against the
            // segment's outer edge.
            Some(ic) => {
                let pad = config.icon_padding.spaces();
                if icon_first {
                    paint(&format!("{ic}{pad}"), &icon_style)
                } else {
                    // The icon comes after the content: the middle whitespace stays one space
                    // before the icon, and the extra `moderate` space goes to the outer side.
                    let extra = if pad.len() > 1 { " " } else { "" };
                    paint(&format!(" {ic}{extra}"), &icon_style)
                }
            }
            None => String::new(),
        };
        // Attach text slots: left (before the icon) / middle (between icon and content,
        // requires both) / right (after the content). Concatenated only, never separate
        // blocks; fg defaults to the segment style. text-middle always sits right between icon
        // and content: with the left column's icon first → middle after the icon; with the
        // right column's icon last → middle between content and icon.
        // Segment prefix/suffix (p10k `SEG_PREFIX`/`SEG_SUFFIX`, e.g. vcs `on `, exec `took `):
        // drawn at the very start/end of the segment, colored with the segment style.
        // p10k segment whitespace (`LEFT_LEFT_WHITESPACE` / `LEFT_RIGHT_WHITESPACE`, one space
        // each by default): a space around the segment content so both sides of a separator
        // (subsep/arrow) have whitespace instead of touching. Segments without a background get
        // no leading space—matching p10k lean, nothing at line start.
        // The segment body (without the surrounding whitespace).
        let mut body = String::new();
        if let Some(p) = &seg.prefix {
            body.push_str(&paint(p, &style));
        }
        if let Some(l) = &seg.text_left {
            body.push_str(&paint_attach(&style, l));
        }
        let middle = if !icon_text.is_empty() && !text.is_empty() {
            seg.text_middle
                .as_ref()
                .map(|m| paint_attach(&style, m))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if icon_first {
            body.push_str(&icon_text);
            body.push_str(&middle);
            body.push_str(&text);
        } else {
            body.push_str(&text);
            body.push_str(&middle);
            body.push_str(&icon_text);
        }
        if let Some(r) = &seg.text_right {
            body.push_str(&paint_attach(&style, r));
        }
        if let Some(s) = &seg.suffix {
            body.push_str(&paint(s, &style));
        }
        // A conditional segment with an unmet condition has nothing to show → hide it
        // entirely, leaving no side whitespace.
        if body.is_empty() {
            return SegmentText {
                text: String::new(),
                style,
                padded: false,
                joined_left,
            };
        }
        let mut out = String::new();
        // Leading space: always for segments with a background; unconditional for right-column
        // segments, which are always preceded by a separator, and p10k's separator is followed
        // by a left_space. Transparent segments at the start of the left column get none. A
        // joined segment gets none: p10k emits no left_space for it, and the previous segment's
        // trailing space is the only gap.
        if (has_real_bg || right) && !joined_left {
            out.push_str(&paint(" ", &style));
        }
        out.push_str(&body);
        out.push_str(&paint(" ", &style));
        SegmentText {
            text: out,
            style,
            padded: true,
            joined_left,
        }
    }
}

pub fn render_header_lines(
    config: &Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    cols: usize,
) -> Vec<String> {
    Renderer::new(config, info, vcs, cols).header_lines()
}

#[cfg(test)]
mod tests {
    use super::env::*;
    use super::icons::os_icon;
    use super::segment::Icon;
    use super::*;
    use crate::config::Segment;

    fn info(cwd: &str, code: Option<i32>) -> HeaderInfo {
        HeaderInfo {
            exit_code: code,
            cwd: cwd.to_string(),
            exec_seconds: 0.0,
            exec_ms: None,
            jobs: 0,
            history: 0,
            pipestatus: Vec::new(),
        }
    }

    /// Last `38;5;N` foreground code before `needle`: asserts which color some text used,
    /// unaffected by background/bold escapes in between.
    fn fg_before(hay: &str, needle: &str) -> Option<String> {
        let i = hay.find(needle)?;
        let head = &hay[..i];
        let j = head.rfind("[38;5;")?;
        let rest = &head[j + 6..];
        Some(rest[..rest.find('m')?].to_string())
    }

    /// Strips SGR sequences, leaving visible characters.
    fn strip_sgr(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c == '\u{1b}' {
                for d in it.by_ref() {
                    if d == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// A `_joined` segment shares a block with the one to its left: no `segment` arrow and no
    /// `sub` line between them, even with different backgrounds (p10k's `elif (( joined ))`,
    /// `p10k.zsh:681-691`).
    #[test]
    fn joined_segment_draws_no_separator() {
        let seps = crate::config::Separators {
            segment: ">".into(),
            sub: "|".into(),
            end: String::new(),
            ..Default::default()
        };
        let seg = |text: &str, bg: u8, joined_left: bool| SegmentText {
            text: text.to_string(),
            style: Style {
                bg: Color::Xterm(bg),
                ..Default::default()
            },
            padded: true,
            joined_left,
        };

        let plain = assemble_row(&[seg("A", 1, false), seg("B", 2, false)], &[], 0, &seps, 0);
        assert!(
            plain.contains('>'),
            "different backgrounds without `_joined` should draw the segment arrow, got {plain:?}"
        );

        let joined = assemble_row(&[seg("A", 1, false), seg("B", 2, true)], &[], 0, &seps, 0);
        assert!(
            !joined.contains('>'),
            "`_joined` should suppress the segment arrow, got {joined:?}"
        );
        assert!(
            !joined.contains('|'),
            "`_joined` should suppress the sub line too, got {joined:?}"
        );
    }

    /// A `_joined` segment shares the block with its left neighbor: p10k emits no left_space
    /// for it, so the only space between the two segments is the previous segment's trailing
    /// space.
    #[test]
    fn joined_segment_has_no_leading_space() {
        let mut cfg = Config::default();
        cfg.segments.insert(
            "vcs".into(),
            crate::config::Segment {
                content: Some("X".into()),
                ..Default::default()
            },
        );
        let plain =
            Renderer::new(&cfg, &info("/tmp", None), None, 0).segment("vcs", true, None, false);
        let joined =
            Renderer::new(&cfg, &info("/tmp", None), None, 0).segment("vcs", true, None, true);
        let p = strip_sgr(&plain.text);
        let j = strip_sgr(&joined.text);
        assert!(
            p.starts_with(' '),
            "a plain right-column segment starts with its padding, got {p:?}"
        );
        assert!(
            !j.starts_with(' '),
            "a joined segment starts with its content, got {j:?}"
        );
        assert_eq!(display_width(&p), display_width(&j) + 1);
    }

    /// A `_joined` suffix on the first segment has no left neighbor to join: it keeps the
    /// normal leading space of a line-start segment (docs/config-language.md).
    #[test]
    fn joined_first_segment_keeps_its_leading_space() {
        let seps = crate::config::Separators {
            segment: ">".into(),
            sub: "|".into(),
            end: String::new(),
            left_tail: String::new(),
            ..Default::default()
        };
        let seg = |text: &str, bg: u8, joined_left: bool| SegmentText {
            text: text.to_string(),
            style: Style {
                bg: Color::Xterm(bg),
                ..Default::default()
            },
            padded: true,
            joined_left,
        };
        let out = assemble_row(&[seg("A", 1, true), seg("B", 2, false)], &[], 0, &seps, 0);
        let visible = strip_sgr(&out);
        let leading = visible.chars().take_while(|&c| c == ' ').count();
        assert_eq!(
            leading, 1,
            "the first joined segment should keep one leading space, got {visible:?}"
        );
        assert!(
            out.contains("\x1b[48;5;1m "),
            "the leading space should carry the segment background, got {out:?}"
        );
    }

    /// The `_joined` suffix only affects the boundary to the left. Prefix, suffix and the
    /// attach slots keep their normal order inside the segment.
    #[test]
    fn joined_segment_keeps_attach_slots_and_prefix_suffix() {
        let mut cfg = Config::default();
        cfg.segments.insert(
            "vcs".into(),
            crate::config::Segment {
                content: Some("X".into()),
                prefix: Some("on ".into()),
                suffix: Some("!".into()),
                text_left: Some(crate::config::AttachText {
                    text: "L".into(),
                    fg: None,
                }),
                text_middle: Some(crate::config::AttachText {
                    text: "M".into(),
                    fg: None,
                }),
                text_right: Some(crate::config::AttachText {
                    text: "R".into(),
                    fg: None,
                }),
                ..Default::default()
            },
        );
        let seg =
            Renderer::new(&cfg, &info("/tmp", None), None, 0).segment("vcs", false, None, true);
        let text = strip_sgr(&seg.text);
        assert!(
            text.starts_with("on L"),
            "a joined segment keeps its prefix and text-left, got {text:?}"
        );
        let li = text.find('L').expect("text-left");
        let mi = text.find('M').expect("text-middle");
        let xi = text.find('X').expect("content");
        let ri = text.find('R').expect("text-right");
        let si = text.find('!').expect("suffix");
        assert!(
            li < mi && mi < xi && xi < ri && ri < si,
            "attach slots and suffix keep their order, got {text:?}"
        );
    }

    /// An unknown segment name is ignored: p10k looks up `prompt_<name>` and has no fallback,
    /// so a name without an implementation renders nothing, its configured content included.
    #[test]
    fn unknown_segment_renders_nothing() {
        let mut cfg = Config::default();
        cfg.layout.left = vec![vec![Element::Seg("nosuch".into())]];
        cfg.segments.insert(
            "nosuch".into(),
            crate::config::Segment {
                content: Some("secret".into()),
                ..Default::default()
            },
        );
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains("secret"),
            "unknown segments are ignored, got {h:?}"
        );
    }

    /// Icon-only segments (`os_icon`, `os`) have no text and no `match` arm: the icon logic
    /// draws the visual identifier. They must survive the unknown-segment check.
    #[test]
    fn icon_only_segment_renders_its_icon() {
        let cfg = Config::parse("layout { left { line { os_icon } } }").unwrap();
        let icon = os_icon(&cfg.mode);
        assert!(!icon.is_empty(), "the os icon has a fallback");
        let h = strip_sgr(&render_header_lines(&cfg, &info("/tmp", None), None, 80)[0]);
        assert!(
            h.contains(&icon),
            "os_icon should render its icon, got {h:?}"
        );
    }

    /// The icon padding sits between the icon and the content. With the icon after the content
    /// (right column), the extra `moderate` space lands on the outer side of the icon; the
    /// middle whitespace before the icon stays one space.
    #[test]
    fn right_column_icon_padding_lands_outside_the_icon() {
        let draw = |padding: Option<&str>| {
            let prefix = padding
                .map(|p| format!("icon-padding \"{p}\"\n"))
                .unwrap_or_default();
            let cfg = Config::parse(&format!(
                "{prefix}layout {{\n    right {{ line {{ dir }} }}\n}}\n"
            ))
            .unwrap();
            strip_sgr(&render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80)[0])
        };
        for (padding, trailing) in [(None, 1usize), (Some("moderate"), 2)] {
            let line = draw(padding);
            let chars: Vec<char> = line.chars().collect();
            let i = chars
                .iter()
                .position(|&c| c == '\u{f115}')
                .unwrap_or_else(|| panic!("folder icon should render, got {line:?}"));
            assert!(
                i >= 2 && chars[i - 1] == ' ' && chars[i - 2] != ' ',
                "{padding:?}: the icon should have one middle space before it, got {line:?}"
            );
            let after = chars[i + 1..]
                .iter()
                .rev()
                .take_while(|&&c| c == ' ')
                .count();
            assert_eq!(
                after, trailing,
                "{padding:?}: wrong space count after the icon, got {line:?}"
            );
        }
    }

    #[test]
    fn segment_prefix_and_suffix_render() {
        // p10k's SEG_PREFIX/SEG_SUFFIX (vcs `on `, exec `took `):
        // drawn at the very start/end of the segment.
        let mut cfg = Config::default();
        cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
        let s = crate::config::Segment {
            content: Some("main".into()),
            prefix: Some("on ".into()),
            suffix: Some("!".into()),
            ..Default::default()
        };
        cfg.segments.insert("vcs".into(), s);
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains("on "), "prefix should render, got {h:?}");
        assert!(h.contains("main"), "content should render, got {h:?}");
        let after = h.split("main").nth(1).unwrap_or("");
        assert!(
            after.contains('!'),
            "suffix should come after the content, got {h:?}"
        );
    }

    #[test]
    fn vcs_changeset_shows_hash_when_detached_or_enabled() {
        let mk = |branch: &str, show: bool| {
            let mut cfg = Config::default();
            cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
            let mut s = crate::config::Segment::default();
            if show {
                s.props
                    .insert("show-changeset".into(), crate::config::Prop::Bool(true));
            }
            cfg.segments.insert("vcs".into(), s);
            let v = GitStatus {
                branch: branch.into(),
                commit: "aa2fdd09deadbeefaa2fdd09deadbeefaa2fdd09".into(),
                staged: 0,
                unstaged: 0,
                conflicted: 0,
                untracked: 0,
                ahead: 0,
                behind: 0,
                push_ahead: 0,
                push_behind: 0,
                stashes: 0,
                action: String::new(),
                tag: String::new(),
                remote_branch: String::new(),
                commit_summary: String::new(),
                index_size: 0,
                remote_url: String::new(),
            };
            render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\n")
        };
        // No local branch (detached HEAD) → the first 8 chars show automatically.
        assert!(
            mk("", false).contains("aa2fdd09"),
            "detached should show the hash"
        );
        // Branch present and not enabled → hidden.
        assert!(
            !mk("main", false).contains("aa2fdd09"),
            "branch present hides it by default"
        );
        // Explicitly enabled → shown.
        assert!(
            mk("main", true).contains("aa2fdd09"),
            "show-changeset should show it"
        );
    }

    #[test]
    fn vcs_branch_shortening_and_symbols() {
        // p10k VCS_SHORTEN_*: shortening needs both length and min-length; symbols are
        // configurable.
        let v = GitStatus {
            branch: "feature/long-branch".into(),
            commit: String::new(),
            staged: 2,
            unstaged: 0,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        let mut cfg = Config::default();
        cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
        let mut s = crate::config::Segment::default();
        s.props
            .insert("shorten-length".into(), crate::config::Prop::Int(4));
        s.props
            .insert("shorten-min-length".into(), crate::config::Prop::Int(6));
        s.props.insert(
            "shorten-strategy".into(),
            crate::config::Prop::Str("truncate_middle".into()),
        );
        s.props
            .insert("staged-symbol".into(), crate::config::Prop::Str("S".into()));
        cfg.segments.insert("vcs".into(), s);
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\n");
        assert!(
            h.contains("feat\u{2026}anch"),
            "branch should shorten in the middle, got {h:?}"
        );
        assert!(
            h.contains("S2"),
            "staged symbol should be configurable, got {h:?}"
        );

        // Only length set, min-length missing → no shortening (p10k requires both).
        let mut cfg2 = Config::default();
        cfg2.layout.left = vec![vec![Element::Seg("vcs".into())]];
        let mut s2 = crate::config::Segment::default();
        s2.props
            .insert("shorten-length".into(), crate::config::Prop::Int(4));
        cfg2.segments.insert("vcs".into(), s2);
        let h2 = render_header_lines(&cfg2, &info("/tmp", None), Some(&v), 80).join("\n");
        assert!(
            h2.contains("feature/long-branch"),
            "missing min-length should not shorten, got {h2:?}"
        );
    }

    #[test]
    fn exec_segment_shows_p10k_hourglass_icon() {
        // p10k `EXECUTION_TIME_ICON`: U+F252 in the nerdfont tier; p10k itself has nothing
        // in compatible/ascii, so only the two nerdfont tiers draw the icon.
        fn render(mode: crate::config::IconMode) -> String {
            let mut cfg = Config {
                mode,
                ..Config::default()
            };
            cfg.layout.right = vec![vec![Element::Seg("command_execution_time".into())]];
            let mut e = crate::config::Segment::default();
            e.props
                .insert("threshold-seconds".into(), crate::config::Prop::Int(3));
            e.props
                .insert("precision".into(), crate::config::Prop::Int(0));
            cfg.segments.insert("command_execution_time".into(), e);
            let i = HeaderInfo {
                exec_seconds: 3.5,
                exec_ms: None,
                ..info("/tmp", None)
            };
            render_header_lines(&cfg, &i, None, 80).join("\n")
        }
        let nf = render(crate::config::IconMode::NerdfontComplete);
        assert!(
            nf.contains("\u{f252}"),
            "nerdfont mode should have the hourglass icon, got {nf:?}"
        );
        assert!(
            nf.contains("4s"),
            "content should still be the duration, got {nf:?}"
        );
        let other = [
            crate::config::IconMode::NerdfontFontconfig,
            crate::config::IconMode::Compatible,
            crate::config::IconMode::Ascii,
        ];
        for mode in other {
            let h = render(mode.clone());
            if mode == crate::config::IconMode::NerdfontFontconfig {
                assert!(
                    h.contains("\u{f252}"),
                    "fontconfig mode has the same glyph, got {h:?}"
                );
            } else {
                assert!(
                    !h.contains('\u{f252}'),
                    "{mode:?} mode has no hourglass, got {h:?}"
                );
            }
        }
    }

    #[test]
    fn vcs_wip_remote_branch_push_counts_and_dash() {
        /// Strips SGR, leaving visible text only.
        fn plain(h: &str) -> String {
            let mut out = String::new();
            let mut it = h.chars().peekable();
            while let Some(c) = it.next() {
                if c == '\u{1b}' {
                    for d in it.by_ref() {
                        if d == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        fn render(v: &GitStatus) -> String {
            let mut cfg = Config::default();
            cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
            let mut s = crate::config::Segment::default();
            s.props
                .insert("clean-foreground".into(), crate::config::Prop::Int(76));
            s.props
                .insert("meta-foreground".into(), crate::config::Prop::Int(246));
            cfg.segments.insert("vcs".into(), s);
            plain(&render_header_lines(&cfg, &info("/tmp", None), Some(v), 120).join("\n"))
        }
        let base = GitStatus {
            branch: "main".into(),
            commit: "aa2fdd09deadbeef".into(),
            staged: 0,
            unstaged: 0,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        // The branch name is directly followed by `:remote-name` (p10k adds no space); hidden
        // when identical to the local name.
        let same = GitStatus {
            remote_branch: "main".into(),
            ..base.clone()
        };
        let same_out = render(&same);
        assert!(
            !same_out.contains(':'),
            "remote branch matching the local name should not be repeated, got {same_out:?}"
        );
        let diff = GitStatus {
            remote_branch: "origin/main".into(),
            ..base.clone()
        };
        assert!(render(&diff).contains("main:origin/main"));
        // wip: only a standalone wip/WIP in the commit summary counts (not one surrounded by
        // alphanumerics).
        for (summary, want) in [
            ("wip", true),
            ("WIP: fix", true),
            ("fix (WIP)", true),
            ("wipe out", false),
            ("swipe", false),
            ("", false),
        ] {
            let v = GitStatus {
                commit_summary: summary.into(),
                ..base.clone()
            };
            let got = render(&v).contains("main wip");
            assert_eq!(got, want, "wip detection is wrong for summary={summary:?}");
        }
        // The push remote's ⇠/⇢ follow directly after ahead/behind.
        let push = GitStatus {
            ahead: 1,
            push_behind: 2,
            push_ahead: 3,
            stashes: 4,
            ..base.clone()
        };
        let out = render(&push);
        assert!(
            out.contains("main ⇡1 ⇠2⇢3 *4"),
            "push count position/symbols are wrong, got {out:?}"
        );
        // With max-index-size-dirty set and a larger index → draw `─` at the end; without it,
        // nothing.
        let dash = GitStatus {
            index_size: 500,
            ..base.clone()
        };
        fn render_with_cap(v: &GitStatus, cap: Option<i64>) -> String {
            let mut cfg = Config::default();
            cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
            let mut s = crate::config::Segment::default();
            if let Some(cap) = cap {
                s.props
                    .insert("max-index-size-dirty".into(), crate::config::Prop::Int(cap));
            }
            cfg.segments.insert("vcs".into(), s);
            plain(&render_header_lines(&cfg, &info("/tmp", None), Some(v), 120).join("\n"))
        }
        assert!(
            render_with_cap(&dash, Some(100))
                .trim_end()
                .ends_with("main ─"),
            "index over the cap should draw ─, got {:?}",
            render_with_cap(&dash, Some(100))
        );
        assert!(
            !render_with_cap(&dash, Some(1000)).contains('─'),
            "index not over the cap should not draw ─"
        );
        assert!(
            !render_with_cap(&dash, None).contains('─'),
            "no cap configured should not draw ─"
        );
    }

    #[test]
    fn vcs_detached_and_tag_match_p10k() {
        // p10k's formatting function: no branch with a tag → `${meta}#${clean}tag`,
        // no branch and no tag → `${meta}@${clean}commit[1,8]`.
        fn render(v: &GitStatus) -> String {
            let mut cfg = Config::default();
            cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
            let mut s = crate::config::Segment::default();
            s.props
                .insert("clean-foreground".into(), crate::config::Prop::Int(76));
            s.props
                .insert("meta-foreground".into(), crate::config::Prop::Int(246));
            cfg.segments.insert("vcs".into(), s);
            render_header_lines(&cfg, &info("/tmp", None), Some(v), 120).join("\n")
        }
        let base = GitStatus {
            branch: String::new(),
            commit: "aa2fdd09deadbeef".into(),
            staged: 0,
            unstaged: 0,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        let h = render(&base);
        assert_eq!(
            fg_before(&h, "@").as_deref(),
            Some("246"),
            "@ should use the meta color"
        );
        assert_eq!(
            fg_before(&h, "aa2fdd09").as_deref(),
            Some("76"),
            "hash should use the clean color"
        );
        assert!(
            h.contains("aa2fdd09"),
            "hash should take the first 8 chars, got {h:?}"
        );
        assert!(
            !h.contains("deadbeef"),
            "should not exceed 8 chars, got {h:?}"
        );
        // A tag present → show #tag and no hash.
        let tagged = GitStatus {
            tag: "v1.2.3".into(),
            ..base
        };
        let h2 = render(&tagged);
        assert_eq!(
            fg_before(&h2, "#").as_deref(),
            Some("246"),
            "# should use the meta color"
        );
        assert_eq!(
            fg_before(&h2, "v1.2.3").as_deref(),
            Some("76"),
            "tag name should use the clean color"
        );
        assert!(
            !h2.contains("aa2fdd09"),
            "a tag present should not show the hash, got {h2:?}"
        );
    }

    #[test]
    fn vcs_count_order_matches_p10k_formatter() {
        /// Strips SGR sequences, leaving visible chars, to assert the count order.
        fn strip_sgr(s: &str) -> String {
            let mut out = String::new();
            let mut it = s.chars().peekable();
            while let Some(c) = it.next() {
                if c == '\u{1b}' {
                    for d in it.by_ref() {
                        if d == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        // p10k's vcs formatting function order: ⇣behind⇡ahead → *stashes → action →
        // ~conflicted → +staged → !unstaged → ?untracked; no space between ahead/behind.
        let cfg = Config::default_lean().unwrap();
        let v = GitStatus {
            branch: "main".into(),
            commit: String::new(),
            staged: 2,
            unstaged: 3,
            conflicted: 1,
            untracked: 4,
            ahead: 2,
            behind: 1,
            push_ahead: 0,
            push_behind: 0,
            stashes: 3,
            action: "merge".into(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 120).join("\n");
        let plain = strip_sgr(&h);
        let i = plain.find("main").expect("should contain the branch name");
        assert_eq!(
            plain[i + "main".len()..].trim(),
            "⇣1⇡2 *3 merge ~1 +2 !3 ?4",
            "count order/symbols should match p10k, got {plain:?}"
        );
    }

    #[test]
    fn vcs_conflicted_uses_own_color_with_modified_fallback() {
        let v = GitStatus {
            branch: "main".into(),
            commit: String::new(),
            staged: 0,
            unstaged: 1,
            conflicted: 2,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        fn render(v: &GitStatus, conflicted_fg: Option<i64>) -> String {
            let mut cfg = Config::default();
            cfg.layout.left = vec![vec![Element::Seg("vcs".into())]];
            let mut s = crate::config::Segment::default();
            s.props
                .insert("modified-foreground".into(), crate::config::Prop::Int(178));
            if let Some(fg) = conflicted_fg {
                s.props
                    .insert("conflicted-foreground".into(), crate::config::Prop::Int(fg));
            }
            cfg.segments.insert("vcs".into(), s);
            render_header_lines(&cfg, &info("/tmp", None), Some(v), 80).join("\n")
        }
        // p10k classic: `~N` (conflicted) uses %196F, `!N` (unstaged) uses %178F.
        let h = render(&v, Some(196));
        assert_eq!(
            fg_before(&h, "~2").as_deref(),
            Some("196"),
            "conflicted should use 196"
        );
        assert_eq!(
            fg_before(&h, "!1").as_deref(),
            Some("178"),
            "unstaged should still use 178"
        );
        // conflicted-foreground unset → fall back to modified, keeping the old behavior.
        let h2 = render(&v, None);
        assert_eq!(
            fg_before(&h2, "~2").as_deref(),
            Some("178"),
            "the default should fall back to 178"
        );
    }

    #[test]
    fn exec_duration_formats_match_p10k() {
        // <60s: precision=0 rounds, >0 keeps decimals (p10k default 2).
        assert_eq!(format_duration(3.4, 0, false), "3s");
        assert_eq!(format_duration(3.44, 2, false), "3.44s");
        // >=60s: multi-level with spaces, up to days.
        assert_eq!(format_duration(65.0, 0, false), "1m 5s");
        assert_eq!(format_duration(3723.0, 0, false), "1h 2m 3s");
        assert_eq!(format_duration(90065.0, 0, false), "1d 1h 1m 5s");
        // H:M:S variant.
        assert_eq!(format_duration(65.0, 0, true), "01:05");
        assert_eq!(format_duration(3723.0, 0, true), "01:02:03");
        assert_eq!(format_duration(36005.0, 0, true), "10:00:05");
    }

    #[test]
    fn time_segment_honors_12h_format() {
        let mut cfg = Config::default();
        let mut t = crate::config::Segment::default();
        t.props
            .insert("time-format".into(), crate::config::Prop::Str("12h".into()));
        cfg.segments.insert("time".into(), t);
        let seg = cfg.segment("time");
        let s24 = time_text(&crate::config::Segment::default());
        let s12 = time_text(seg);
        // 24h is HH:MM:SS; 12h is HH:MM:SS AM/PM.
        assert_eq!(s24.len(), 8, "24h should be 8 chars, got {s24:?}");
        assert_eq!(s12.len(), 11, "12h should be 11 chars, got {s12:?}");
        assert!(
            s12.ends_with(" AM") || s12.ends_with(" PM"),
            "12h should end with AM/PM, got {s12:?}"
        );
    }

    #[test]
    fn tail_separators_render_at_row_edges() {
        let mut cfg = Config::default();
        cfg.layout.left = vec![vec![Element::Seg("dir".into())]];
        cfg.segments.insert("dir".into(), {
            let mut d = crate::config::Segment::default();
            d.style.bg = Color::Xterm(31);
            d
        });
        cfg.separators.left_tail = "\u{e0b2}".into();
        cfg.separators.right_tail = "\u{e0b0}".into();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        // The left column's first segment is preceded by left_tail (), followed by the directory content.
        assert!(
            h.contains("\u{e0b2}"),
            "should draw the left tail, got {h:?}"
        );
    }

    #[test]
    fn renders_pure_text_header_with_right_align() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("\r\n");
        // The header is a single line: dir + status icon (nerdfont default ); prompt_char (❯) is not in the header.
        assert!(
            h.contains("tmp"),
            "header should contain the dir, got: {h:?}"
        );
        assert!(h.contains("\u{f00c}"), "header: {h:?}");
        assert!(
            !h.contains('❯'),
            "input line prefix ❯ is drawn by render_prompt and should not be in the header"
        );
        assert_eq!(h.split("\r\n").count(), 1, "lean header should be one line");
    }

    #[test]
    fn right_aligns_to_cols() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("\r\n");
        // The right status icon is right-aligned: gap filling makes the row's display width
        // = cols. The right column also keeps `right-indent` (default 1, same as zsh
        // `ZLE_RPROMPT_INDENT`) from the right edge.
        assert!(
            h.contains("\u{f00c}"),
            "right segment should exist, got: {h:?}"
        );
        assert_eq!(
            display_width(&h),
            80 - cfg.layout.right_indent,
            "right-aligned row width should be cols - right-indent"
        );
    }

    #[test]
    fn line_text_literal_renders() {
        // A line can interleave static text text "some text".
        let cfg = Config::parse(
            "layout {\n  left {\n    line { dir #true; text \"some text\"; vcs #true }\n  }\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            h.contains("some text"),
            "static text in line should render, got: {h:?}"
        );
    }

    #[test]
    fn line_text_expands_env() {
        // ${VAR}/$VAR in text expand to env vars.
        let home = std::env::var("HOME").unwrap_or_default();
        let cfg =
            Config::parse("layout {\n  left {\n    line { text \"home=${HOME} $USER\" }\n  }\n}")
                .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains(&home), "text should expand env vars, got: {h:?}");
    }

    /// `text` elements inherit `fg` and `bold` from `defaults`, never its background. A
    /// background the `text` segment sets itself still applies.
    #[test]
    fn text_elements_do_not_inherit_the_defaults_background() {
        let cfg = Config::parse("defaults fg=200 bg=236\nlayout {}").unwrap();
        let row = Renderer::new(&cfg, &info("/tmp", None), None, 0).row(
            &[Element::Text("hi".into())],
            false,
            None,
        );
        assert_eq!(
            row[0].style.bg,
            Color::Default,
            "text must stay transparent"
        );
        assert_eq!(row[0].style.fg, Color::Xterm(200));

        let cfg = Config::parse("defaults bg=236\nsegments { text bg=31 }\nlayout {}").unwrap();
        let row = Renderer::new(&cfg, &info("/tmp", None), None, 0).row(
            &[Element::Text("hi".into())],
            false,
            None,
        );
        assert_eq!(
            row[0].style.bg,
            Color::Xterm(31),
            "an explicit text bg applies"
        );
    }

    /// A transparent `text` element must not fall back to black.
    #[test]
    fn text_elements_stay_transparent_without_defaults_bg() {
        let cfg = Config::parse("layout {}").unwrap();
        let row = Renderer::new(&cfg, &info("/tmp", None), None, 0).row(
            &[Element::Text("hi".into())],
            false,
            None,
        );
        assert_eq!(row[0].style.bg, Color::Default);
    }

    #[test]
    fn first_header_line_gets_frame() {
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  first-prefix \"╭─\"\n  first-suffix \"─╮\"\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            h.contains("╭─"),
            "first line should have the first-prefix frame, got: {h:?}"
        );
        assert!(
            h.contains("─╮"),
            "first line should have the first-suffix frame"
        );
    }

    #[test]
    fn input_prefix_has_the_expected_text() {
        // With the frame last-prefix ╰─, the visible prefix is "╰─❯ ".
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  last-prefix \"╰─\"\n}",
        )
        .unwrap();
        let p = input_prefix(&cfg, None);
        assert_eq!(
            display_width(&p.text),
            4,
            "╰─❯ plus space should be width 4, got text={:?}",
            p.text
        );
        assert!(
            p.text.contains('╰'),
            "prefix should contain the frame last-prefix"
        );
        assert!(p.text.contains('❯'), "prefix should contain prompt_char");
        assert!(p.text.ends_with(' '), "prefix should end with a space");
    }

    #[test]
    fn vcs_state_colors_and_inner_spaces() {
        // classic(2): segment background 238, clean 76, modified 178.
        let cfg = crate::presets::classic(2);
        let v = GitStatus {
            branch: "main".into(),
            commit: String::new(),
            staged: 0,
            unstaged: 2,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\n");
        // Branch name/icon use clean-foreground (76), `!2` (unstaged) uses
        // modified-foreground (178), both on the segment background (238).
        assert!(
            h.contains("\u{1b}[38;5;76m\u{1b}[48;5;238m"),
            "branch/icon should be colored clean with the segment background, got {h:?}"
        );
        assert!(
            h.contains("\u{1b}[38;5;178m\u{1b}[48;5;238m!2"),
            "unstaged should be colored modified, got {h:?}"
        );
        // The space between branch and counts must carry the segment background (48;5;238); a
        // bare space would show a black seam.
        let after_branch = h
            .split("main")
            .nth(1)
            .expect("should contain the branch name");
        let gap = after_branch.split('!').next().unwrap_or("");
        assert!(
            gap.contains("48;5;238"),
            "spaces between vcs content should carry the segment background, got {gap:?}"
        );
    }

    #[test]
    fn vcs_counts_appear() {
        let cfg = Config::default_lean().unwrap();
        let v = GitStatus {
            branch: "master".into(),
            commit: String::new(),
            staged: 1,
            unstaged: 2,
            conflicted: 0,
            untracked: 3,
            ahead: 1,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\r\n");
        assert!(h.contains("master"));
        assert!(h.contains("+1"));
        // p10k's count symbols: staged +, unstaged !, conflicted ~, untracked ?,
        // ahead ⇡, behind ⇣, stashes *.
        assert!(h.contains("!2"));
        assert!(h.contains("?3"));
        assert!(h.contains("⇡1"), "ahead should use ⇡, got {h:?}");
    }

    #[test]
    fn background_blocks_and_powerline_arrow() {
        // dir/vcs both have backgrounds and they differ → draw the ``.
        let cfg = Config::parse(
            "layout { left { line { dir #true; vcs #true } } }\n\
             segments { dir { bg 39 } }\n",
        )
        .unwrap();
        let mut cfg = cfg;
        cfg.segments.get_mut("dir").unwrap().style.bg = Color::Xterm(39);
        cfg.segments.get_mut("dir").unwrap().style.fg = Color::Xterm(0);
        cfg.segments.insert(
            "vcs".into(),
            crate::config::Segment {
                style: crate::config::Style {
                    fg: Color::Xterm(0),
                    bg: Color::Xterm(76),
                    bold: false,
                },
                ..Default::default()
            },
        );
        let v = GitStatus {
            branch: "master".into(),
            commit: String::new(),
            staged: 0,
            unstaged: 0,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            push_ahead: 0,
            push_behind: 0,
            stashes: 0,
            action: String::new(),
            tag: String::new(),
            remote_branch: String::new(),
            commit_summary: String::new(),
            index_size: 0,
            remote_url: String::new(),
        };
        // Set a powerline arrow between segments with different backgrounds, plus the end cap.
        cfg.separators.segment = "\u{e0b0}".into();
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\r\n");
        assert!(
            h.contains("\x1b[48;5;39m"),
            "dir should have the background block 39"
        );
        assert!(
            h.contains("\x1b[48;5;76m"),
            "vcs should have the background block 76"
        );
        assert!(
            h.contains('\u{e0b0}'),
            "segments with different backgrounds should draw the configurable segment separator ()"
        );
    }

    #[test]
    fn frame_colors_pieces_with_override() {
        // The frame-level fg applies to all pieces; a child's own fg overrides it.
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             frame fg=76 {\n  first-prefix \"╭─\"\n  last-prefix \"╰─\" fg=196\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            h.contains("\x1b[38;5;76m╭─"),
            "first line prefix should use the frame-level fg, got: {h:?}"
        );
        // Input line prefix: last-prefix uses the child color (196), prompt_char ❯ is
        // uncolored by default.
        let p = input_prefix(&cfg, None);
        assert!(
            p.text.contains("\x1b[38;5;196m╰─"),
            "last-prefix child fg should override the frame-level fg, got: {:?}",
            p.text
        );
        assert!(
            p.text.contains('❯'),
            "input line prefix should still contain prompt_char ❯"
        );
    }

    #[test]
    fn prompt_char_vi_states() {
        // With VIINS/VICMD chars configured the prompt switches with the edit mode
        // (p10k's PROMPT_CHAR_{OK,ERROR}_{VIINS,VICMD,...}).
        let cfg = Config::parse(
            "layout { left { line { dir } } }\nsegments { prompt_char fg=76 {\n  state VIINS char=\"I\"\n  state VICMD char=\"C\"\n} }",
        )
        .unwrap();
        *CURRENT_VI_MODE.lock().unwrap() = "vicmd".to_string();
        assert!(
            input_prefix(&cfg, None).text.contains('C'),
            "vicmd should switch to the VICMD char"
        );
        *CURRENT_VI_MODE.lock().unwrap() = "viins".to_string();
        assert!(
            input_prefix(&cfg, None).text.contains('I'),
            "viins should switch to the VIINS char"
        );
        // A config without vi states is unaffected by the edit mode and still uses ❯/ERROR.
        *CURRENT_VI_MODE.lock().unwrap() = String::new();
        let plain = Config::default_lean().unwrap();
        assert!(input_prefix(&plain, None).text.contains('❯'));
    }

    #[test]
    fn extended_status_states_follow_p10k() {
        // p10k's STATUS_EXTENDED_STATES: $pipestatus distinguishes a pipeline from a plain
        // failure, and an exit code above 128 means death by signal.
        let cfg = Config::parse(
            "segments { prompt_char {\n\
               state \"OK_PIPE\" fg=70\n\
               state \"ERROR_PIPE\" fg=160\n\
               state \"ERROR_SIGNAL\" fg=160\n\
             } }",
        )
        .unwrap();
        let info_with = |code: i32, pipes: &[i32]| {
            let mut i = info("/tmp", Some(code));
            i.pipestatus = pipes.to_vec();
            i
        };
        // Last stage succeeded while an earlier one failed.
        assert_eq!(
            prompt_state(&cfg, Some(&info_with(0, &[0, 1]))),
            Some("OK_PIPE")
        );
        // Plain success has no state at all.
        assert_eq!(prompt_state(&cfg, Some(&info_with(0, &[0]))), None);
        // The pipeline failed.
        assert_eq!(
            prompt_state(&cfg, Some(&info_with(1, &[0, 1]))),
            Some("ERROR_PIPE")
        );
        // Killed by a signal, and not a pipeline.
        assert_eq!(
            prompt_state(&cfg, Some(&info_with(130, &[130]))),
            Some("ERROR_SIGNAL")
        );
        // Plain failure.
        assert_eq!(prompt_state(&cfg, Some(&info_with(1, &[1]))), Some("ERROR"));

        // Undeclared extended states fall back to ERROR (and OK_PIPE to no state), so a
        // config that only knows OK/ERROR keeps behaving as before.
        let plain = Config::parse("segments { prompt_char { state \"ERROR\" fg=196 } }").unwrap();
        assert_eq!(
            prompt_state(&plain, Some(&info_with(1, &[0, 1]))),
            Some("ERROR")
        );
        assert_eq!(prompt_state(&plain, Some(&info_with(0, &[0, 1]))), None);
    }

    #[test]
    fn prompt_char_configurable_with_error_state() {
        // The char prop sets the prompt char; state ERROR overrides char and color for the
        // error state (non-zero exit code).
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\">\" fg=76 {\n  state ERROR char=\"✘\" fg=196\n} }",
        )
        .unwrap();
        let ok = input_prefix(&cfg, Some(&info("/tmp", Some(0))));
        assert!(
            ok.text.contains("\x1b[38;5;76m>"),
            "ok state should show char (>) with fg=76, got: {:?}",
            ok.text
        );
        assert!(
            !ok.text.contains('❯'),
            "with char configured the default ❯ should not be used"
        );
        let err = input_prefix(&cfg, Some(&info("/tmp", Some(1))));
        assert!(
            err.text.contains("\x1b[38;5;196m✘"),
            "error state should show the state ERROR char/color, got: {:?}",
            err.text
        );
        let none = input_prefix(&cfg, None);
        assert!(
            none.text.contains('>'),
            "no exit code (first prompt) should use the ok state"
        );
    }

    #[test]
    fn transient_prompt_zsh_conditional_colors() {
        // transient is zsh-only: the engine precomputes zsh %F escapes and the conditional
        // color switch, with a space after ❯.
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char fg=76 {\n  state ERROR fg=196\n} }",
        )
        .unwrap();
        let s = transient_prompt_zsh(&cfg);
        assert_eq!(
            s, "%(?\u{1}%F{76}❯ \u{1}%F{196}❯ )%f",
            "transient should generate the zsh conditional colors, got: {s:?}"
        );
    }

    #[test]
    fn transient_prompt_zsh_default_color_no_leading_space() {
        let cfg = Config::parse("layout { left { line { dir #true } } }\nsegments { prompt_char }")
            .unwrap();
        let s = transient_prompt_zsh(&cfg);
        assert_eq!(
            s, "%(?\u{1}%f❯ \u{1}%f❯ )%f",
            "default fg should generate %f with no leading space, got: {s:?}"
        );
    }

    #[test]
    fn icon_mode_switches_default_icons() {
        // The status icon is the segment's visual identifier, not part of its text (p10k draws
        // `1 ✘`, not `✘ 1`, in the right column).
        let mut seg = Segment::default();
        seg.props
            .insert("verbose".into(), crate::config::Prop::Bool(true));
        let style = Style::default();
        assert_eq!(status_icon_key(&info("/tmp", Some(0)), &seg), Some("ok"));
        assert_eq!(status_icon_key(&info("/tmp", Some(1)), &seg), Some("error"));
        assert_eq!(status_icon_key(&info("/tmp", None), &seg), None);
        // A non-verbose success draws neither icon nor text.
        let mut quiet = Segment::default();
        quiet
            .props
            .insert("verbose".into(), crate::config::Prop::Bool(false));
        assert_eq!(status_icon_key(&info("/tmp", Some(0)), &quiet), None);
        assert!(status_text(&info("/tmp", Some(0)), &seg, &style).is_empty());
        assert!(status_text(&info("/tmp", Some(1)), &seg, &style).contains('1'));
        // Icon defaults follow mode: the dir icons are p10k's FOLDER_ICON and are empty in
        // compat/ascii; go has text in all three tiers.
        let mk = |m: &str| {
            Config::parse(&format!(
                "mode \"{m}\"\nlayout {{ left {{ line {{ dir #true }} }} }}"
            ))
            .unwrap()
        };
        assert_eq!(
            icon_str(&mk("nerdfont-complete"), "folder").unwrap(),
            "\u{f115}"
        );
        assert_eq!(icon_str(&mk("compatible"), "folder"), None);
        assert_eq!(icon_str(&mk("ascii"), "folder"), None);
        assert_eq!(
            icon_str(&mk("nerdfont-complete"), "go").unwrap(),
            "\u{e626}"
        );
        assert_eq!(icon_str(&mk("compatible"), "go").unwrap(), "Go");
        assert_eq!(icon_str(&mk("ascii"), "go").unwrap(), "go");
    }

    #[test]
    fn icon_overrides_per_mode() {
        // Top-level icon{}: an exact ascii tier overrides ascii only; all auto-detects the
        // char class.
        let nf = |extra: &str| {
            Config::parse(&format!(
                "{extra}\nlayout {{ left {{ line {{ dir #true }} }} }}"
            ))
            .unwrap()
        };
        // error with an exact ascii tier → applies to ascii mode only, nf falls back to the default .
        let c = nf("icon { error { ascii \"X\" } }");
        assert_eq!(icon_str(&c, "error").unwrap(), "\u{f00d}");
        let c = nf("mode \"ascii\"\nicon { error { ascii \"X\" } }");
        assert_eq!(icon_str(&c, "error").unwrap(), "X");
        // all "✔" (standard Unicode) → overrides nf+compat; ascii falls back to the default ok.
        let c = nf("icon { ok { all \"\u{2714}\" } }");
        assert_eq!(icon_str(&c, "ok").unwrap(), "\u{2714}");
        let c = nf("mode \"compatible\"\nicon { ok { all \"\u{2714}\" } }");
        assert_eq!(icon_str(&c, "ok").unwrap(), "\u{2714}");
        let c = nf("mode \"ascii\"\nicon { ok { all \"\u{2714}\" } }");
        assert_eq!(icon_str(&c, "ok").unwrap(), "ok");
        // all with plain ASCII → applies to all three tiers.
        let c = nf("icon { ok { all \"V\" } }");
        assert_eq!(icon_str(&c, "ok").unwrap(), "V");
        let c = nf("mode \"ascii\"\nicon { ok { all \"V\" } }");
        assert_eq!(icon_str(&c, "ok").unwrap(), "V");
    }

    /// `icon-before-content` overrides the column default: the left column puts the icon first,
    /// the right column puts it last, and the property reverses either.
    #[test]
    fn icon_before_content_overrides_the_column_default() {
        let c = Config::parse(
            "layout {\n    left { line { dir } }\n}\nsegments {\n    dir icon-before-content=#false\n}\n",
        )
        .unwrap();
        let lines = render_header_lines(&c, &info("/tmp", Some(0)), None, 80);
        let line = &lines[0];
        let text_at = line.find("tmp").expect("directory text");
        let icon_at = line.find('\u{f115}').expect("folder icon");
        assert!(
            text_at < icon_at,
            "icon-before-content=false should put the icon after the text, got {line:?}"
        );
    }

    /// `icon-padding "moderate"` widens the gap between the icon and the content (p10k
    /// `POWERLEVEL9K_ICON_PADDING=moderate`); the default keeps one space.
    #[test]
    fn icon_padding_moderate_widens_the_gap() {
        let layout = "layout {\n    left { line { dir } }\n}\n";
        let none = Config::parse(layout).unwrap();
        let moderate = Config::parse(&format!("icon-padding \"moderate\"\n{layout}")).unwrap();
        let draw = |c: &Config| render_header_lines(c, &info("/tmp", Some(0)), None, 80)[0].clone();
        let narrow = display_width(&draw(&none));
        let wide = display_width(&draw(&moderate));
        assert_eq!(
            wide,
            narrow + 1,
            "moderate should add one more space between icon and content: {narrow} -> {wide}"
        );
    }

    #[test]
    fn segment_attach_text_slots() {
        // Uses the real dir segment: default folder icon + content overriding the dir text,
        // with all attach slots.
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { dir content=\"02:49:19\" {\n\
               text-left \"L\"\n\
               text-middle \"M\"\n\
               text-right \"R\" fg=196\n\
             } }",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        let li = h.find('L').expect("text-left should render");
        let ii = h.find("\u{f115}").expect("folder icon should render");
        let mi = h.find('M').expect("text-middle should render");
        let ci = h.find("02:49:19").expect("content should render");
        let ri = h.find('R').expect("text-right should render");
        assert!(
            li < ii && ii < mi && mi < ci && ci < ri,
            "order should be left<icon<middle<content<right, got: {h:?}"
        );
        assert_eq!(
            fg_before(&h, "R").as_deref(),
            Some("196"),
            "text-right with fg=196 should override the foreground, got: {h:?}"
        );
    }

    #[test]
    fn segment_attach_middle_requires_icon_and_text() {
        // text-middle renders only when the segment has both an icon and text.
        // Icon only: dir + empty content → empty text, the folder icon remains.
        let only_icon = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { dir content=\"\" { text-middle \"M\" } }",
        )
        .unwrap();
        let h = render_header_lines(&only_icon, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains('M'),
            "text-middle should not render with an icon but no text, got: {h:?}"
        );
        // Text only: the history segment has no default icon but has text (the history number).
        let only_text = Config::parse(
            "layout { left { line { history #true } } }\n\
             segments { history { text-middle \"M\" } }",
        )
        .unwrap();
        let h = render_header_lines(&only_text, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains('M'),
            "text-middle should not render with text but no icon, got: {h:?}"
        );
    }

    #[test]
    fn display_width_counts_wide_chars() {
        assert_eq!(display_width("🎂"), 2, "emoji should be 2 columns wide");
        assert_eq!(display_width("a🎂b"), 4);
        assert_eq!(display_width("2026"), 4);
    }

    #[test]
    fn attach_emoji_keeps_row_width_aligned() {
        // The attached emoji takes 2 columns; the right-align budget must use Unicode width,
        // or the row wraps at the end.
        let src = "layout {\n  left { line { dir #true } }\n  right { line { dir #true } }\n}\n\
                   segments { dir content=\"12:34:56\" { text-right \"🎂\" } }";
        let cfg = Config::parse(src).unwrap_or_else(|e| panic!("parse: {e}\nsrc={src:?}"));
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 40).join("\r\n");
        assert_eq!(
            display_width(&h),
            40 - cfg.layout.right_indent,
            "row width should still align to cols - right-indent, got: {h:?}"
        );
    }

    #[test]
    fn row_never_exceeds_cols_at_any_width() {
        // The row width must never exceed cols: the prompt protocol assumes one terminal
        // row per line, and an overflow wrap would break the header row count (and miscount
        // the rows erased for the instant header).
        let cfg = Config::parse(
            "layout {\n  left { line { os_icon; dir; vcs } }\n  \
             right { line { status; command_execution_time; background_jobs; time } }\n}\n\
             segments { dir fg=31 shorten-strategy=\"truncate_to_unique\"\n  \
             status ok-foreground=70\n  time fg=66 }",
        )
        .unwrap();
        // Starts at 20: narrower than the left column itself (here /tmp is about 10 columns)
        // has no solution; p10k wraps too.
        for cols in 20..120 {
            let rows = render_header_lines(&cfg, &info("/tmp", None), None, cols);
            for (i, row) in rows.iter().enumerate() {
                assert!(
                    display_width(row) <= cols,
                    "cols={cols} row {i} width {} exceeds cols, content={row:?}",
                    display_width(row)
                );
            }
        }
    }

    #[test]
    fn right_column_is_dropped_when_the_row_does_not_fit() {
        // p10k: when the width is insufficient the whole right column (gap included) is not
        // drawn rather than overflowing. An overflowing row breaks the prompt protocol
        // (header rows = rows reserved by the shell).
        let cfg = Config::parse(
            "layout {\n  left { line { dir #true } }\n  right { line { time #true } }\n}\n\
             segments { dir fg=31\n  time fg=66 }",
        )
        .unwrap();
        let info_ = info("/tmp", None);
        // Wide enough → the right column is present and the row width stays within cols.
        for cols in [40, 60, 100] {
            let row = &render_header_lines(&cfg, &info_, None, cols)[0];
            assert!(
                row.contains(&time_text(cfg.segment("time"))),
                "wide row should have the right column, got {row:?}"
            );
            assert!(
                display_width(row) <= cols,
                "row width must not exceed cols({cols}), got {}",
                display_width(row)
            );
        }
        // Very narrow (the left column alone cannot fit the right one) → the right column
        // disappears and the row stops growing.
        let narrow = &render_header_lines(&cfg, &info_, None, 12)[0];
        let time = time_text(cfg.segment("time"));
        assert!(
            !narrow.contains(&time),
            "narrow row should not have the right column time, got {narrow:?}"
        );
        assert!(
            !narrow.contains('·') && !narrow.contains("\u{e0b2}"),
            "dropping the right column should leave no gap or separator, got {narrow:?}"
        );
    }

    #[test]
    fn dir_classes_match_pattern_state_and_icon() {
        // p10k DIR_CLASSES: the first matching rule decides state and icon.
        fn render(cwd: &str, extra: &str) -> String {
            let cfg = Config::parse(&format!(
                "layout {{ left {{ line {{ dir #true }} }} }}\n\
                 dir-classes {{\n  class \"~/work/**\" state=\"WORK\" icon=\"★\"\n  \
                 class \"~/**\" state=\"HOME\"\n}}\n\
                 segments {{ dir fg=31 {extra} {{\n  state \"WORK\" fg=196\n  state \"HOME\" fg=39\n}} }}"
            ))
            .unwrap();
            render_header_lines(&cfg, &info(cwd, None), None, 120).join("\n")
        }
        let home = std::env::var("HOME").unwrap();
        // WORK matches: custom icon + 196 color.
        let h = render(&format!("{home}/work/proj"), "");
        assert!(h.contains('★'), "should match the WORK icon, got {h:?}");
        assert!(
            h.contains("[38;5;196m"),
            "should match the WORK state color, got {h:?}"
        );
        // Only HOME matches.
        let h = render(&format!("{home}/other"), "");
        assert!(!h.contains('★'), "should not match WORK, got {h:?}");
        assert!(
            h.contains("[38;5;39m"),
            "should match the HOME state color, got {h:?}"
        );
        // Non-writable directory: the icon becomes a lock (p10k's _NOT_WRITABLE suffix rule).
        let h = render("/proc", "show-writable=v3");
        assert!(
            h.contains("\u{f023}"),
            "non-writable should show the lock, got {h:?}"
        );
        // No match → the segment default color.
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             dir-classes { class \"/nope/**\" state=\"X\" }\n\
             segments { dir fg=31 { state \"X\" fg=196 } }",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 120).join("\n");
        assert!(
            h.contains("[38;5;31m"),
            "no match should use default segment color, got {h:?}"
        );
        assert!(!h.contains("[38;5;196m"), "got {h:?}");
    }

    #[test]
    fn dir_show_writable_swaps_in_lock_icon() {
        // p10k DIR_SHOW_WRITABLE: the dir icon of a non-writable directory becomes LOCK_ICON.
        fn render(cwd: &str, props: &str) -> String {
            let cfg = Config::parse(&format!(
                "layout {{ left {{ line {{ dir #true }} }} }}\nsegments {{ dir {props} }}"
            ))
            .unwrap();
            render_header_lines(&cfg, &info(cwd, None), None, 120).join("\n")
        }
        // /proc exists but is not writable (nothing but root can write there).
        let locked = render("/proc", "show-writable=v3");
        assert!(
            locked.contains("\u{f023}"),
            "non-writable dir should show the lock icon, got {locked:?}"
        );
        assert!(
            !locked.contains("\u{f115}"),
            "the folder icon should not be shown as well, got {locked:?}"
        );
        // A writable directory behaves as usual.
        let normal = render("/tmp", "show-writable=v3");
        assert!(normal.contains("\u{f115}"), "got {normal:?}");
        assert!(!normal.contains("\u{f023}"), "got {normal:?}");
        // Unset means no check at all.
        let off = render("/proc", "");
        assert!(off.contains("\u{f115}"), "got {off:?}");
    }

    #[test]
    fn ruler_line_renders_above_the_header() {
        // p10k SHOW_RULER: lay a full row of RULER_CHAR above the header (default `─`).
        let cfg = Config::parse(
            "layout {\n  show-ruler #true\n  left { line { dir #true } }\n}\n\
             segments { ruler fg=240 }",
        )
        .unwrap();
        let lines = render_header_lines(&cfg, &info("/tmp", None), None, 20);
        assert_eq!(lines.len(), 2, "ruler line + header line");
        assert_eq!(
            display_width(&lines[0]),
            20,
            "the ruler should span the full row width"
        );
        assert!(
            lines[0].contains("\u{2500}") && lines[0].contains("[38;5;240m"),
            "ruler char and color are wrong, got {:?}",
            lines[0]
        );
        // Disabled means no ruler row.
        let off = Config::parse("layout { left { line { dir #true } } }").unwrap();
        assert_eq!(
            render_header_lines(&off, &info("/tmp", None), None, 20).len(),
            1
        );
        // The ascii tier uses `-`.
        let asc = Config::parse(
            "mode \"ascii\"\nlayout {\n  show-ruler #true\n  left { line { dir #true } }\n}",
        )
        .unwrap();
        let l = render_header_lines(&asc, &info("/tmp", None), None, 10);
        assert!(
            l[0].contains('-') && !l[0].contains('\u{2500}'),
            "got {:?}",
            l[0]
        );
    }

    #[test]
    fn dir_absolute_omit_first_highlight_and_hyperlink() {
        /// Strips SGR, leaving visible text only.
        fn plain(h: &str) -> String {
            let mut out = String::new();
            let mut it = h.chars().peekable();
            while let Some(c) = it.next() {
                if c == '\u{1b}' {
                    for d in it.by_ref() {
                        if d == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        fn render(cwd: &str, props: &str) -> String {
            // Strategy given explicitly: p10k's SHORTEN_STRATEGY declaration defaults to an
            // empty string, which takes its default branch (keep the last N levels); user
            // configs all use truncate_to_unique.
            let cfg = Config::parse(&format!(
                "layout {{ left {{ line {{ dir #true }} }} }}\n\
                 segments {{ dir shorten-strategy=\"truncate_to_unique\" {props} }}"
            ))
            .unwrap();
            render_header_lines(&cfg, &info(cwd, None), None, 200).join("\n")
        }
        // p10k DIR_PATH_ABSOLUTE: show the absolute path even under $HOME (no ~ abbreviation).
        let home = std::env::var("HOME").unwrap();
        let under_home = format!("{home}/probe");
        assert!(plain(&render(&under_home, "")).contains("~/probe"));
        let abs = plain(&render(&under_home, "path-absolute=#true"));
        assert!(abs.contains(&under_home), "got {abs:?}");
        assert!(
            !abs.contains('~'),
            "should not have the ~ abbreviation, got {abs:?}"
        );
        // p10k DIR_OMIT_FIRST_CHARACTER: drop the leading `/` of an absolute path; root still
        // shows `/`.
        // Multi-level paths need shorten-dir-length=2, otherwise only the last level is kept.
        let two = "shorten-dir-length=2";
        assert!(plain(&render("/tmp/x", two)).contains("/tmp/x"));
        let omitted = plain(&render(
            "/tmp/x",
            &format!("{two} omit-first-character=#true"),
        ));
        assert!(omitted.contains("tmp/x"), "got {omitted:?}");
        assert!(
            !omitted.contains("/tmp/x"),
            "the leading slash should be omitted"
        );
        assert_eq!(
            plain(&render("/", "omit-first-character=#true")).trim(),
            "\u{f115} /"
        );
        // p10k DIR_PATH_HIGHLIGHT_FOREGROUND/BOLD: recolor/bold the last component only.
        let h = render(
            "/a/b",
            "shorten-dir-length=2 path-highlight-foreground=196 path-highlight-bold=#true",
        );
        let i = h
            .rfind("\u{1b}[38;5;196m")
            .expect("the last component should use 196");
        assert!(
            h[i..].contains('b'),
            "the last component should be b, got {h:?}"
        );
        assert!(
            !h[..i].contains("196"),
            "non-last components should not use the highlight color, got {h:?}"
        );
        assert!(
            h[i..].contains("\u{1b}[1m"),
            "highlight-bold should add bold"
        );
        // p10k DIR_HYPERLINK: wrap the directory in OSC 8 and URL-escape the path.
        let link = render("/a/b", "shorten-dir-length=2 hyperlink=#true");
        assert!(
            link.contains("\u{1b}]8;;file:///a/b\u{7}"),
            "hyperlink head is wrong, got {link:?}"
        );
        assert!(link.contains("\u{1b}]8;;\u{7}"), "hyperlink tail is wrong");
        assert!(
            !render("/a/b", "").contains("]8;;"),
            "hyperlinks should be absent when disabled"
        );
    }

    #[test]
    fn dir_unique_shortening_depends_on_row_width() {
        // p10k's truncate_to_unique shortens dynamically by row width: unchanged on a wide
        // terminal, shortened to unique prefixes only when narrow (measured: one path is
        // unshortened at 130/100 cols, one level at 90, two levels at 80).
        fn dir_text(cols: usize, cwd: &str) -> String {
            let cfg = Config::parse(
                "layout { left { line { dir #true } } }\n\
                 segments { dir shorten-strategy=\"truncate_to_unique\" shorten-dir-length=1 }",
            )
            .unwrap();
            let h = render_header_lines(&cfg, &info(cwd, None), None, cols).join("\n");
            let mut out = String::new();
            let mut it = h.chars().peekable();
            while let Some(c) = it.next() {
                if c == '\u{1b}' {
                    for d in it.by_ref() {
                        if d == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        // The fixture lives under the temp directory, so how many levels a given deficit
        // reaches depends on where that is, and CI's `HOME` is the build directory. Assert the
        // direction, not a level count: a deeper deficit shortens strictly more levels.
        let root = std::env::temp_dir().join("p11k-width-fixture");
        let deep = root.join("Projects/powerlevel11k/crates/p11k-engine");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&deep).unwrap();
        let cwd = deep.to_str().unwrap().to_string();
        // First measure the row width with no shortening at all (200 cols is wide enough).
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { dir shorten-strategy=\"truncate_to_unique\" shorten-dir-length=1 }",
        )
        .unwrap();
        let full = render_header_lines(&cfg, &info(&cwd, None), None, 200).join("\n");
        let full_w = display_width(&full);
        // A level counts as shortened once its name stops appearing in the rendered row.
        let names = [
            "p11k-width-fixture",
            "Projects",
            "powerlevel11k",
            "crates",
            "p11k-engine",
        ];
        let shortened = |cols: usize| {
            let text = dir_text(cols, &cwd);
            names.iter().filter(|name| !text.contains(**name)).count()
        };
        assert_eq!(shortened(200), 0, "a wide row shortens nothing");
        let mid = shortened(full_w - 20);
        assert!(
            mid >= 1,
            "a 20-column deficit shortens at least one level, got {mid}"
        );
        let narrow = shortened(full_w - 40);
        assert!(
            narrow > mid,
            "a deeper deficit shortens more levels: {narrow} vs {mid}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn user_and_date_segments_render() {
        let cfg = Config::parse(
            "layout { left { line { user #true; date #true } } }\n\
             segments { date date-format=\"%Y\" }",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        let user = std::env::var("USER").unwrap_or_default();
        assert!(
            h.contains(&user),
            "user segment should show $USER, got: {h:?}"
        );
        let digits: Vec<char> = h.chars().filter(|c| c.is_ascii_digit()).collect();
        assert!(
            digits.len() >= 4,
            "date segment with date-format=%Y should output a four-digit year, got: {h:?}"
        );
    }

    #[test]
    fn context_hidden_for_local_non_ssh() {
        // Without SSH, context does not take the user@host form (local root shows user only).
        let cfg = Config::parse("layout { left { line { context #true } } }").unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains('@'),
            "non-SSH should not show user@host, got: {h:?}"
        );
    }

    #[test]
    fn env_indicator_segments() {
        // ranger: setting RANGER_LEVEL shows the level; clearing it hides the segment.
        let r = Config::parse("layout { left { line { ranger #true } } }").unwrap();
        unsafe {
            std::env::set_var("RANGER_LEVEL", "2");
        }
        let h = render_header_lines(&r, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains('2'), "ranger should show the level, got: {h:?}");
        unsafe {
            std::env::remove_var("RANGER_LEVEL");
        }
        let h2 = render_header_lines(&r, &info("/tmp", None), None, 80).join("\r\n");
        assert_eq!(
            h2.trim(),
            "",
            "ranger should be hidden after clearing, got: {h2:?}"
        );
        // proxy: setting http_proxy shows its host:port (clearing is not asserted, the machine
        // may ship proxy env).
        let p = Config::parse("layout { left { line { proxy #true } } }").unwrap();
        unsafe {
            std::env::set_var("http_proxy", "http://proxy.example:8080");
        }
        let hp = render_header_lines(&p, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            hp.contains("proxy.example:8080"),
            "proxy should show the http_proxy host:port, got: {hp:?}"
        );
        unsafe {
            std::env::remove_var("http_proxy");
        }
    }

    #[test]
    fn version_segments_resolve() {
        // cpu_arch always has a value from /proc or uname; a missing command → the segment
        // hides (None).
        assert!(
            !cpu_arch().is_empty(),
            "cpu_arch should have a value, got: {:?}",
            cpu_arch()
        );
        assert_eq!(run_cmd("no-such-cmd-p11k-test", &["--version"]), None);
    }

    #[test]
    fn system_resource_segments_resolve() {
        assert!(!load().is_empty(), "load should be read from /proc/loadavg");
        assert!(!ram().is_empty(), "ram should be MemAvailable");
        assert!(
            !disk_usage("/tmp").is_empty(),
            "disk_usage should have df output"
        );
    }

    /// Fixed git status for the golden renders.
    /// The status covers every vcs state that the header can draw.
    fn golden_vcs() -> crate::theme::GitStatus {
        crate::theme::GitStatus {
            branch: "main".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            staged: 1,
            unstaged: 2,
            conflicted: 0,
            untracked: 3,
            ahead: 1,
            behind: 1,
            push_ahead: 2,
            push_behind: 1,
            stashes: 2,
            action: "rebase".into(),
            tag: "v1".into(),
            remote_branch: "origin/main".into(),
            commit_summary: "fix wip".into(),
            index_size: 42,
            remote_url: "https://github.com/example/repo".into(),
        }
    }

    fn golden_info() -> crate::theme::HeaderInfo {
        crate::theme::HeaderInfo {
            exit_code: Some(9),
            cwd: "/srv/p11k/golden/repo".into(),
            exec_seconds: 12.5,
            exec_ms: None,
            jobs: 2,
            history: 5,
            pipestatus: vec![1, 0],
        }
    }

    /// A preset with the machine-dependent and clock segments removed.
    /// The os_icon segment reads the host os-release. The time and date segments read the
    /// clock. None of them can be part of a committed snapshot.
    fn golden_config(kind: crate::presets::PresetKind) -> Config {
        let mut cfg = crate::presets::build(kind);
        for row in cfg
            .layout
            .left
            .iter_mut()
            .chain(cfg.layout.right.iter_mut())
        {
            row.retain(|el| {
                !matches!(el, Element::Seg(name) if matches!(name.as_str(), "os_icon" | "time" | "date"))
            });
        }
        cfg
    }

    /// The full header and input-line output for one preset.
    /// The output covers each width in the matrix.
    fn golden_lines(kind: crate::presets::PresetKind) -> Vec<(usize, Vec<String>)> {
        let cfg = golden_config(kind);
        let info = golden_info();
        let vcs = golden_vcs();
        [40usize, 80, 120, 160]
            .into_iter()
            .map(|cols| {
                let mut lines = render_header_lines(&cfg, &info, Some(&vcs), cols);
                lines.push(crate::render::input_prefix(&cfg, None).text);
                (cols, lines)
            })
            .collect()
    }

    /// The registry is the single source for delegating segments: names are unique and every
    /// fixed icon key resolves. The special segments must not be listed, or two paths would
    /// define the same segment.
    #[test]
    fn registry_is_complete_and_unique() {
        use std::collections::HashSet;
        let names: HashSet<&str> = registry::SEGMENTS.iter().map(|s| s.name).collect();
        assert_eq!(
            names.len(),
            registry::SEGMENTS.len(),
            "duplicate segment name"
        );
        let cfg = Config::default_lean().unwrap();
        let header = info("/tmp", Some(0));
        for spec in registry::SEGMENTS {
            let ctx = Ctx {
                config: &cfg,
                segment: cfg.segment(spec.name),
                info: &header,
                vcs: None,
                dir_budget: None,
            };
            let value = spec.render.render(&ctx);
            match &value.icon {
                Icon::None => {}
                Icon::Key(key) | Icon::Content(key) => assert!(
                    icon_str(&cfg, key).is_some(),
                    "icon key {key:?} of segment {:?} does not resolve",
                    spec.name
                ),
                other => {
                    let glyph = resolve_icon_choice(&cfg, other, &header, None);
                    assert!(
                        glyph.is_some(),
                        "{:?} of segment {:?} does not resolve",
                        std::mem::discriminant(other),
                        spec.name
                    );
                }
            }
        }
        for special in [
            "dir",
            "vcs",
            "status",
            "prompt_char",
            "command_execution_time",
            "symfony2_tests",
            "os",
            "os_icon",
        ] {
            assert!(
                names.contains(special),
                "special segment {special:?} must be in the registry"
            );
        }
        assert!(
            registry::find("dir").unwrap().render.width_dependent(),
            "only dir depends on the shortening budget"
        );
        for content in ["status", "os", "os_icon"] {
            let meta = registry::find(content).unwrap();
            let ctx = Ctx {
                config: &cfg,
                segment: cfg.segment(meta.name),
                info: &header,
                vcs: None,
                dir_budget: None,
            };
            assert!(
                meta.render.render(&ctx).icon.is_content(),
                "{content:?} draws its icon with empty text"
            );
        }
    }

    /// The dir_writable icon is a fixed key now: the text decides whether the segment shows.
    /// The old conditional helper read the engine process directory instead of the shell's.
    /// A hyperlink wraps the dir text in OSC 8; the URL is not visible text, and the width must
    /// not count it (nor stop at an `m` inside the path).
    #[test]
    fn hyperlink_does_not_change_the_visible_width() {
        let cfg = Config::parse(
            "mode \"ascii\"\nsegments { dir hyperlink=#true }\nlayout { left { line { dir } } }",
        )
        .unwrap();
        let line = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("");
        assert!(line.contains("\x1b]8;;"), "no hyperlink: {line:?}");
        assert_eq!(display_width(&line), 5, "got {line:?}");
    }

    /// In the ascii tier the dir icon is empty. An empty icon is no icon: the row must not gain
    /// the icon padding.
    #[test]
    fn ascii_mode_draws_no_padding_for_empty_icons() {
        let cfg = Config::parse("mode \"ascii\"\nlayout { left { line { dir } } }").unwrap();
        let line = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("");
        assert_eq!(strip_ansi(&line), "/tmp ", "got {line:?}");
    }

    #[test]
    fn dir_writable_icon_uses_the_prompt_directory() {
        let mut cfg = Config::default_lean().unwrap();
        cfg.layout.left = vec![vec![Element::Seg("dir_writable".into())]];
        let lock = icon_str(&cfg, "lock").unwrap();
        let line = render_header_lines(&cfg, &info("/proc", Some(0)), None, 80).join("");
        assert!(
            line.contains('!'),
            "the not-writable mark must show, got {line:?}"
        );
        assert!(
            line.contains(&lock),
            "the lock icon must show for the prompt directory, got {line:?}"
        );
    }

    #[test]
    fn golden_lean() {
        insta::assert_debug_snapshot!(golden_lines(crate::presets::PresetKind::Lean));
    }

    #[test]
    fn golden_classic() {
        insta::assert_debug_snapshot!(golden_lines(crate::presets::PresetKind::Classic));
    }

    #[test]
    fn golden_rainbow() {
        insta::assert_debug_snapshot!(golden_lines(crate::presets::PresetKind::Rainbow));
    }

    #[test]
    fn golden_pure() {
        insta::assert_debug_snapshot!(golden_lines(crate::presets::PresetKind::Pure));
    }
}
