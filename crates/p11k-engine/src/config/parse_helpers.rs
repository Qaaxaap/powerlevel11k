use std::collections::BTreeMap;

use super::*;
use crate::i18n::{t, t_args};
use kdl::{KdlNode, KdlValue};

/// Parse the top-level `icon` node: each child is the override for one icon name, and the
/// override fields are its children (e.g. `ok { all "✔" ascii "V" }`, one each for all/nf/compat/ascii).
pub(super) fn parse_icon_table(node: &KdlNode) -> BTreeMap<String, IconOverride> {
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

pub(super) fn default_remote_icons() -> Vec<(String, String)> {
    vec![
        ("github".into(), "\u{f113}".into()),            // 
        ("gitlab".into(), "\u{f296}".into()),            // 
        ("bitbucket".into(), "\u{f171}".into()),         // 
        ("aur.archlinux.org".into(), "\u{f303}".into()), // 
        ("archlinux.org".into(), "\u{f303}".into()),     // 
    ]
}

/// Merge multiple nodes with the same segment name: non-Default styles override, text/icons take the later value, states/props accumulate.
pub(super) fn merge_segment(a: &mut Segment, b: Segment) {
    if b.style.fg.is_some() {
        a.style.fg = b.style.fg;
    }
    if b.style.bg.is_some() {
        a.style.bg = b.style.bg;
    }
    if b.style.bold.is_some() {
        a.style.bold = b.style.bold;
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
        match a.states.get_mut(&k) {
            Some(existing) => {
                // A later state declaration overrides only the fields it sets; the rest of the
                // earlier declaration survives.
                existing.style = merge_style(&v.style, &existing.style);
                if v.char.is_some() {
                    existing.char = v.char;
                }
            }
            None => {
                a.states.insert(k, v);
            }
        }
    }
    for (k, v) in b.props {
        a.props.insert(k, v);
    }
}

/// Parse `dir-classes { class "pattern" state="WORK" icon="…" }`.
/// The first argument is the pattern, `state` / `icon` are node properties; matched in declaration order, first match wins.
pub(super) fn parse_dir_classes(node: &KdlNode) -> Vec<DirClass> {
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
pub(super) fn parse_remote_icons(node: &KdlNode) -> Vec<(String, String)> {
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
pub(super) fn parse_frame(node: &KdlNode) -> Result<Frame, String> {
    let mut f = Frame {
        style: Style::from_style_entries(node.entries())?,
        ..Frame::default()
    };
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            let Some(v) = first_value(child) else {
                continue;
            };
            let Some(text) = str_val(v) else { continue };
            let st = Style::from_style_entries(child.entries())?;
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
    Ok(f)
}

/// Parse `layout`: the two layout line groups `left`/`right`, the `input` row the user types on,
/// plus `prompt-add-newline`, `transient-prompt`, `show-ruler`, `right-indent`.
pub(super) fn parse_layout(node: &KdlNode) -> Result<Layout, String> {
    let mut layout = Layout::default();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "left" => layout.left = parse_lines(child),
                "right" => layout.right = parse_lines(child),
                "input" => layout.input = Some(parse_input(child)),
                // `prompt-add-newline #true` = 1 line; a number is taken as is (0 = off).
                "prompt-add-newline" => {
                    layout.prompt_add_newline = match first_value(child) {
                        Some(KdlValue::Integer(n)) => (*n).clamp(0, 9) as usize,
                        v => usize::from(v.map(bool_val).unwrap_or(false)),
                    };
                }
                "transient-prompt" => {
                    // `#true` keeps the meaning it had before the modes existed: fold after
                    // every command.
                    layout.transient_prompt = match first_value(child) {
                        Some(KdlValue::String(s)) => TransientPrompt::from_str(s),
                        v => {
                            if v.map(bool_val).unwrap_or(false) {
                                TransientPrompt::Always
                            } else {
                                TransientPrompt::Off
                            }
                        }
                    };
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

/// Parse `input`: the row the user types on. `left` and `right` each hold the elements of that
/// one row, so they are parsed like a `line` body without the `line` wrapper.
pub(super) fn parse_input(node: &KdlNode) -> InputLine {
    let mut input = InputLine::default();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            match child.name().value() {
                "left" => input.left = parse_line(child),
                "right" => input.right = parse_line(child),
                _ => warn_unknown_key(child.name().value()),
            }
        }
    }
    input
}

/// Parse one side: each child node is one line.
pub(super) fn parse_lines(node: &KdlNode) -> Vec<Vec<Element>> {
    let mut rows = Vec::new();
    if let Some(ch) = node.children() {
        for row in ch.nodes() {
            rows.push(parse_line(row));
        }
    }
    rows
}

/// Parse one line: each child node is a segment (boolean enable), or `text "…"` static text.
pub(super) fn parse_line(node: &KdlNode) -> Vec<Element> {
    let mut out = Vec::new();
    if let Some(ch) = node.children() {
        for child in ch.nodes() {
            // A string value → static text (e.g. `text "some text"`); keep the node name so
            // `to_kdl` writes the same shape back. Otherwise treat it as a segment.
            if let Some(KdlValue::String(s)) = first_value(child) {
                if child.name().value() == "text" {
                    out.push(Element::Text(s.clone()));
                } else {
                    out.push(Element::NamedText(
                        child.name().value().to_string(),
                        s.clone(),
                    ));
                }
                continue;
            }
            let name = child.name().value();
            // An absent value means enabled; an explicit #false disables it. The node is kept
            // so a configure round trip does not delete the commented-out segment.
            let enabled = first_value(child).map(bool_val).unwrap_or(true);
            if !enabled {
                out.push(Element::Disabled(name.to_string()));
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
pub(super) fn parse_segment(node: &KdlNode) -> Result<Segment, String> {
    let mut seg = Segment {
        style: Style::from_mixed_entries(node.entries())?,
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
                        return Err(t("config-state-needs-name"));
                    };
                    let st = Style::from_mixed_entries(child.entries())?;
                    let ch = named_str(child, "char");
                    let spec = StateSpec {
                        style: st,
                        char: ch,
                    };
                    match seg.states.get_mut(&nm) {
                        Some(existing) => {
                            // Repeated `state` nodes in one block merge by field, like repeated
                            // segment declarations do.
                            existing.style = merge_style(&spec.style, &existing.style);
                            if spec.char.is_some() {
                                existing.char = spec.char;
                            }
                        }
                        None => {
                            seg.states.insert(nm, spec);
                        }
                    }
                }
                // Attached text slots: left/middle/right. The positional string is the text, `fg` is optional.
                "text-left" | "text-middle" | "text-right" => {
                    let Some(text) = first_state_name(child) else {
                        return Err(t_args(
                            "config-element-needs-text",
                            &[("element", child.name().value())],
                        ));
                    };
                    let attach = AttachText {
                        text,
                        fg: named_color(child, "fg")?,
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
pub(super) fn named_str(node: &KdlNode, name: &str) -> Option<String> {
    node.entries()
        .iter()
        .find(|e| e.name().map(|n| n.value() == name).unwrap_or(false))
        .and_then(|e| str_val(e.value()))
}

/// A node's named colour property (e.g. attached text's `fg=196`). A missing property is
/// `None`; a present one must parse.
pub(super) fn named_color(node: &KdlNode, name: &str) -> Result<Option<Color>, String> {
    for e in node.entries() {
        if e.name().map(|n| n.value() == name).unwrap_or(false) {
            return Ok(Some(Color::from_value(e.value(), name)?));
        }
    }
    Ok(None)
}

/// A node's first positional argument value (the entry with no name).
pub(super) fn first_value(node: &KdlNode) -> Option<&KdlValue> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .map(|e| e.value())
}

/// A state node's name = its first positional argument.
pub(super) fn first_state_name(node: &KdlNode) -> Option<String> {
    match first_value(node) {
        Some(KdlValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Warn when the config contains a key the engine does not know.
pub(super) fn warn_unknown_key(name: &str) {
    eprintln!("p11k: {}", t_args("config-unknown-key", &[("key", name)]));
}

/// Parse the `separators` node: character child nodes (`segment`/`sub`/`end`/`right-start`/`right-segment`/`right-sub`/`left-tail`/`right-tail`/`gap`), values read as whole strings.
pub(super) fn parse_separators(node: &KdlNode) -> Result<Separators, String> {
    let mut s = Separators::default();
    // `gap-foreground` / `sub-foreground` / `right-sub-foreground` are
    // properties of the separators node (not child nodes).
    if let Some(c) = named_color(node, "gap-foreground")? {
        s.gap_foreground = Some(c);
    }
    if let Some(c) = named_color(node, "sub-foreground")? {
        s.sub_foreground = Some(c);
    }
    if let Some(c) = named_color(node, "right-sub-foreground")? {
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
    Ok(s)
}

pub(super) static EMPTY_SEG: Segment = Segment {
    style: Style {
        fg: None,
        bg: None,
        bold: None,
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

/// Merge: the upper layer's set fields override the lower layer (the tail end of the three-way
/// fallback). `Some(Color::Default)` and `Some(false)` are explicit values and still override.
pub(super) fn merge_style(over: &Style, base: &Style) -> Style {
    Style {
        fg: over.fg.clone().or_else(|| base.fg.clone()),
        bg: over.bg.clone().or_else(|| base.bg.clone()),
        bold: over.bold.or(base.bold),
    }
}

pub(super) fn bool_val(v: &KdlValue) -> bool {
    matches!(v, KdlValue::Bool(true))
}
pub(super) fn str_val(v: &KdlValue) -> Option<String> {
    match v {
        KdlValue::String(s) => Some(s.clone()),
        KdlValue::Integer(n) => Some(n.to_string()),
        KdlValue::Float(f) => Some(f.to_string()),
        KdlValue::Bool(b) => Some(b.to_string()),
        KdlValue::Null => None,
    }
}
pub(super) fn prop_val(v: &KdlValue) -> Prop {
    match v {
        KdlValue::String(s) => Prop::Str(s.clone()),
        KdlValue::Integer(n) => Prop::Int((*n) as i64),
        KdlValue::Float(f) => Prop::Float(*f),
        KdlValue::Bool(b) => Prop::Bool(*b),
        KdlValue::Null => Prop::Str(String::new()),
    }
}
