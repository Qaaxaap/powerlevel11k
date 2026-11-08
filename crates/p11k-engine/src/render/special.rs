//! Segments that need the renderer state instead of a text function.
//!
//! Each one implements [`SegmentRender`] directly: it reads the prompt state (exit code, cwd,
//! timing, vcs) and returns the same painted text the delegating segments return.

use crate::config::Style;

use super::segment::{Ctx, Icon, IconStyle, SegmentRender, SegmentValue};
use super::segments::*;
use super::style::{paint, prompt_state};

/// The directory segment with its shortening budget.
pub(super) struct DirSegment;

impl SegmentRender for DirSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        SegmentValue::text(dir_seg_text(
            ctx.config,
            ctx.info,
            ctx.segment,
            &style,
            ctx.dir_budget,
        ))
        .with_icon(Icon::Dir)
    }

    fn width_dependent(&self) -> bool {
        true
    }
}

/// The vcs segment: the caller may replace the text with `content`.
pub(super) struct VcsSegment;

impl SegmentRender for VcsSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        let text = match ctx.segment.content.as_deref() {
            Some(content) => paint(&value_of(Some(content), String::new()), &style),
            None => {
                let branch = super::icons::icon_str(ctx.config, "branch").unwrap_or_default();
                let commit = super::icons::icon_str(ctx.config, "commit").unwrap_or_default();
                vcs_text(ctx.vcs, &branch, &commit, ctx.segment, &style)
            }
        };
        SegmentValue {
            text,
            icon: Icon::VcsRemote,
            icon_style: IconStyle::VcsClean,
        }
    }
}

/// The status segment: the icon key depends on the state.
pub(super) struct StatusSegment;

impl SegmentRender for StatusSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        // The status identifier is its content: a quiet successful status draws only the icon.
        let icon = match status_icon_key(ctx.info, ctx.segment) {
            Some(key) => Icon::Content(key),
            None => Icon::None,
        };
        SegmentValue {
            text: status_text(ctx.info, ctx.segment, &style),
            icon,
            icon_style: IconStyle::Status,
        }
    }
}

/// `prompt_char`: the char and colour depend on the prompt state.
pub(super) struct PromptCharSegment;

impl SegmentRender for PromptCharSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let state = prompt_state(ctx.config, Some(ctx.info));
        let style = ctx.segment.effective_style(state, &ctx.config.defaults);
        SegmentValue::text(paint(ctx.segment.char_for(state, "❯"), &style))
    }
}

/// `command_execution_time`: formatted from the shell-measured duration.
pub(super) struct ExecTimeSegment;

impl SegmentRender for ExecTimeSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        let threshold = match ctx.segment.prop("threshold-seconds") {
            Some(crate::config::Prop::Int(n)) => (*n as f64).max(0.0),
            _ => 3.0,
        };
        if ctx.info.exec_seconds < threshold {
            return SegmentValue::text(paint("", &style));
        }
        // p10k's PRECISION defaults to 2 decimals and FORMAT to multi-level (with spaces).
        let precision = match ctx.segment.prop("precision") {
            Some(crate::config::Prop::Int(n)) => (*n).clamp(0, 6) as usize,
            _ => 2,
        };
        let hms = matches!(
            ctx.segment.prop("format"),
            Some(crate::config::Prop::Str(s)) if s == "H:M:S"
        );
        SegmentValue::text(paint(
            &format_duration(ctx.info.exec_seconds, precision, hms),
            &style,
        ))
        .with_icon(Icon::Key("execution-time"))
    }
}

/// `symfony2_tests`: the test ratio selects the GOOD/AVG/BAD state.
pub(super) struct Symfony2TestsSegment;

impl SegmentRender for Symfony2TestsSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        let text = symfony2_tests_text(&ctx.info.cwd);
        if text.is_empty() {
            return SegmentValue::text(paint("", &style));
        }
        let pct: f64 = text
            .trim_start_matches("SF2: ")
            .trim_end_matches('%')
            .parse()
            .unwrap_or(0.0);
        let (state, default_fg) = if pct >= 75.0 {
            ("GOOD", 6u8)
        } else if pct >= 50.0 {
            ("AVG", 3)
        } else {
            ("BAD", 1)
        };
        let st = if ctx.segment.states.contains_key(state) {
            ctx.segment
                .effective_style(Some(state), &ctx.config.defaults)
        } else {
            Style {
                fg: crate::config::Color::Xterm(default_fg),
                ..style
            }
        };
        SegmentValue::text(paint(&text, &st)).with_icon(Icon::Key("test"))
    }
}

/// `os` and `os_icon`: no text, the icon is the content.
pub(super) struct OsSegment;

impl SegmentRender for OsSegment {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        SegmentValue {
            text: paint("", &style),
            icon: Icon::Os,
            icon_style: IconStyle::Segment,
        }
    }
}
