//! Tests for the wizard flow, the questions and the previews.

use std::io;

use crate::config::{Color, Element, Frame, IconMode, Prop, Separators};
use crate::presets::{self, PresetKind};

use super::input::*;
use super::questions::*;
use super::ui::*;
use super::write::*;
use super::{Answer, Control};

#[test]
fn new_time_segment_gets_the_style_background() {
    // classic: the new segment follows the shade's background (taken from the existing dir
    // segment; in p10k this is the global POWERLEVEL9K_BACKGROUND); rainbow: each segment
    // has its own background (TIME_BACKGROUND=7); lean/pure: transparent.
    let classic_cfg = presets::classic(2);
    assert_eq!(time_bg(PresetKind::Classic, &classic_cfg), Some(238));
    let rainbow_cfg = presets::rainbow(1);
    assert_eq!(time_bg(PresetKind::Rainbow, &rainbow_cfg), Some(7));
    let lean_cfg = presets::lean(false);
    assert_eq!(time_bg(PresetKind::Lean, &lean_cfg), None);
    assert_eq!(time_bg(PresetKind::Pure, &presets::pure(false)), None);
}

/// Ctrl-C and Esc abort; ordinary answer keys pass through.
#[test]
fn key_normalization_keeps_answers_and_maps_aborts_to_q() {
    assert_eq!(normalize_key(0x03), b'q');
    assert_eq!(normalize_key(0x1b), b'q');
    assert_eq!(normalize_key(b'r'), b'r');
    assert_eq!(normalize_key(b'y'), b'y');
    assert_eq!(normalize_key(b'1'), b'1');
}

/// Scripted keyboard for the question flow: the bytes are answers, not terminal input.
struct FakeInput {
    bytes: std::collections::VecDeque<u8>,
}

impl FakeInput {
    fn new(bytes: &[u8]) -> FakeInput {
        FakeInput {
            bytes: bytes.iter().copied().collect(),
        }
    }
}

impl Input for FakeInput {
    fn byte(&mut self) -> io::Result<u8> {
        // EOF maps to `q`, matching the terminal implementation.
        Ok(self.bytes.pop_front().unwrap_or(b'q'))
    }

    fn readable_within(&mut self, _ms: i32) -> io::Result<bool> {
        Ok(!self.bytes.is_empty())
    }
}

fn ask_yn_scripted(bytes: &[u8]) -> Answer<bool> {
    ask_yn(
        &mut Vec::new(),
        &mut FakeInput::new(bytes),
        "Question?",
        "Yes.",
        "No.",
        None,
    )
}

#[test]
fn yes_no_reads_the_scripted_answer() {
    assert!(matches!(ask_yn_scripted(b"y"), Ok(true)));
    assert!(matches!(ask_yn_scripted(b"n"), Ok(false)));
    // An unknown key redraws the question and waits for the next one.
    assert!(matches!(ask_yn_scripted(b"xyz"), Ok(true)));
    // Arrow keys are consumed as a sequence, not as answers.
    assert!(matches!(ask_yn_scripted(b"\x1b[Ay"), Ok(true)));
    assert!(matches!(ask_yn_scripted(b"\x1b[3~n"), Ok(false)));
}

/// Run one question with scripted keys and a memory output.
fn run_question<T>(
    bytes: &[u8],
    f: impl FnOnce(&mut Vec<u8>, &mut FakeInput) -> Answer<T>,
) -> (Answer<T>, String) {
    let mut out = Vec::new();
    let mut input = FakeInput::new(bytes);
    let result = f(&mut out, &mut input);
    (result, String::from_utf8_lossy(&out).into_owned())
}

#[test]
fn ask_yn_title_uses_the_dynamic_title() {
    let (answer, screen) = run_question(b"y", |out, input| {
        ask_yn_title(out, input, "Overwrite ./p11k.kdl?", "Yes.", "No.", None)
    });
    assert!(matches!(answer, Ok(true)));
    assert!(
        screen.contains("Overwrite ./p11k.kdl?"),
        "screen: {screen:?}"
    );
}

