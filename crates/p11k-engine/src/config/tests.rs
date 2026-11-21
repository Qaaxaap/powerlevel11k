use super::locale::*;
use super::parse_helpers::*;
use super::*;

/// A malformed colour fails the load with the property name and the value. The old
/// parser accepted any string and painted it white, and clamped an out-of-range index.
#[test]
fn invalid_colours_fail_the_load() {
    let layout = "layout { left { line { dir } } }";
    // The message text is localised, so the assertions check the property name and the
    // value, which stay the same in every language.
    for (bad, key, value) in [
        ("defaults fg=999", "fg", "999"),
        ("defaults bg=-1", "bg", "-1"),
        ("defaults fg=\"#zzzzzz\"", "fg", "#zzzzzz"),
        ("defaults fg=\"#12345\"", "fg", "#12345"),
        ("defaults fg=\"nope\"", "fg", "nope"),
        ("defaults fg=#true", "fg", ""),
    ] {
        let src = format!("{layout}\n{bad}");
        let err = Config::parse(&src).expect_err(&format!("{bad} must fail"));
        assert!(
            err.contains(key) && err.contains(value),
            "{bad} should report {key:?} and {value:?}, got {err:?}"
        );
    }
    // Attached text and the separator colours go through the same parser.
    let err = Config::parse(
        "layout { left { line { prompt_char } } }\nsegments { prompt_char { state ERROR fg=999 } }",
    )
    .expect_err("a state colour must parse");
    assert!(
        err.contains("fg"),
        "the state error should name fg, got {err:?}"
    );
    let err =
        Config::parse("separators gap-foreground=999").expect_err("a separator colour must parse");
    assert!(
        err.contains("gap-foreground"),
        "the separator error should name the key, got {err:?}"
    );
}

/// The accepted colour forms keep their meaning, including uppercase hex and names.
#[test]
fn valid_colours_are_read() {
    let d = Config::parse("layout {}\ndefaults fg=\"#ABCDEF\" bg=\"red\"").unwrap();
    assert_eq!(d.defaults.fg, Some(Color::Rgb(0xab, 0xcd, 0xef)));
    assert_eq!(d.defaults.bg, Some(Color::Named("red".into())));
    let d = Config::parse("layout {}\ndefaults fg=196 bg=default").unwrap();
    assert_eq!(d.defaults.fg, Some(Color::Xterm(196)));
    assert_eq!(d.defaults.bg, Some(Color::Default));
}

#[test]
fn shell_node_is_read_and_round_trips() {
    let c = Config::parse("layout { left { line { dir } } }\nshell \"bash\"").unwrap();
    assert_eq!(c.shell.as_deref(), Some("bash"));
    let again = Config::parse(&c.to_kdl()).unwrap();
    assert_eq!(again.shell.as_deref(), Some("bash"));
}

/// The instant prompt is on by default; `#false` turns it off and round-trips.
#[test]
fn instant_prompt_is_on_by_default_and_can_be_turned_off() {
    assert!(Config::default().instant_prompt);
    let off = Config::parse("layout { left { line { dir } } }\ninstant-prompt #false").unwrap();
    assert!(!off.instant_prompt);
    let text = off.to_kdl();
    assert!(
        text.contains("instant-prompt #false"),
        "the disabled option must serialize, got:\n{text}"
    );
    assert!(!Config::parse(&text).unwrap().instant_prompt);
}

/// The layout flags and the indent are read, clamped, serialized and read back.
#[test]
fn layout_flags_and_indent_round_trip() {
    let cfg = Config::parse("layout { left { line { dir } } }").unwrap();
    assert!(!cfg.layout.show_ruler);
    assert_eq!(cfg.layout.right_indent, 1);

    let cfg = Config::parse(
        "layout {\n    left { line { dir } }\n    show-ruler #true\n    right-indent 0\n}\n",
    )
    .unwrap();
    assert!(cfg.layout.show_ruler);
    assert_eq!(cfg.layout.right_indent, 0);
    let text = cfg.to_kdl();
    assert!(text.contains("show-ruler #true"), "{text}");
    assert!(text.contains("right-indent 0"), "{text}");
    let again = Config::parse(&text).unwrap();
    assert!(again.layout.show_ruler);
    assert_eq!(again.layout.right_indent, 0);

    // `#false` is accepted and turns the ruler off.
    let cfg =
        Config::parse("layout {\n    left { line { dir } }\n    show-ruler #false\n}\n").unwrap();
    assert!(!cfg.layout.show_ruler);

    // Out-of-range values are clamped instead of rejected.
    let cfg =
        Config::parse("layout {\n    left { line { dir } }\n    right-indent 99\n}\n").unwrap();
    assert_eq!(cfg.layout.right_indent, 9);
}

