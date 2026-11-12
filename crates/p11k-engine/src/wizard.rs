//! `p11k configure` — the interactive configuration wizard.
//!
//! Aligned with the question set p10k `p10k configure` exposes: font detection (three glyphs
//! confirmed by eye), style, character set, color variant, time, separators/ends, height,
//! connection, frame, spacing, icons, prefixes, transient. Answers edit [`Config`] fields
//! directly and the result is serialized to KDL on disk. Keys: `q` quits (writing nothing),
//! `r` restarts, digits/letters select.

use crate::config::{Color, Config, Element, Frame, IconMode, Prop, Segment, Separators};
use crate::i18n::{msgid, t};
use crate::model::HeaderInfo;
use crate::presets::{self, PresetKind};
use std::io::{self, Write};

/// Why a question stopped: the user asked to restart or quit, or a write failed.
#[derive(Debug)]
enum Control {
    Restart,
    Quit,
    Io(io::Error),
}

impl From<io::Error> for Control {
    fn from(e: io::Error) -> Control {
        Control::Io(e)
    }
}

/// One answer; an error carries the control flow out of the wizard.
type Answer<T> = Result<T, Control>;

pub fn run() -> anyhow::Result<()> {
    // The wizard reads single keystrokes; without a terminal there is nothing to read and the
    // raw-mode switch cannot apply. Fail instead of reading EOF as an answer.
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 0 {
        anyhow::bail!("{}", t("p11k configure needs an interactive terminal"));
    }
    let _raw = crate::state::RawTerminal::enter(libc::STDIN_FILENO)?;
    let mut out = io::stdout();
    let mut input = StdinInput;
    loop {
        match flow(&mut out, &mut input) {
            Ok(()) => return Ok(()),
            Err(Control::Restart) => continue,
            Err(Control::Quit) => return goodbye(&mut out),
            Err(Control::Io(e)) => return Err(e.into()),
        }
    }
}

fn flow(out: &mut dyn Write, input: &mut dyn Input) -> Answer<()> {
    // 1. Font detection → icon mode.
    let mode = ask_font(out, input)?;
    // 2. Style.
    let kind = ask_style(out, input, &mode)?;
    // 3. Color variant (branched by style) → build a Config with the color arguments.
    let mut cfg = ask_color(out, input, kind, &mode)?;
    // 4. Character set (asked when not ascii; can explicitly switch to ASCII).
    if cfg.mode != IconMode::Ascii && ask_charset(out, input, &cfg, &mode)? {
        cfg.mode = IconMode::Ascii;
    }
    // 5. Location of pure's non-permanent content.
    if kind == PresetKind::Pure {
        ask_use_rprompt(out, input, &mut cfg)?;
    }
    // 6. Time.
    ask_time(out, input, &mut cfg, kind)?;
    // 7. Separators/ends (classic/rainbow only).
    if matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        ask_separators(out, input, &mut cfg)?;
        ask_heads(out, input, &mut cfg)?;
        ask_tails(out, input, &mut cfg)?;
    }
    // 8. Height.
    ask_num_lines(out, input, &mut cfg)?;
    // 9. Connection line (two lines and classic/rainbow only).
    if header_on(&cfg) && matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        ask_gap_char(out, input, &mut cfg)?;
        ask_frame(out, input, &mut cfg)?;
    }
    // 10. Spacing.
    ask_empty_line(out, input, &mut cfg)?;
    // 11. Icons.
    ask_extra_icons(out, input, &mut cfg)?;
    // 12. Prefixes.
    ask_prefixes(out, input, &mut cfg)?;
    // 13. transient.
    ask_transient(out, input, &mut cfg)?;
    // 14. Preview + confirmation.
    preview(out, &cfg)?;
    if !ask_yn(
        out,
        input,
        msgid("Use this theme?"),
        msgid("Yes, write the config."),
        msgid("No."),
        None,
    )? {
        return Err(Control::Quit);
    }
    // 15. Write to disk (asking about overwrite first if it exists).
    write_config(out, input, &cfg)?;
    Ok(())
}

// ---- UI basics ----

fn clear(out: &mut dyn Write) {
    let _ = out.write_all(b"\x1b[2J\x1b[H");
    let _ = out.flush();
}

