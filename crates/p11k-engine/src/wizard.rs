//! `p11k configure` — the interactive configuration wizard.
//!
//! Aligned with the question set p10k `p10k configure` exposes: font detection (three glyphs
//! confirmed by eye), style, character set, color variant, time, separators/ends, height,
//! connection, frame, spacing, icons, prefixes, transient. Answers edit [`Config`] fields
//! directly and the result is serialized to KDL on disk. Keys: `q` quits (writing nothing),
//! `r` restarts, digits/letters select.

use crate::config::{Color, Config, Element, Frame, IconMode, Prop, Segment, Separators};
use crate::i18n::{msgid, t};
use crate::presets::{self, PresetKind};
use crate::theme::HeaderInfo;
use std::io::{self, Write};

enum Step<T> {
    Answer(T),
    Restart,
    Quit,
}

/// Convert any `Step<T>`'s Restart/Quit variants into `Step<U>` (Answer never reaches here).
fn early<T, U>(s: Step<T>) -> Step<U> {
    match s {
        Step::Answer(_) => unreachable!("early only handles Restart/Quit"),
        Step::Restart => Step::Restart,
        Step::Quit => Step::Quit,
    }
}

pub fn run() -> anyhow::Result<()> {
    let _raw = crate::RawTerminal::enter(libc::STDIN_FILENO)?;
    let mut out = io::stdout();
    loop {
        match flow(&mut out)? {
            Step::Answer(()) => return Ok(()),
            Step::Restart => continue,
            Step::Quit => return goodbye(&mut out),
        }
    }
}