#[test]
fn font_detection_maps_the_answer_pairs() {
    // Diamond + lock → Nerd Font Complete; diamond + no lock → Compatible.
    assert!(matches!(
        run_question(b"yy", |o, i| ask_font(o, i)).0,
        Ok(IconMode::NerdfontComplete)
    ));
    assert!(matches!(
        run_question(b"yn", |o, i| ask_font(o, i)).0,
        Ok(IconMode::Compatible)
    ));
    // No diamond: the `><` question; yes → Compatible, no → Ascii.
    assert!(matches!(
        run_question(b"ny", |o, i| ask_font(o, i)).0,
        Ok(IconMode::Compatible)
    ));
    assert!(matches!(
        run_question(b"nn", |o, i| ask_font(o, i)).0,
        Ok(IconMode::Ascii)
    ));
}

#[test]
fn style_options_map_to_the_presets() {
    for (key, kind) in [
        (b"1".as_slice(), PresetKind::Lean),
        (b"2", PresetKind::Classic),
        (b"3", PresetKind::Rainbow),
        (b"4", PresetKind::Pure),
    ] {
        let mode = IconMode::NerdfontComplete;
        let (answer, _) = run_question(key, |o, i| ask_style(o, i, &mode));
        assert!(
            matches!(answer, Ok(k) if k == kind),
            "key {key:?} → {answer:?}"
        );
    }
}

#[test]
fn color_options_build_the_matching_preset() {
    let mode = IconMode::NerdfontComplete;
    let cases = [
        (PresetKind::Lean, b"2".as_slice(), presets::lean(true)),
        (PresetKind::Classic, b"3", presets::classic(3)),
        (PresetKind::Rainbow, b"1", presets::rainbow(1)),
        (PresetKind::Pure, b"2", presets::pure(true)),
    ];
    for (kind, key, expected) in cases {
        let (answer, _) = run_question(key, |o, i| ask_color(o, i, kind, &mode));
        let got = answer.expect("a color answer");
        // Compare the serialized config: the preview and the result share one source.
        assert_eq!(got.to_kdl(), expected.to_kdl(), "{kind:?} key {key:?}");
        assert_eq!(got.mode, mode);
    }
}

#[test]
fn charset_reports_ascii_on_the_second_option() {
    let mut cfg = presets::lean(false);
    cfg.mode = IconMode::NerdfontComplete;
    let mode = IconMode::NerdfontComplete;
    let (answer, _) = run_question(b"1", |o, i| ask_charset(o, i, &cfg, &mode));
    assert!(
        matches!(answer, Ok(false)),
        "Unicode is not ASCII: {answer:?}"
    );
    let (answer, _) = run_question(b"2", |o, i| ask_charset(o, i, &cfg, &mode));
    assert!(matches!(answer, Ok(true)), "ASCII: {answer:?}");
}