/// The `input` node is the row the user types on: one row per side, parsed like a `line` body,
/// serialized without the `line` wrapper, and outside the header lines.
#[test]
fn input_row_parses_and_round_trips() {
    let cfg = Config::parse(
        "layout {\n    input {\n        left { dir; vcs; text \"|\" }\n        right { status; time #false }\n    }\n}\n",
    )
    .unwrap();
    let input = cfg.layout.input.as_ref().expect("input row");
    assert_eq!(
        input.left,
        vec![
            Element::Seg("dir".into()),
            Element::Seg("vcs".into()),
            Element::Text("|".into()),
        ]
    );
    assert_eq!(
        input.right,
        vec![
            Element::Seg("status".into()),
            Element::Disabled("time".into()),
        ]
    );
    // The row is not a header line: a layout with only an `input` node has no header rows.
    assert!(cfg.layout.left.is_empty() && cfg.layout.right.is_empty());

    let text = cfg.to_kdl();
    assert!(
        !text.contains("line {"),
        "the input row has no line wrapper:\n{text}"
    );
    let again = Config::parse(&text).unwrap();
    assert_eq!(
        again.layout.input, cfg.layout.input,
        "round trip failed:\n{text}"
    );
}

/// No `input` node keeps the implicit prefix-only row instead of an empty declared one.
#[test]
fn a_config_without_an_input_node_has_no_input_row() {
    assert_eq!(Config::default().layout.input, None);
    let cfg = Config::parse("layout { left { line { dir } } }").unwrap();
    assert_eq!(cfg.layout.input, None);
    assert!(!cfg.to_kdl().contains("input"));
}

/// An explicitly empty row still round-trips: `input {}` means the row exists with no segments.
#[test]
fn an_empty_input_row_round_trips() {
    let cfg = Config::parse("layout { input {} }").unwrap();
    assert_eq!(cfg.layout.input, Some(InputLine::default()));
    let again = Config::parse(&cfg.to_kdl()).unwrap();
    assert_eq!(again.layout.input, Some(InputLine::default()));
}

/// An explicit `fg=default` or `bold=#false` is a real value: a later layer can clear a
/// colour or a bold flag with it, so it must survive the serializer too.
#[test]
fn explicit_default_and_false_survive_a_round_trip() {
    let c = Config::parse("layout {}\nsegments { dir fg=default bg=default bold=#false }").unwrap();
    let d = c.segment("dir");
    assert_eq!(d.style.fg, Some(Color::Default));
    assert_eq!(d.style.bg, Some(Color::Default));
    assert_eq!(d.style.bold, Some(false));
    let again = Config::parse(&c.to_kdl()).unwrap();
    assert_eq!(again.segment("dir").style, d.style);
}

/// Two declarations of the same state merge by field: the later one only overrides the
/// values it sets.
#[test]
fn later_state_declaration_merges_by_field() {
    let mut a = Segment::default();
    a.states.insert(
        "SHORTENED".into(),
        StateSpec {
            style: Style {
                fg: Some(Color::Xterm(103)),
                bold: Some(true),
                ..Style::default()
            },
            char: None,
        },
    );
    let mut b = Segment::default();
    b.states.insert(
        "SHORTENED".into(),
        StateSpec {
            style: Style {
                fg: Some(Color::Xterm(200)),
                ..Style::default()
            },
            char: Some("x".into()),
        },
    );
    merge_segment(&mut a, b);
    let st = &a.states["SHORTENED"];
    assert_eq!(st.style.fg, Some(Color::Xterm(200)));
    assert_eq!(st.style.bold, Some(true), "the earlier bold survives");
    assert_eq!(st.char.as_deref(), Some("x"));
}