fn flow(out: &mut io::Stdout) -> io::Result<Step<()>> {
    // 1. Font detection → icon mode.
    let mode = match ask_font(out)? {
        Step::Answer(m) => m,
        s => return Ok(early(s)),
    };
    // 2. Style.
    let kind = match ask_style(out)? {
        Step::Answer(k) => k,
        s => return Ok(early(s)),
    };
    // 3. Color variant (branched by style) → build a Config with the color arguments.
    let mut cfg = match ask_color(out, kind, &mode)? {
        Step::Answer(c) => c,
        s => return Ok(early(s)),
    };
    // 4. Character set (asked when not ascii; can explicitly switch to ASCII).
    if cfg.mode != IconMode::Ascii {
        match ask_charset(out, &cfg, &mode)? {
            Step::Answer(ascii) => {
                if ascii {
                    cfg.mode = IconMode::Ascii;
                }
            }
            s => return Ok(early(s)),
        }
    }
    // 5. Location of pure's non-permanent content.
    if kind == PresetKind::Pure {
        match ask_use_rprompt(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
    }
    // 6. Time.
    match ask_time(out, &mut cfg, kind)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 7. Separators/ends (classic/rainbow only).
    if matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        match ask_separators(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
        match ask_heads(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
        match ask_tails(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
    }
    // 8. Height.
    match ask_num_lines(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 9. Connection line (two lines and classic/rainbow only).
    if header_on(&cfg) && matches!(kind, PresetKind::Classic | PresetKind::Rainbow) {
        match ask_gap_char(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
        match ask_frame(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
    }
    // 10. Spacing.
    match ask_empty_line(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 11. Icons.
    match ask_extra_icons(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 12. Prefixes.
    match ask_prefixes(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 13. transient.
    match ask_transient(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // 14. Preview + confirmation.
    preview(out, &cfg)?;
    match ask_yn(
        out,
        msgid("Use this theme?"),
        msgid("Yes, write the config."),
        msgid("No."),
        None,
    )? {
        Step::Answer(true) => {}
        Step::Answer(false) => return Ok(Step::Quit),
        Step::Restart => return Ok(Step::Restart),
        Step::Quit => return Ok(Step::Quit),
    }
    // 15. Write to disk (asking about overwrite first if it exists).
    write_config(out, &cfg).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Step::Answer(()))
}

// ---- UI basics ----

fn clear(out: &mut io::Stdout) {
    let _ = out.write_all(b"\x1b[2J\x1b[H");
    let _ = out.flush();
}

/// Write one line (raw terminal has OPOST off, so `\n` does not return to column 0; `\r` is added explicitly).
fn line(out: &mut io::Stdout, s: &str) -> io::Result<()> {
    out.write_all(s.as_bytes())?;
    out.write_all(b"\r\n")?;
    out.flush()
}

/// Read one key and normalize it: `q`, Ctrl-C (ISIG is cleared in raw mode, so 0x03 arrives
/// as a byte) and Esc all count as `q` (abort and exit), and EOF counts as `q` too — otherwise
/// the caller would receive `None`, fall into `_ => {}` and redraw forever. Other bytes are
/// returned as is; their meaning (`r`/`y`/`n`/digits) is up to the caller.
fn key() -> io::Result<u8> {
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
            return Ok(b'q'); // EOF (Ctrl-D) is treated as aborting
        }
        return Ok(match b[0] {
            0x03 | 0x1b => b'q', // Ctrl-C / Esc
            other => other,
        });
    }
}

fn goodbye(out: &mut io::Stdout) -> anyhow::Result<()> {
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
        jobs: 1,
        history: 0,
        pipestatus: Vec::new(),
    }
}

fn header_on(cfg: &Config) -> bool {
    !(cfg.layout.left.is_empty() && cfg.layout.right.is_empty())
}

/// Sample git status: makes the preview's vcs segment look like a real prompt (p10k's wizard uses fake data too).
fn sample_vcs() -> crate::theme::GitStatus {
    crate::theme::GitStatus {
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
    let cols = crate::tty_size().map(|(_, c, ..)| c as usize).unwrap_or(80);
    let vcs = sample_vcs();
    let mut lines = crate::render::render_header_lines(cfg, &sample_info(), Some(&vcs), cols);
    // Input line: the prompt_char prefix the engine draws (the shell's buffer follows).
    lines.push(crate::render::input_prefix(cfg, None).text);
    lines
}

// ---- Question set ----

/// Font detection: diamond (⮀⮂) + lock ()/quotes (❯❮) — three glyphs landing on the three mode tiers.
fn ask_font(out: &mut io::Stdout) -> io::Result<Step<IconMode>> {
    let diamond = match ask_yn(
        out,
        msgid("Does this look like a diamond (rotated square)?"),
        msgid("Yes."),
        msgid("No."),
        Some("--->  \u{e0b2}\u{e0b0}  <---"),
    )? {
        Step::Answer(b) => b,
        s => return Ok(early(s)),
    };
    if diamond {
        match ask_yn(
            out,
            msgid("Does this look like a lock?"),
            msgid("Yes."),
            msgid("No."),
            Some("--->  \u{f023}  <---"),
        )? {
            Step::Answer(true) => Ok(Step::Answer(IconMode::NerdfontComplete)),
            Step::Answer(false) => Ok(Step::Answer(IconMode::Compatible)),
            s => Ok(early(s)),
        }
    } else {
        match ask_yn(
            out,
            msgid("Does this look like >< but taller and fatter?"),
            msgid("Yes."),
            msgid("No."),
            Some("--->  \u{276f}\u{276e}  <---"),
        )? {
            Step::Answer(true) => Ok(Step::Answer(IconMode::Compatible)),
            Step::Answer(false) => Ok(Step::Answer(IconMode::Ascii)),
            s => Ok(early(s)),
        }
    }
}

/// Style: each of the four presets comes with a live prompt preview.
fn ask_style(out: &mut io::Stdout) -> io::Result<Step<PresetKind>> {
    let titles = PresetKind::ALL.map(|k| k.title());
    let i = ask_choice_preview(out, msgid("Prompt Style"), &titles, |i| {
        preview_lines(&presets::build(PresetKind::ALL[i]))
    })?;
    match i {
        Step::Answer(i) => Ok(Step::Answer(PresetKind::ALL[i])),
        s => Ok(early(s)),
    }
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
fn ask_color(out: &mut io::Stdout, kind: PresetKind, mode: &IconMode) -> io::Result<Step<Config>> {
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
    let i = ask_choice_preview(out, title, options, |i| {
        let mut c = build_with_color(kind, i);
        c.mode = mode.clone();
        preview_lines(&c)
    })?;
    match i {
        Step::Answer(i) => {
            let mut c = build_with_color(kind, i);
            c.mode = mode.clone();
            Ok(Step::Answer(c))
        }
        s => Ok(early(s)),
    }
}

/// Character set: Unicode / ASCII (the preview reflects the icon/separator substitutions under ASCII).
fn ask_charset(out: &mut io::Stdout, cfg: &Config, mode: &IconMode) -> io::Result<Step<bool>> {
    let i = ask_choice_preview(
        out,
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
    match i {
        Step::Answer(i) => Ok(Step::Answer(i == 1)),
        s => Ok(early(s)),
    }
}

/// Location of pure's non-permanent content (exec/context/virtualenv).
fn ask_use_rprompt(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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

fn ask_time(out: &mut io::Stdout, cfg: &mut Config, kind: PresetKind) -> io::Result<Step<()>> {
    ask_apply(
        out,
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
fn ask_separators(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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
fn ask_heads(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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
fn ask_tails(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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
fn ask_num_lines(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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

fn ask_gap_char(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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

fn ask_frame(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    use crate::config::FramePiece;
    ask_apply(
        out,
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

fn ask_empty_line(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        msgid("Prompt Spacing"),
        &[msgid("Compact."), msgid("Sparse.")],
        cfg,
        |c, i| c.layout.prompt_add_newline = usize::from(i == 1),
    )
}

fn ask_extra_icons(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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
fn ask_prefixes(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
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

fn ask_transient(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let b = ask_yn(
        out,
        msgid("Enable Transient Prompt?"),
        msgid("Yes."),
        msgid("No."),
        None,
    )?;
    match b {
        Step::Answer(b) => {
            cfg.layout.transient_prompt = b;
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

// ---- Generic prompts ----

/// Pick 1 of n, rendering each option's live prompt preview below it (aligns with p10k
/// wizard's add_prompt: every entry carries a full prompt under the current config).
fn ask_choice_preview(
    out: &mut io::Stdout,
    title: &str,
    options: &[&str],
    preview: impl Fn(usize) -> Vec<String>,
) -> io::Result<Step<usize>> {
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
        match key()? {
            b'q' => return Ok(Step::Quit),
            b'r' => return Ok(Step::Restart),
            k @ b'1'..=b'9' => {
                let i = (k - b'1') as usize;
                if i < options.len() {
                    return Ok(Step::Answer(i));
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
    out: &mut io::Stdout,
    title: &str,
    options: &[&str],
    cfg: &mut Config,
    apply: impl Fn(&mut Config, usize),
) -> io::Result<Step<()>> {
    let i = {
        let base = cfg.clone();
        ask_choice_preview(out, title, options, |i| {
            let mut c = base.clone();
            apply(&mut c, i);
            preview_lines(&c)
        })?
    };
    match i {
        Step::Answer(i) => {
            apply(cfg, i);
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// Yes/no question; when `sample` is present, a line with the glyph sample is shown before the options.
fn ask_yn(
    out: &mut io::Stdout,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> io::Result<Step<bool>> {
    loop {
        clear(out);
        line(out, &t(title))?;
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
        match key()? {
            b'q' => return Ok(Step::Quit),
            b'r' => return Ok(Step::Restart),
            b'y' => return Ok(Step::Answer(true)),
            b'n' => return Ok(Step::Answer(false)),
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

fn preview(out: &mut io::Stdout, cfg: &Config) -> io::Result<()> {
    clear(out);
    line(out, &t("Preview (current directory + exit code):"))?;
    line(out, "")?;
    for l in preview_lines(cfg) {
        line(out, &l)?;
    }
    line(out, "")?;
    Ok(())
}

fn write_config(out: &mut io::Stdout, cfg: &Config) -> anyhow::Result<()> {
    let path = crate::config::default_config_path();
    let p = path.display();
    if path.exists() {
        // p10k: `p11k config file already exists. Overwrite <path>?`
        match ask_yn(
            out,
            &format!(
                "{} {p}?",
                t(msgid("p11k config file already exists. Overwrite"))
            ),
            msgid("Yes."),
            msgid("No."),
            None,
        )? {
            Step::Answer(true) => {}
            _ => return Ok(()),
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
    line(out, &t("Add the engine to your shell rc (pick yours):"))?;
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
}