/// Write one line (raw terminal has OPOST off, so `\n` does not return to column 0; `\r` is added explicitly).
fn line(out: &mut dyn Write, s: &str) -> io::Result<()> {
    out.write_all(s.as_bytes())?;
    out.write_all(b"\r\n")?;
    out.flush()
}

/// Keyboard input of the wizard. Production reads the stdin byte stream; tests script the
/// answers, so the question flow can run without a terminal.
trait Input {
    fn byte(&mut self) -> io::Result<u8>;
    fn readable_within(&mut self, ms: i32) -> io::Result<bool>;
}

/// The stdin keyboard.
struct StdinInput;

impl Input for StdinInput {
    fn byte(&mut self) -> io::Result<u8> {
        read_byte()
    }

    fn readable_within(&mut self, ms: i32) -> io::Result<bool> {
        readable_within(ms)
    }
}

/// Read one key and normalize it: `q`, Ctrl-C (ISIG is cleared in raw mode, so 0x03 arrives
/// as a byte) and Esc all count as `q` (abort and exit), and EOF counts as `q` too — otherwise
/// the caller would receive `None`, fall into `_ => {}` and redraw forever. Arrow keys and other
/// escape sequences are consumed and ignored: a direction key is not an answer and must not quit
/// the wizard. Other bytes are returned as is; their meaning (`r`/`y`/`n`/digits) is up to the
/// caller.
fn key(input: &mut dyn Input) -> io::Result<u8> {
    loop {
        let b = input.byte()?;
        if b != 0x1b {
            return Ok(normalize_key(b));
        }
        // Esc alone aborts. A sequence (arrow keys, Home, End, Delete, ...) follows within a few
        // milliseconds; wait for that follow-up byte instead of treating the Esc as final.
        if !input.readable_within(20)? {
            return Ok(b'q');
        }
        let next = input.byte()?;
        if next == b'[' || next == b'O' {
            // Consume the parameter, intermediate and final bytes of the CSI or SS3 sequence.
            loop {
                let c = input.byte()?;
                if is_escape_final(c) {
                    break;
                }
            }
            continue;
        }
        return Ok(b'q');
    }
}

/// Read one byte from the terminal. EOF maps to `q` so callers cannot loop on `None`.
fn read_byte() -> io::Result<u8> {
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(libc::STDIN_FILENO, b.as_mut_ptr().cast(), 1) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(b'q');
        }
        return Ok(b[0]);
    }
}

/// Ctrl-C and a lone Esc abort; every other byte is an answer.
fn normalize_key(byte: u8) -> u8 {
    match byte {
        0x03 | 0x1b => b'q',
        other => other,
    }
}

/// Final byte of a CSI/SS3 sequence (0x40..=0x7e). Parameter and intermediate bytes are below
/// that range, so this finds the end of arrow-key and similar sequences.
fn is_escape_final(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte)
}

/// Whether stdin has a byte within `ms` milliseconds. `false` also covers the lone-Esc case when
/// the terminal sent nothing else.
fn readable_within(ms: i32) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        return Ok(n > 0);
    }
}

fn goodbye(out: &mut dyn Write) -> anyhow::Result<()> {
    let _ = line(out, "");
    let _ = line(out, &t("p11k configuration wizard has been aborted."));
    let _ = line(out, &t("Nothing was written."));
    Ok(())
}

/// Current directory + sample state that makes the exec/status segments render content.
fn sample_info() -> HeaderInfo {
    HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
        exec_seconds: 3.5,
        exec_ms: None,
        jobs: 1,
        history: 0,
        pipestatus: Vec::new(),
    }
}

fn header_on(cfg: &Config) -> bool {
    !(cfg.layout.left.is_empty() && cfg.layout.right.is_empty())
}

/// Sample git status: makes the preview's vcs segment look like a real prompt (p10k's wizard uses fake data too).
fn sample_vcs() -> crate::model::GitStatus {
    crate::model::GitStatus {
        branch: "main".into(),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        staged: 0,
        unstaged: 2,
        conflicted: 0,
        untracked: 1,
        ahead: 1,
        behind: 0,
        push_ahead: 0,
        push_behind: 0,
        stashes: 0,
        action: String::new(),
        tag: String::new(),
        remote_branch: String::new(),
        commit_summary: "Add feature".into(),
        index_size: 3,
        remote_url: "https://github.com/example/repo".into(),
    }
}