/// A segment override can clear a value: `fg=default` and `bold=#false` are explicit, not
/// "unset".
#[test]
fn explicit_defaults_override_segment_style() {
    let mut a = Segment::default();
    a.style.fg = Some(Color::Xterm(1));
    a.style.bold = Some(true);
    let mut b = Segment::default();
    b.style.fg = Some(Color::Default);
    b.style.bold = Some(false);
    merge_segment(&mut a, b);
    assert_eq!(a.style.fg, Some(Color::Default));
    assert_eq!(a.style.bold, Some(false));
}

/// A named text node and a commented-out segment keep their shape through `to_kdl`.
#[test]
fn named_text_and_disabled_elements_round_trip() {
    let c = Config::parse("layout { left { line { dir \"x\"; time #false } } }").unwrap();
    assert_eq!(
        c.layout.left[0][0],
        Element::NamedText("dir".into(), "x".into())
    );
    assert_eq!(c.layout.left[0][1], Element::Disabled("time".into()));
    let again = Config::parse(&c.to_kdl()).unwrap();
    assert_eq!(again.layout.left[0], c.layout.left[0]);
}

/// Directory classification rules keep their pattern, state and icon through a round trip.
#[test]
fn dir_classes_parse_and_round_trip() {
    let src = "layout {}\ndir-classes {\n    class \"src/**\" state=\"WORK\" icon=\"x\"\n    class \"docs/**\";\n}\n";
    let cfg = Config::parse(src).unwrap();
    assert_eq!(cfg.dir_classes.len(), 2);
    assert_eq!(cfg.dir_classes[0].pattern, "src/**");
    assert_eq!(cfg.dir_classes[0].state, "WORK");
    assert_eq!(cfg.dir_classes[0].icon, "x");
    assert_eq!(cfg.dir_classes[1].pattern, "docs/**");
    assert!(cfg.dir_classes[1].state.is_empty());
    assert!(cfg.dir_classes[1].icon.is_empty());

    let again = Config::parse(&cfg.to_kdl()).unwrap();
    assert_eq!(cfg.dir_classes, again.dir_classes);

    // A rule without the pattern argument is skipped instead of creating an empty rule.
    let cfg = Config::parse("layout {}\ndir-classes { class state=\"WORK\" }").unwrap();
    assert!(cfg.dir_classes.is_empty());
}

#[test]
fn layout_is_required() {
    assert!(
        Config::parse("mode \"ascii\"\nsegments { dir fg=31 }\n").is_err(),
        "a config without a layout block must be rejected"
    );
}

/// Invalid input is either reported or ignored with the documented fallback; it never panics.
#[test]
fn invalid_config_input_is_reported_or_ignored() {
    assert!(
        Config::parse("layout {").is_err(),
        "an unterminated document is an error"
    );

    // Unknown top-level nodes warn and do not fail the parse.
    let cfg = Config::parse("unknown-node 1\nlayout { left { line { dir } } }\n").unwrap();
    assert!(!cfg.layout.left.is_empty());

    // Unknown enum values fall back to the defaults.
    let cfg =
        Config::parse("layout {}\nmode \"not-a-mode\"\nicon-padding \"not-a-padding\"\n").unwrap();
    assert_eq!(cfg.mode, IconMode::NerdfontComplete);
    assert_eq!(cfg.icon_padding, IconPadding::None);
}

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
    let c = Config::parse("mode \"compatible\"\nlayout { left { line { dir #true } } }").unwrap();
    assert_eq!(c.mode, IconMode::Compatible);
    let c = Config::parse("mode \"nerdfont-fontconfig\"\nlayout { left { line { dir #true } } }")
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