/// The exec segment moves to the chosen column; a missing segment is added.
#[test]
fn non_permanent_content_moves_between_columns() {
    let mut cfg = presets::pure(false);
    remove_segment(&mut cfg, "command_execution_time");
    let (answer, _) = run_question(b"1", |o, i| ask_use_rprompt(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    let left_has = cfg
        .layout
        .left
        .iter()
        .any(|row| row.contains(&Element::Seg("command_execution_time".into())));
    assert!(
        left_has,
        "left column should hold exec: {:?}",
        cfg.layout.left
    );

    let (answer, _) = run_question(b"2", |o, i| ask_use_rprompt(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    let right_has = cfg
        .layout
        .right
        .iter()
        .any(|row| row.contains(&Element::Seg("command_execution_time".into())));
    assert!(
        right_has,
        "right column should hold exec: {:?}",
        cfg.layout.right
    );
    let left_has = cfg
        .layout
        .left
        .iter()
        .any(|row| row.contains(&Element::Seg("command_execution_time".into())));
    assert!(
        !left_has,
        "exec must leave the left column: {:?}",
        cfg.layout.left
    );
}

/// Time: the first option removes the segment; the other two set the format and the style
/// background for the preset.
#[test]
fn time_options_set_the_format_and_the_background() {
    // Classic derives the background from the existing dir segment.
    let mut cfg = presets::classic(2);
    let bg = match cfg.segment("dir").style.bg {
        Color::Xterm(n) => Some(n),
        _ => None,
    };
    let (answer, _) = run_question(b"2", |o, i| ask_time(o, i, &mut cfg, PresetKind::Classic));
    assert!(matches!(answer, Ok(())));
    let time = cfg.segment("time");
    assert_eq!(
        time.props.get("time-format"),
        Some(&Prop::Str("12h".into()))
    );
    assert_eq!(time.style.fg, Color::Xterm(66));
    assert_eq!(
        time.style.bg,
        bg.map(Color::Xterm).unwrap_or(Color::Default)
    );

    let (answer, _) = run_question(b"3", |o, i| ask_time(o, i, &mut cfg, PresetKind::Classic));
    assert!(matches!(answer, Ok(())));
    assert_eq!(
        cfg.segment("time").props.get("time-format"),
        Some(&Prop::Str("24h".into()))
    );

    let (answer, _) = run_question(b"1", |o, i| ask_time(o, i, &mut cfg, PresetKind::Classic));
    assert!(matches!(answer, Ok(())));
    let has = cfg
        .layout
        .left
        .iter()
        .chain(cfg.layout.right.iter())
        .any(|row| row.contains(&Element::Seg("time".into())));
    assert!(!has, "No should remove the time segment");
}

/// Separators, heads and tails: each option writes its glyph pair.
#[test]
fn separator_questions_write_their_glyphs() {
    let mut cfg = presets::classic(2);
    let (answer, _) = run_question(b"2", |o, i| ask_separators(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.segment, "");
    assert_eq!(cfg.separators.sub, "\u{2502}");
    assert_eq!(cfg.separators.right_segment, "");
    assert_eq!(cfg.separators.right_sub, "\u{2502}");
    let (answer, _) = run_question(b"4", |o, i| ask_separators(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.segment, "\u{e0b4}");

    let (answer, _) = run_question(b"2", |o, i| ask_heads(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.end, "\u{2593}\u{2592}\u{2591}");
    assert_eq!(cfg.separators.right_start, "\u{2591}\u{2592}\u{2593}");
    let (answer, _) = run_question(b"3", |o, i| ask_heads(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.end, "\u{e0b0}");

    let (answer, _) = run_question(b"2", |o, i| ask_tails(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.left_tail, "\u{2591}\u{2592}\u{2593}");
    assert_eq!(cfg.separators.right_tail, "\u{2593}\u{2592}\u{2591}");
}

#[test]
fn height_and_spacing_options() {
    let mut cfg = presets::classic(2);
    let (answer, _) = run_question(b"1", |o, i| ask_num_lines(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert!(cfg.layout.left.is_empty() && cfg.layout.right.is_empty());
    assert_eq!(cfg.frame, Frame::default());
    assert_eq!(cfg.separators, Separators::default());

    let mut cfg = presets::classic(2);
    let (answer, _) = run_question(b"2", |o, i| ask_empty_line(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.layout.prompt_add_newline, 1);
    let (answer, _) = run_question(b"1", |o, i| ask_empty_line(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.layout.prompt_add_newline, 0);
}

#[test]
fn connection_and_frame_options() {
    let mut cfg = presets::classic(2);
    let (answer, _) = run_question(b"2", |o, i| ask_gap_char(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.gap, "\u{b7}");
    assert_eq!(cfg.separators.gap_foreground, Some(Color::Xterm(240)));
    let (answer, _) = run_question(b"1", |o, i| ask_gap_char(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.separators.gap, " ");
    assert_eq!(cfg.separators.gap_foreground, None);

    let mut cfg = presets::classic(2);
    let (answer, _) = run_question(b"2", |o, i| ask_frame(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    // Left frame: the suffix is cleared, the prefix stays.
    assert_ne!(cfg.frame.first_prefix, crate::config::FramePiece::default());
    assert_eq!(cfg.frame.first_suffix, crate::config::FramePiece::default());
    let (answer, _) = run_question(b"3", |o, i| ask_frame(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    // Right frame: the prefix is cleared.
    assert_eq!(cfg.frame.first_prefix, crate::config::FramePiece::default());
}

/// Few icons removes the os badge, the remote icon table and empties the icon overrides.
#[test]
fn few_icons_clears_the_optional_icons() {
    let mut cfg = presets::lean(false);
    cfg.vcs_remote_icons
        .push(("github".into(), "\u{e709}".into()));
    let (answer, _) = run_question(b"1", |o, i| ask_extra_icons(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert!(!cfg.segments.contains_key("os_icon"));
    assert!(cfg.vcs_remote_icons.is_empty());
    for key in ["folder", "git", "branch", "time", "execution-time"] {
        let o = cfg.icon_overrides.get(key).expect("an override");
        assert_eq!(o.all.as_deref(), Some(""), "override {key}");
    }

    let mut cfg = presets::lean(false);
    let (answer, _) = run_question(b"2", |o, i| ask_extra_icons(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert!(cfg.icon_overrides.is_empty(), "Many keeps the icons");
}

#[test]
fn prefixes_and_transient_options() {
    let mut cfg = presets::classic(2);
    // The time segment exists only after the time question; ask it so its prefix is covered.
    let (answer, _) = run_question(b"2", |o, i| ask_time(o, i, &mut cfg, PresetKind::Classic));
    assert!(matches!(answer, Ok(())));
    let (answer, _) = run_question(b"2", |o, i| ask_prefixes(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.segment("vcs").prefix.as_deref(), Some("on "));
    assert_eq!(
        cfg.segment("command_execution_time").prefix.as_deref(),
        Some("took ")
    );
    assert_eq!(cfg.segment("time").prefix.as_deref(), Some("at "));
    let (answer, _) = run_question(b"1", |o, i| ask_prefixes(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.segment("vcs").prefix, None);

    let mut cfg = presets::lean(false);
    let (answer, _) = run_question(b"y", |o, i| ask_transient(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(
        cfg.layout.transient_prompt,
        crate::config::TransientPrompt::Always
    );
    let (answer, _) = run_question(b"n", |o, i| ask_transient(o, i, &mut cfg));
    assert!(matches!(answer, Ok(())));
    assert_eq!(
        cfg.layout.transient_prompt,
        crate::config::TransientPrompt::Off
    );
}

/// apply applies the same edit that the preview showed, on a clone.
#[test]
fn ask_apply_edits_the_real_config() {
    let mut cfg = presets::lean(false);
    let (answer, screen) = run_question(b"2", |o, i| {
        ask_apply(o, i, "Pick.", &["A.", "B."], &mut cfg, |c, n| {
            c.layout.prompt_add_newline = n;
        })
    });
    assert!(matches!(answer, Ok(())));
    assert_eq!(cfg.layout.prompt_add_newline, 1);
    assert!(screen.contains("Pick."), "screen: {screen:?}");
    // The questions offer restart and quit on every screen.
    assert!(
        screen.contains("(r)") && screen.contains("(q)"),
        "screen: {screen:?}"
    );
}

#[test]
fn control_keys_leave_the_question() {
    assert!(matches!(ask_yn_scripted(b"q"), Err(Control::Quit)));
    assert!(matches!(ask_yn_scripted(b"r"), Err(Control::Restart)));
    // Ctrl-C and a lone Esc abort like `q`.
    assert!(matches!(ask_yn_scripted(b"\x03"), Err(Control::Quit)));
    assert!(matches!(ask_yn_scripted(b"\x1b"), Err(Control::Quit)));
}

fn choice_scripted(bytes: &[u8], options: &[&str]) -> Answer<usize> {
    ask_choice_preview(
        &mut Vec::new(),
        &mut FakeInput::new(bytes),
        "Pick.",
        options,
        |_| Vec::new(),
    )
}

#[test]
fn choice_reads_the_scripted_number() {
    assert!(matches!(choice_scripted(b"1", &["a", "b"]), Ok(0)));
    assert!(matches!(choice_scripted(b"2", &["a", "b"]), Ok(1)));
    // A number past the last option is ignored; the next answer counts.
    assert!(matches!(choice_scripted(b"9\n2", &["a", "b"]), Ok(1)));
}

/// CSI/SS3 final bytes end an escape sequence; digits and punctuation inside it do not.
#[test]
fn escape_sequences_are_consumed_to_their_final_byte() {
    // Up arrow `ESC [ A`, Delete `ESC [ 3 ~`, SS3 up `ESC O A`.
    assert!(is_escape_final(b'A'));
    assert!(is_escape_final(b'~'));
    assert!(!is_escape_final(b'3'));
    assert!(!is_escape_final(b';'));
}

/// The style preview has to use the chosen icon mode. ASCII and Nerd Font produce different
/// glyphs, so the rendered previews differ.
#[test]
fn the_style_preview_uses_the_icon_mode() {
    let ascii = style_preview(PresetKind::Lean, &IconMode::Ascii);
    let nerd = style_preview(PresetKind::Lean, &IconMode::NerdfontComplete);
    assert_ne!(ascii, nerd);
}
