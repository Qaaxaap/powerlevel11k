//! Segment evaluation: one interface for every built-in segment.
//!
//! A segment sees a [`Ctx`] and returns a [`SegmentValue`]: the plain text, the icon decision
//! and an optional style override. The renderer paints the value and owns the layout; the
//! segment owns only what it contributes. Delegating segments share one implementation
//! ([`Delegated`]); segments that need the renderer state implement the trait directly.

use crate::config::{Config, Segment};
use crate::theme::{GitStatus, HeaderInfo};

use super::env::*;
use super::style::paint;

/// What a segment sees when it is evaluated.
pub(super) struct Ctx<'a> {
    pub config: &'a Config,
    pub segment: &'a Segment,
    pub info: &'a HeaderInfo,
    pub vcs: Option<&'a GitStatus>,
    pub dir_budget: Option<usize>,
}

/// The icon decision: the renderer resolves keys and draws the icon.
pub(super) enum Icon {
    None,
    /// A fixed icon key from the icon table.
    Key(&'static str),
    /// The icon is the content: it draws even with empty text.
    Content(&'static str),
    /// The directory icon: lock when not writable, then `dir-classes`, then the default.
    Dir,
    /// The vcs icon: a `vcs-remote-icons` match or the git icon.
    VcsRemote,
    /// The OS badge (the icon is the content).
    Os,
}

/// How the icon is coloured.
pub(super) enum IconStyle {
    /// The segment style, honouring `visual-identifier-color`.
    Segment,
    /// The vcs segment: fall back to `clean-foreground`.
    VcsClean,
    /// The status segment: fall back to the state's foreground.
    Status,
}

impl Icon {
    /// Whether the icon is the content, so empty text still draws it.
    pub(super) fn is_content(&self) -> bool {
        matches!(self, Icon::Content(_) | Icon::Os)
    }
}

/// What one segment contributes before the renderer paints it.
pub(super) struct SegmentValue {
    /// Painted text: the segment returns its own ANSI, the renderer adds icon, attach slots and
    /// spacing.
    pub text: String,
    pub icon: Icon,
    pub icon_style: IconStyle,
}

impl SegmentValue {
    pub(super) fn text(text: String) -> SegmentValue {
        SegmentValue {
            text,
            icon: Icon::None,
            icon_style: IconStyle::Segment,
        }
    }

    pub(super) fn with_icon(mut self, icon: Icon) -> SegmentValue {
        self.icon = icon;
        self
    }
}

/// One built-in segment.
pub(super) trait SegmentRender: Sync {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue;
    /// Whether the text depends on the shortening budget (`dir` only).
    fn width_dependent(&self) -> bool {
        false
    }
}

/// Where a delegating segment's text comes from.
pub(super) enum TextSource {
    Plain(fn() -> String),
    Cwd(fn(&str) -> String),
    Segment(fn(&Segment) -> String),
    Arg(fn(&str) -> String, &'static str),
    Env(&'static str),
    /// The number of background jobs.
    Jobs,
    History,
    Empty,
}

impl TextSource {
    fn eval(&self, ctx: &Ctx<'_>) -> String {
        match self {
            TextSource::Plain(f) => f(),
            TextSource::Cwd(f) => f(&ctx.info.cwd),
            TextSource::Segment(f) => f(ctx.segment),
            TextSource::Arg(f, a) => f(a),
            TextSource::Env(k) => env_var(k).unwrap_or_default(),
            TextSource::Jobs => {
                if ctx.info.jobs > 0 {
                    ctx.info.jobs.to_string()
                } else {
                    String::new()
                }
            }
            TextSource::History => ctx.info.history.to_string(),
            TextSource::Empty => String::new(),
        }
    }
}

/// How a delegating segment's icon is chosen.
pub(super) enum IconSource {
    None,
    Key(&'static str),
    /// A fixed key when a condition holds; the icon is the segment content.
    ContentCond(fn() -> Option<&'static str>),
}

impl IconSource {
    fn eval(&self) -> Icon {
        match self {
            IconSource::None => Icon::None,
            IconSource::Key(k) => Icon::Key(k),
            IconSource::ContentCond(f) => match f() {
                Some(k) => Icon::Content(k),
                None => Icon::None,
            },
        }
    }
}

/// Every segment that only delegates to a text function shares this implementation.
pub(super) struct Delegated {
    pub text: TextSource,
    pub icon: IconSource,
}

impl SegmentRender for Delegated {
    fn render(&self, ctx: &Ctx<'_>) -> SegmentValue {
        let style = ctx.segment.effective_style(None, &ctx.config.defaults);
        SegmentValue {
            text: paint(&self.text.eval(ctx), &style),
            icon: self.icon.eval(),
            icon_style: IconStyle::Segment,
        }
    }
}