/// `icon-padding` must survive a round trip: the wizard writes the theme back through
/// `to_kdl`, and a dropped node would silently reset the option.
#[test]
fn icon_padding_round_trips() {
    let mut c = Config::parse("layout {\n    left { line { dir } }\n}\n").unwrap();
    c.icon_padding = IconPadding::Moderate;
    let back = Config::parse(&c.to_kdl()).unwrap();
    assert_eq!(back.icon_padding, IconPadding::Moderate);
}

#[test]
fn parses_joined_element_and_disabled() {
    // "dir_joined" shares the background; an explicit #false keeps the segment commented
    // out in the layout so a configure round trip does not drop it.
    let c = Config::parse(
        "layout {\n  left {\n    line { dir_joined #true; vcs #true; time #false }\n  }\n}",
    )
    .unwrap();
    assert_eq!(c.layout.left[0][0], Element::Joined("dir".into()));
    assert_eq!(c.layout.left[0].len(), 3, "#false time is kept");
    assert_eq!(c.layout.left[0][2], Element::Disabled("time".into()));
    let again = Config::parse(&c.to_kdl()).unwrap();
    assert_eq!(
        again.layout.left[0], c.layout.left[0],
        "the commented-out segment survives the round trip"
    );
}

/// `transient-prompt` accepts the boolean form earlier themes used plus p10k's `same-dir`
/// mode; both survive a round trip through the serializer.
#[test]
fn parses_the_three_transient_prompt_values() {
    let parse = |src: &str| {
        Config::parse(&format!(
            "layout {{\n    left {{ line {{ dir }} }}\n    {src}\n}}\n"
        ))
        .unwrap()
        .layout
        .transient_prompt
    };
    assert_eq!(parse("transient-prompt #true"), TransientPrompt::Always);
    assert_eq!(parse("transient-prompt #false"), TransientPrompt::Off);
    assert_eq!(
        parse("transient-prompt \"always\""),
        TransientPrompt::Always
    );
    assert_eq!(
        parse("transient-prompt \"same-dir\""),
        TransientPrompt::SameDir
    );

    for mode in [
        TransientPrompt::Off,
        TransientPrompt::Always,
        TransientPrompt::SameDir,
    ] {
        let mut c = Config::parse("layout {\n    left { line { dir } }\n}\n").unwrap();
        c.layout.transient_prompt = mode;
        let back = Config::parse(&c.to_kdl()).unwrap();
        assert_eq!(
            back.layout.transient_prompt, mode,
            "round trip for {mode:?}"
        );
    }
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
    assert_eq!(d.style.fg, Some(Color::Xterm(39)));
    assert_eq!(
        d.props["shorten-strategy"],
        Prop::Str("truncate_to_unique".into())
    );
    assert_eq!(d.states["SHORTENED"].style.fg, Some(Color::Xterm(103)));
    assert_eq!(d.states["ANCHOR"].style.bold, Some(true));
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
        Some(Color::Xterm(103))
    );
    assert_eq!(
        c.segment("dir").effective_style(None, &c.defaults).fg,
        Some(Color::Xterm(39))
    );
    assert_eq!(
        c.segment("nope").effective_style(None, &c.defaults).fg,
        Some(Color::Xterm(200))
    );
}

#[test]
fn default_lean_parses() {
    let c = Config::default_lean().unwrap();
    assert!(!c.segments.is_empty());
    let row = c
        .layout
        .input
        .as_ref()
        .expect("lean draws its segments on the row the user types on");
    assert!(
        c.layout.left.is_empty() && c.layout.right.is_empty(),
        "lean has no header row"
    );
    assert_eq!(
        row.left,
        vec![Element::Seg("dir".into()), Element::Seg("vcs".into())]
    );
    assert_eq!(
        row.right,
        vec![
            Element::Seg("status".into()),
            Element::Seg("command_execution_time".into()),
            Element::Seg("background_jobs".into())
        ]
    );
    assert!(c.segment("dir").props.contains_key("shorten-strategy"));
    // prompt_char colors the input-line prefix and is never a layout element.
    assert_eq!(c.segment("prompt_char").style.fg, Some(Color::Xterm(76)));
    for row in c
        .layout
        .left
        .iter()
        .chain(&c.layout.right)
        .chain([&row.left, &row.right])
    {
        assert!(
            !row.iter()
                .any(|e| matches!(e, Element::Seg(s) if s == "prompt_char")),
            "prompt_char belongs to the prefix, not to the layout: {row:?}"
        );
    }
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
    assert_eq!(c.defaults.fg, Some(Color::Rgb(255, 255, 255)));
    assert_eq!(c.segment("a").style.bold, Some(true));
    assert_eq!(c.segment("a").props["x"], Prop::Int(3));
}

