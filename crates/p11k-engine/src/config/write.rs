use super::parse_helpers::*;
use super::*;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

impl Config {
    pub fn to_kdl(&self) -> String {
        let mut doc = self.to_document();
        doc.autoformat();
        quote_string_values(&mut doc);
        doc.to_string()
    }

    /// Build the equivalent KDL document. Fields equal to the `parse` default are omitted.
    pub fn to_document(&self) -> KdlDocument {
        let mut doc = KdlDocument::new();
        // mode must be emitted explicitly: the parse default follows the locale, so omitting it breaks round-trip.
        doc.nodes_mut().push(leaf("mode", self.mode.name()));
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
        if let Some(shell) = &self.shell {
            let mut n = KdlNode::new("shell");
            n.entries_mut().push(KdlEntry::new(shell.clone()));
            doc.nodes_mut().push(n);
        }
        if !self.instant_prompt {
            doc.nodes_mut().push(leaf("instant-prompt", false));
        }
        if self.icon_padding == IconPadding::Moderate {
            doc.nodes_mut().push(leaf("icon-padding", "moderate"));
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
        if let Some(input) = &self.layout.input {
            // The input row is one row per side, so it has no `line` wrapper.
            let mut node = KdlNode::new("input");
            let ich = node.ensure_children();
            ich.nodes_mut().push(side_node("left", &input.left));
            ich.nodes_mut().push(side_node("right", &input.right));
            ch.nodes_mut().push(node);
        }
        if self.layout.prompt_add_newline == 1 {
            ch.nodes_mut().push(leaf("prompt-add-newline", true));
        } else if self.layout.prompt_add_newline > 1 {
            ch.nodes_mut().push(leaf(
                "prompt-add-newline",
                self.layout.prompt_add_newline as i128,
            ));
        }
        match self.layout.transient_prompt {
            TransientPrompt::Off => {}
            // The boolean form is what earlier themes wrote, so an `always` theme round-trips to
            // the same text it came in with.
            TransientPrompt::Always => ch.nodes_mut().push(leaf("transient-prompt", true)),
            TransientPrompt::SameDir => ch.nodes_mut().push(leaf("transient-prompt", "same-dir")),
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
            let mut node = KdlNode::new(name);
            node.entries_mut().push(KdlEntry::new(piece.text.clone()));
            if let Some(st) = &piece.style {
                node.entries_mut().extend(style_entries(st));
            }
            ch.nodes_mut().push(node);
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
}

// ---- Serialization helpers (Config → KDL) ----

/// Give every string value a quoted representation. kdl-rs leaves strings that can be bare
/// identifiers unquoted
/// (e.g. `mode nerdfont-complete`, `segment <U+E0B0>`, which looks like an empty value),
/// while p11k's presets/docs are always quoted, so this unifies them. `autoformat_keep` stops
/// autoformat from overwriting that representation (it keeps only value_repr and leading and
/// reformats the rest).
pub(super) fn quote_string_values(doc: &mut KdlDocument) {
    for node in doc.nodes_mut() {
        quote_node_strings(node);
    }
}

pub(super) fn quote_node_strings(node: &mut KdlNode) {
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
pub(super) fn quote_kdl_string(s: &str) -> String {
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
pub(super) fn leaf(name: &str, value: impl Into<KdlValue>) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(value));
    n
}

pub(super) fn color_value(c: &Color) -> KdlValue {
    match c {
        Color::Default => KdlValue::from("default"),
        Color::Xterm(n) => KdlValue::from(*n as i128),
        Color::Rgb(r, g, b) => KdlValue::from(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Named(n) => KdlValue::from(n.clone()),
    }
}

/// Style → KDL properties (fg/bg/bold, only non-default entries emitted).
pub(super) fn style_entries(style: &Style) -> Vec<KdlEntry> {
    let mut v = Vec::new();
    if let Some(fg) = &style.fg {
        v.push(KdlEntry::new_prop("fg", color_value(fg)));
    }
    if let Some(bg) = &style.bg {
        v.push(KdlEntry::new_prop("bg", color_value(bg)));
    }
    if let Some(bold) = style.bold {
        v.push(KdlEntry::new_prop("bold", bold));
    }
    v
}

pub(super) fn prop_value(p: &Prop) -> KdlValue {
    match p {
        Prop::Str(s) => KdlValue::from(s.clone()),
        Prop::Int(n) => KdlValue::from(*n as i128),
        Prop::Float(f) => KdlValue::from(*f),
        Prop::Bool(b) => KdlValue::from(*b),
    }
}

/// One line of layout elements → a `line { … }` node.
pub(super) fn line_node(row: &[Element]) -> KdlNode {
    let mut n = KdlNode::new("line");
    n.ensure_children().nodes_mut().extend(element_nodes(row));
    n
}

/// One row of layout elements under a named node: the input row's `left` / `right`, which carry
/// their elements directly instead of a nested `line`.
pub(super) fn side_node(name: &str, row: &[Element]) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.ensure_children().nodes_mut().extend(element_nodes(row));
    n
}

/// The element nodes of one row.
fn element_nodes(row: &[Element]) -> Vec<KdlNode> {
    let mut out = Vec::with_capacity(row.len());
    for el in row {
        match el {
            Element::Seg(name) => {
                out.push(KdlNode::new(name.as_str()));
            }
            Element::Joined(name) => {
                out.push(KdlNode::new(format!("{name}_joined")));
            }
            Element::Text(t) => {
                out.push(leaf("text", t.clone()));
            }
            Element::NamedText(name, t) => {
                let mut n = KdlNode::new(name.as_str());
                n.entries_mut().push(KdlEntry::new(t.clone()));
                out.push(n);
            }
            Element::Disabled(name) => {
                let mut n = KdlNode::new(name.as_str());
                n.entries_mut().push(KdlEntry::new(false));
                out.push(n);
            }
        }
    }
    out
}

/// One segment → a `<name> … { state … text-* … }` node.
pub(super) fn segment_node(name: &str, seg: &Segment) -> KdlNode {
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
pub(super) fn attach_node(name: &str, a: &AttachText) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.entries_mut().push(KdlEntry::new(a.text.clone()));
    if let Some(fg) = &a.fg {
        n.entries_mut()
            .push(KdlEntry::new_prop("fg", color_value(fg)));
    }
    n
}
