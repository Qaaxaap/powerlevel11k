use std::collections::BTreeMap;

use super::*;
use crate::i18n::{t, t_args};
use kdl::KdlDocument;

impl Config {
    pub fn parse(src: &str) -> Result<Config, String> {
        let doc = KdlDocument::parse(src).map_err(|e| {
            let detail = e.to_string();
            t_args("config-kdl-parse-error", &[("detail", &detail)])
        })?;
        Self::parse_doc(&doc)
    }
}

impl Config {
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
        let mut shell: Option<String> = None;
        let mut instant_prompt = true;
        let mut icon_padding = IconPadding::default();
        let mut seen_layout = false;

        for node in doc.nodes() {
            match node.name().value() {
                "layout" => {
                    layout = parse_layout(node)?;
                    seen_layout = true;
                }
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
                "defaults" => defaults = Style::from_style_entries(node.entries())?,
                "separators" => separators = parse_separators(node)?,
                "frame" => frame = parse_frame(node)?,
                "vcs-remote-icons" => vcs_remote_icons = parse_remote_icons(node),
                "mode" => {
                    if let Some(s) = first_value(node).and_then(str_val) {
                        mode = IconMode::from_str(&s);
                    }
                }
                "icon" => icon_overrides = parse_icon_table(node),
                "dir-classes" => dir_classes = parse_dir_classes(node),
                "shell" => shell = first_value(node).and_then(str_val),
                "instant-prompt" => {
                    instant_prompt = first_value(node).map(bool_val).unwrap_or(true);
                }
                "icon-padding" => {
                    if let Some(s) = first_value(node).and_then(str_val) {
                        icon_padding = IconPadding::from_str(&s);
                    }
                }
                _ => warn_unknown_key(node.name().value()),
            }
        }
        if !seen_layout {
            return Err(t("config-layout-required"));
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
            shell,
            instant_prompt,
            icon_padding,
        })
    }
}

use super::locale::*;
use super::parse_helpers::*;