/// Render a full prompt preview of the current config (header lines + input line) for per-option previews.
fn preview_lines(cfg: &Config) -> Vec<String> {
    let cols = crate::state::tty_size()
        .map(|(_, c, ..)| c as usize)
        .unwrap_or(80);
    let vcs = sample_vcs();
    let mut lines = crate::render::render_header_lines(cfg, &sample_info(), Some(&vcs), cols);
    // Input line: the prompt_char prefix the engine draws (the shell's buffer follows).
    lines.push(crate::render::input_prefix(cfg, None, crate::model::ViMode::Unreported).text);
    lines
}

// ---- Question set ----

/// Font detection: diamond (⮀⮂) + lock ()/quotes (❯❮) — three glyphs landing on the three mode tiers.
fn ask_font(out: &mut dyn Write, input: &mut dyn Input) -> Answer<IconMode> {
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
fn style_preview(kind: PresetKind, mode: &IconMode) -> Vec<String> {
    let mut cfg = presets::build(kind);
    cfg.mode = mode.clone();
    preview_lines(&cfg)
}

/// Style: each of the four presets comes with a live prompt preview.
fn ask_style(out: &mut dyn Write, input: &mut dyn Input, mode: &IconMode) -> Answer<PresetKind> {
    let titles = PresetKind::ALL.map(|k| k.title());
    let i = ask_choice_preview(out, input, msgid("Prompt Style"), &titles, |i| {
        style_preview(PresetKind::ALL[i], mode)
    })?;
    Ok(PresetKind::ALL[i])
}

/// Build a config from style + color shade index (preview and final result share one source).
fn build_with_color(kind: PresetKind, i: usize) -> Config {
    match kind {
        PresetKind::Lean => presets::lean(i == 1),
        PresetKind::Classic => presets::classic(i + 1),
        PresetKind::Rainbow => presets::rainbow(i + 1),
        PresetKind::Pure => presets::pure(i == 1),
    }
}

/// Color variant (branched by style): lean 256/8, classic four shades, rainbow four frame shades, pure two palettes.
fn ask_color(
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
fn ask_charset(
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
fn ask_use_rprompt(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

fn ask_time(
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
fn time_bg(kind: PresetKind, cfg: &Config) -> Option<u8> {
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
fn ask_separators(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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
fn ask_heads(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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
fn ask_tails(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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
fn ask_num_lines(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

fn ask_gap_char(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

fn ask_frame(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

fn ask_empty_line(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
    ask_apply(
        out,
        input,
        msgid("Prompt Spacing"),
        &[msgid("Compact."), msgid("Sparse.")],
        cfg,
        |c, i| c.layout.prompt_add_newline = usize::from(i == 1),
    )
}

fn ask_extra_icons(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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
fn ask_prefixes(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

fn ask_transient(out: &mut dyn Write, input: &mut dyn Input, cfg: &mut Config) -> Answer<()> {
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

// ---- Generic prompts ----

/// Pick 1 of n, rendering each option's live prompt preview below it (aligns with p10k
/// wizard's add_prompt: every entry carries a full prompt under the current config).
fn ask_choice_preview(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    options: &[&str],
    preview: impl Fn(usize) -> Vec<String>,
) -> Answer<usize> {
    loop {
        clear(out);
        // The title and option labels are msgids, translated here in one place; preview lines
        // are real prompts rendered from the config and do not go through gettext.
        line(out, &t(title))?;
        line(out, "")?;
        for (i, label) in options.iter().enumerate() {
            line(out, &format!("({})  {}", i + 1, t(label)))?;
            for l in preview(i) {
                line(out, &l)?;
            }
            line(out, "")?;
        }
        line(out, &format!("(r)  {}", t("Restart from the beginning.")))?;
        line(out, &format!("(q)  {}", t("Quit")))?;
        match key(input)? {
            b'q' => return Err(Control::Quit),
            b'r' => return Err(Control::Restart),
            k @ b'1'..=b'9' => {
                let i = (k - b'1') as usize;
                if i < options.len() {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
}

/// Previewed-question skeleton: clone the current config → apply the i-th option → render the
/// preview; on selection apply the same edit to the real config (preview and result share one
/// source and cannot drift).
fn ask_apply(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    options: &[&str],
    cfg: &mut Config,
    apply: impl Fn(&mut Config, usize),
) -> Answer<()> {
    let i = {
        let base = cfg.clone();
        ask_choice_preview(out, input, title, options, |i| {
            let mut c = base.clone();
            apply(&mut c, i);
            preview_lines(&c)
        })?
    };
    apply(cfg, i);
    Ok(())
}

/// Yes/no question; when `sample` is present, a line with the glyph sample is shown before the
/// options. `title`, `yes` and `no` are msgids.
fn ask_yn(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> Answer<bool> {
    ask_yn_title(out, input, &t(title), yes, no, sample)
}

/// Like [`ask_yn`] with an already translated title. Dynamic titles (an overwrite question with a
/// path) go through this variant so gettext sees the text once.
fn ask_yn_title(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> Answer<bool> {
    loop {
        clear(out);
        line(out, title)?;
        if let Some(s) = sample {
            line(out, "")?;
            // The glyph sample is not text; emit it verbatim.
            line(out, s)?;
        }
        line(out, "")?;
        line(out, &format!("(y)  {}", t(yes)))?;
        line(out, "")?;
        line(out, &format!("(n)  {}", t(no)))?;
        line(out, &format!("(r)  {}", t("Restart from the beginning.")))?;
        line(out, &format!("(q)  {}", t("Quit")))?;
        match key(input)? {
            b'q' => return Err(Control::Quit),
            b'r' => return Err(Control::Restart),
            b'y' => return Ok(true),
            b'n' => return Ok(false),
            _ => {}
        }
    }
}

// ---- Config editing helpers ----

fn remove_segment(cfg: &mut Config, name: &str) {
    for row in cfg
        .layout
        .left
        .iter_mut()
        .chain(cfg.layout.right.iter_mut())
    {
        row.retain(|e| !matches!(e, Element::Seg(s) if s == name));
    }
}

fn add_to_right(cfg: &mut Config, name: &str) {
    if cfg
        .layout
        .right
        .iter()
        .flatten()
        .any(|e| matches!(e, Element::Seg(s) if s == name))
    {
        return;
    }
    if cfg.layout.right.is_empty() {
        cfg.layout.right.push(Vec::new());
    }
    cfg.layout.right[0].push(Element::Seg(name.into()));
}

fn set_prefix(cfg: &mut Config, name: &str, prefix: &str) {
    if let Some(s) = cfg.segments.get_mut(name) {
        s.prefix = Some(prefix.into());
    }
}

// ---- Preview / writing to disk ----

fn preview(out: &mut dyn Write, cfg: &Config) -> io::Result<()> {
    clear(out);
    line(out, &t("Preview (current directory + exit code):"))?;
    line(out, "")?;
    for l in preview_lines(cfg) {
        line(out, &l)?;
    }
    line(out, "")?;
    Ok(())
}

fn write_config(out: &mut dyn Write, input: &mut dyn Input, cfg: &Config) -> Answer<()> {
    let path = crate::config::default_config_path();
    let p = path.display();
    if path.exists() {
        // p10k: `p11k config file already exists. Overwrite <path>?`
        let title = format!(
            "{} {p}?",
            t(msgid("p11k config file already exists. Overwrite"))
        );
        if !ask_yn_title(out, input, &title, msgid("Yes."), msgid("No."), None)? {
            return Err(Control::Quit);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, cfg.to_kdl())?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "p11k".into());
    line(out, "")?;
    // p10k: `New config: <path>.`
    line(out, &format!("{} {p}.", t("New config:")))?;
    line(out, "")?;
    line(out, &t("Add one of these to your shell rc:"))?;
    line(
        out,
        &format!("  zsh:  [[ -z \"$P11K_ENGINE\" ]] && exec {exe} --shell zsh --config {p}"),
    )?;
    line(
        out,
        &format!("  bash: [[ -z \"$P11K_ENGINE\" ]] && exec {exe} --shell bash --config {p}"),
    )?;
    line(
        out,
        &format!("  fish: if not set -q P11K_ENGINE; exec {exe} --shell fish --config {p}; end"),
    )?;
    // pwsh has no exec, so start the engine, wait for it to exit, then exit the outer pwsh. This line goes into $PROFILE.
    line(
        out,
        &format!(
            "  pwsh: if (-not $env:P11K_ENGINE) {{ & '{exe}' --shell pwsh --config '{p}'; exit }}"
        ),
    )?;
    line(out, "")?;
    line(
        out,
        &t(
            "The engine is the theme: remove any ZSH_THEME/theme setup first, otherwise they will stack.",
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