/// The glyph tiers: ASCII, standard Unicode and the Nerd Font private-use areas.
#[test]
fn glyph_required_modes() {
    assert_eq!(glyph_required_mode('a'), IconMode::Ascii);
    assert_eq!(glyph_required_mode(' '), IconMode::Ascii);
    assert_eq!(glyph_required_mode('\u{276f}'), IconMode::Compatible); // ❯
    assert_eq!(glyph_required_mode('\u{2500}'), IconMode::Compatible); // ─
    assert_eq!(glyph_required_mode('\u{e0b0}'), IconMode::Compatible); // Powerline
    assert_eq!(glyph_required_mode('\u{e0a2}'), IconMode::Compatible); // Powerline
    assert_eq!(
        glyph_required_mode('\u{f115}'),
        IconMode::NerdfontFontconfig
    ); // Nerd Font
    assert_eq!(
        glyph_required_mode('\u{f0001}'),
        IconMode::NerdfontFontconfig
    ); // plane 15
    assert_eq!(
        glyph_required_mode('\u{100000}'),
        IconMode::NerdfontFontconfig
    ); // plane 16
}

/// Mode order and the fallback mode selection.
#[test]
fn mode_order_and_min() {
    assert_eq!(
        IconMode::Ascii.clone().min(IconMode::NerdfontComplete),
        IconMode::Ascii
    );
    assert_eq!(
        IconMode::Compatible.clone().min(IconMode::Ascii),
        IconMode::Ascii
    );
    assert_eq!(
        IconMode::NerdfontFontconfig
            .clone()
            .min(IconMode::NerdfontComplete),
        IconMode::NerdfontFontconfig
    );
    assert_eq!(IconMode::NerdfontComplete.name(), "nerdfont-complete");
    assert_eq!(IconMode::Ascii.name(), "ascii");
}

/// Declared glyphs the locale cannot display fall back to the built-in defaults with a
/// warning, and ASCII declarations stay.
#[test]
fn locale_fallback_rolls_back_declared_glyphs() {
    let mut c = Config::parse(
        "layout {}\nmode \"nerdfont-complete\"\n\
             frame { last-prefix \"╰─\" first-suffix \"─╮\" }\n\
             separators { segment \"\" gap \"·\" }\n\
             segments {\n\
                 prompt_char content=\"é\" {\n\
                     state \"ERROR\" char=\"❯\"\n\
                 }\n\
             }\n\
             icon {\n\
                 folder {\n\
                     all \"\\u{f115}\"\n\
                     ascii \"d\"\n\
                 }\n\
             }\n",
    )
    .unwrap();
    let warnings = c.apply_locale_fallback_with(IconMode::Ascii).unwrap();
    assert_eq!(c.mode, IconMode::Ascii);
    assert!(
        warnings.iter().any(|w| w.contains("falling back to")),
        "the mode fallback needs a warning: {warnings:?}"
    );
    assert_eq!(c.frame.last_prefix.text, "");
    assert_eq!(c.frame.first_suffix.text, "");
    assert_eq!(c.separators.segment, "");
    assert_eq!(c.separators.gap, "");
    let prompt_char = c.segment("prompt_char");
    assert_eq!(prompt_char.states["ERROR"].char, None);
    assert_eq!(prompt_char.content, None);
    let folder = &c.icon_overrides["folder"];
    assert_eq!(folder.all, None);
    assert_eq!(folder.ascii, Some("d".to_string()));
}

/// A UTF-8 locale can display the declared glyphs, so nothing is rolled back.
#[test]
fn locale_fallback_keeps_glyphs_for_utf8() {
    let mut c =
        Config::parse("layout {}\nmode \"nerdfont-complete\"\nframe { last-prefix \"╰─\" }\n")
            .unwrap();
    let warnings = c
        .apply_locale_fallback_with(IconMode::NerdfontComplete)
        .unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(c.mode, IconMode::NerdfontComplete);
    assert_eq!(c.frame.last_prefix.text, "╰─");
}

/// A declared ascii mode is kept under a UTF-8 locale, and its unrenderable glyphs still
/// roll back.
#[test]
fn locale_fallback_keeps_a_declared_ascii_mode() {
    let mut c = Config::parse("layout {}\nmode \"ascii\"\nframe { last-prefix \"╰─\" }\n").unwrap();
    let warnings = c
        .apply_locale_fallback_with(IconMode::NerdfontComplete)
        .unwrap();
    assert_eq!(c.mode, IconMode::Ascii);
    assert_eq!(c.frame.last_prefix.text, "");
    assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
}

/// The last-resort check reports the field of a glyph that no fallback removed.
#[test]
fn unrenderable_glyph_reports_the_field() {
    let c = Config::parse("layout {}\nframe { last-prefix \"╰─\" }\n").unwrap();
    let error = c.unrenderable_glyph(&IconMode::Ascii).unwrap();
    assert!(error.contains("frame last-prefix"), "got {error:?}");
}

/// Powerline glyphs stay renderable in compatible mode; the documentation and the built-in
/// compatible tier both use them.
#[test]
fn compatible_keeps_powerline_glyphs() {
    let mut c = Config::parse(
        "layout {}\nmode \"compatible\"\n\
             separators { segment \"\u{e0b0}\" }\n\
             segments { dir prefix=\"\u{e0b1}\" }\n\
             icon { lock { compat \"\u{e0a2}\" } }\n",
    )
    .unwrap();
    let warnings = c
        .apply_locale_fallback_with(IconMode::NerdfontComplete)
        .unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(c.separators.segment, "\u{e0b0}");
    assert_eq!(c.segment("dir").prefix.as_deref(), Some("\u{e0b1}"));
    assert_eq!(c.icon_overrides["lock"].compat.as_deref(), Some("\u{e0a2}"));
}

/// A slot for an inactive mode is kept and does not block startup, so the documented
/// per-mode override works in a C locale.
#[test]
fn inactive_icon_slots_do_not_block_startup() {
    let mut c = Config::parse(
        "layout {}\nmode \"nerdfont-complete\"\n\
             icon {\n\
                 folder {\n\
                     nf \"\u{f07c}\"\n\
                     compat \"\"\n\
                     ascii \"d\"\n\
                 }\n\
             }\n",
    )
    .unwrap();
    let result = c.apply_locale_fallback_with(IconMode::Ascii);
    assert!(result.is_ok(), "unexpected error: {result:?}");
    // The inactive `nf` slot survives for a later UTF-8 session.
    assert_eq!(c.icon_overrides["folder"].nf.as_deref(), Some("\u{f07c}"));
    assert_eq!(c.icon_overrides["folder"].ascii.as_deref(), Some("d"));
}

/// The explicit `char` property of `prompt_char` rolls back like the other declared glyphs.
#[test]
fn prompt_char_char_property_rolls_back() {
    let mut c = Config::parse(
        "layout {}\nmode \"nerdfont-complete\"\n\
             segments { prompt_char char=\"❯\" }\n",
    )
    .unwrap();
    let warnings = c.apply_locale_fallback_with(IconMode::Ascii).unwrap();
    assert_eq!(c.segment("prompt_char").props.get("char"), None);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("segment prompt_char char")),
        "the char fallback needs a warning: {warnings:?}"
    );
}

/// Two `state` nodes with the same name inside one segment merge by field, not replace.
#[test]
fn repeated_state_nodes_merge_by_field() {
    let c =
        Config::parse("layout {}\nsegments { dir { state \"X\" fg=1; state \"X\" bold=#true } }")
            .unwrap();
    let st = &c.segment("dir").states["X"];
    assert_eq!(st.style.fg, Some(Color::Xterm(1)));
    assert_eq!(st.style.bold, Some(true));
}
