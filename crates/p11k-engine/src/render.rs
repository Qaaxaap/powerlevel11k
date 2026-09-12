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

use std::fmt::Write as _;

use crate::config::{AttachText, Color, Config, Element, Segment, Style};
use crate::i18n::t;
use crate::theme::{GitStatus, HeaderInfo};
use std::cell::RefCell;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

/// Current zsh edit mode (reported by the zle hook, updated live by the engine); empty when
/// not zsh or unreported.
pub static CURRENT_VI_MODE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Input line prefix: `frame.last_prefix` + `prompt_char` content; `width` is its display
/// width, from which the engine generates equal-width placeholders.
pub struct InputPrefix {
    pub text: String,
    pub width: usize,
}

/// vi state for the current zsh edit mode (p10k's `VIINS`/`VICMD`/`VIVIS`/`VIOWR`).
fn vi_state() -> Option<&'static str> {
    let mode = CURRENT_VI_MODE
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    match mode.as_str() {
        "viins" => Some("VIINS"),
        "vicmd" => Some("VICMD"),
        "vis" | "viopp" => Some("VIVIS"),
        "viowr" => Some("VIOWR"),
        _ => None,
    }
}

/// prompt_char state. vi mode wins (p10k
/// `PROMPT_CHAR_{OK,ERROR}_{VIINS,VICMD,VIVIS,VIOWR}`)—but only when a char is really
/// configured for that state; otherwise the exit code decides: non-zero → ERROR, else normal.
/// State for the input-line prefix: the vi mode's own state when it has one, otherwise the
/// status state. Extended states mirror p10k's `STATUS_EXTENDED_STATES`: a failing pipeline
/// is `ERROR_PIPE`, death by signal is `ERROR_SIGNAL`, and a pipeline whose last stage
/// succeeded while an earlier one failed is `OK_PIPE`.
fn prompt_state(config: &Config, info: Option<&HeaderInfo>) -> Option<&'static str> {
    if let Some(st) = vi_state() {
        let configured = config
            .segment("prompt_char")
            .states
            .get(st)
            .map(|s| s.char.is_some())
            .unwrap_or(false);
        if configured {
            return Some(st);
        }
    }
    let info = info?;
    let Some(code) = info.exit_code else {
        return None; // first prompt: nothing has run yet
    };
    let pipes = &info.pipestatus;
    let state = if code == 0 {
        if pipes.len() > 1 && pipes.iter().any(|&c| c != 0) {
            "OK_PIPE"
        } else {
            return None;
        }
    } else if pipes.len() > 1 {
        "ERROR_PIPE"
    } else if code > 128 {
        "ERROR_SIGNAL"
    } else {
        "ERROR"
    };
    // p10k gives every state its own toggle (`STATUS_ERROR_PIPE` and friends). Here a state
    // the config defines wins; otherwise fall back to the plain failure state.
    if segment_has_state(config, state) {
        Some(state)
    } else if state.starts_with("ERROR") {
        Some("ERROR")
    } else {
        None
    }
}

/// Whether the `prompt_char` segment declares this state at all.
fn segment_has_state(config: &Config, state: &str) -> bool {
    config.segment("prompt_char").states.contains_key(state)
}

/// Input line prefix text for a state (frame last_prefix + prompt_char char + space).
fn prefix_text(config: &Config, state: Option<&str>) -> String {
    let mut text = String::new();
    if !config.frame.last_prefix.text.is_empty() {
        let piece = &config.frame.last_prefix;
        text.push_str(&paint(&piece.text, &config.frame_piece_style(piece)));
    }
    let pc = config.segment("prompt_char");
    let st = pc.effective_style(state, &config.defaults);
    text.push_str(&paint(pc.char_for(state, "❯"), &st));
    text.push(' '); // space after the prefix (uncolored), matching the original `❯ ` geometry
    text
}

fn prefix_width(config: &Config, state: Option<&str>) -> usize {
    display_width(&prefix_text(config, state))
}

/// Computes the input line prefix (e.g. `╰─❯`). `last_prefix` (frame style) and the prompt
/// char (prompt_char style, entering ERROR state from `exit_code`) are colored; `width` is
/// the ANSI-stripped display width.
pub fn input_prefix(config: &Config, info: Option<&HeaderInfo>) -> InputPrefix {
    let state = prompt_state(config, info);
    let text = prefix_text(config, state);
    let width = display_width(&text);
    InputPrefix { text, width }
}

/// Single-line prompt for transient folding (zsh format). The engine precomputes the zsh
/// prompt escapes; zsh swaps PROMPT in zle-line-finish and folds in sync via reset-prompt.
/// Not easily portable to bash/fish, hence the zsh-only approach.
/// Uses native `%F{...}` escapes;
/// the conditional `%(?\x01OK\x01ERR)` picks a color from the last command's exit code.
pub fn transient_prompt_zsh(config: &Config) -> String {
    let pc = config.segment("prompt_char");
    let ch = pc.char_for(None, "❯");
    let ok = pc.effective_style(None, &config.defaults);
    let err = pc.effective_style(Some("ERROR"), &config.defaults);
    let ok_prompt = format!("{}{} ", zsh_fg(&ok.fg), ch);
    let err_prompt = format!("{}{} ", zsh_fg(&err.fg), ch);
    format!("%(?\u{1}{}\u{1}{})%f", ok_prompt, err_prompt)
}

/// Color → zsh `%F{...}` foreground escape (for the transient prompt).
fn zsh_fg(c: &Color) -> String {
    match c {
        Color::Default => "%f".into(),
        Color::Xterm(n) => format!("%F{{{n}}}"),
        Color::Rgb(r, g, b) => format!("%F{{#{r:02x}{g:02x}{b:02x}}}"),
        Color::Named(n) => format!("%F{{{}}}", named_256(n)),
    }
}

/// Validates prompt_char: every state with a char (e.g. ERROR) must be as wide as the normal
/// one. The prompt width must be fixed at startup (the placeholder protocol is generated from
/// it), so a mismatch breaks the geometry and is reported as a startup error.
pub fn check_prompt_char_widths(config: &Config) -> Result<(), String> {
    let pc = config.segment("prompt_char");
    let base = prefix_width(config, None);
    for (name, spec) in &pc.states {
        if spec.char.is_some() {
            let w = prefix_width(config, Some(name));
            if w != base {
                return Err(format!(
                    "{}: prompt_char state `{name}` is {w} wide, the normal state is {base}",
                    t(
                        "prompt_char states must all be the same width (the prompt width is fixed at startup)"
                    )
                ));
            }
        }
    }
    Ok(())
}

struct SegmentText {
    text: String,
    style: Style,
    /// Whether the text already carries the segment's trailing padding; segments without a
    /// background get no extra space between them.
    padded: bool,
}

/// Renders the header as per-line content (each line = left segments + right-aligned right
/// segments, without cursor/clear/newline). The theme layer draws it line by line as
/// `\r\e[K` + content + `\r\n`.
pub fn render_header_lines(
    config: &Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    cols: usize,
) -> Vec<String> {
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
                let render_side = |budget: Option<usize>| {
                    let l = left
                        .get(i)
                        .map(|seg| render_row(config, seg, info, vcs, false, budget))
                        .unwrap_or_default();
                    let r = right
                        .get(i)
                        .map(|seg| render_row(config, seg, info, vcs, true, budget))
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
                // Right-align budget = cols - prefix width - suffix width, or the frame widens
                // the row and pushes the suffix to the next line.
                let body_budget = cols.saturating_sub(pre_w + suf_w);
                let seps = &config.separators;
                let indent = config.layout.right_indent;
                // Width of each column (the right one including its start separator), used to
                // decide whether the whole row fits. p10k's test is stricter than "just
                // fits": content + indent + 1 must fit (measured: the same content is hidden
                // at 33 cols and shown at 34, while the content is only 32 cols wide).
                // Passing cols 0 for the right column alone skips the gap calculation.
                let left_w =
                    |l: &[SegmentText]| display_width(&assemble_row(l, &[], body_budget, seps, 0));
                let right_w = |r: &[SegmentText]| display_width(&assemble_row(&[], r, 0, seps, 0));
                let fits = |lw: usize, rw: usize| lw + rw + indent < body_budget;
                // Pass 1: no shortening at all (p10k leaves it as-is when it fits). Shortening
                // budgets always use the **unshortened** width, otherwise shortening from an
                // already shortened width would shorten less than needed.
                let (l0, r0) = render_side(None);
                let (lw0, rw0) = (left_w(&l0), right_w(&r0));
                let body = if fits(lw0, rw0) {
                    assemble_row(&l0, &r0, body_budget, seps, indent)
                } else {
                    // Pass 2: shorten dir by the number of overflowing columns (narrow
                    // terminals shorten, wide terminals do not).
                    let overflow = (lw0 + rw0 + indent + 1).saturating_sub(body_budget);
                    let (l1, r1) = render_side(Some(overflow));
                    let (lw1, rw1) = (left_w(&l1), right_w(&r1));
                    if fits(lw1, rw1) {
                        assemble_row(&l1, &r1, body_budget, seps, indent)
                    } else {
                        // Pass 3: still does not fit after shortening → drop the whole right
                        // column (gap included). p10k omits the right column when the width is
                        // insufficient instead of letting it overflow. The left column is
                        // shortened again by its overflow over the unshortened width.
                        let (l2, _) = render_side(Some(lw0.saturating_sub(body_budget)));
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

fn render_row(
    config: &Config,
    elements: &[Element],
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    right: bool,
    dir_budget: Option<usize>,
) -> Vec<SegmentText> {
    elements
        .iter()
        .map(|el| match el {
            Element::Seg(name) => render_segment(config, name, info, vcs, right, dir_budget),
            Element::Joined(name) => render_segment(config, name, info, vcs, right, dir_budget),
            Element::Text(t) => {
                let mut style = config
                    .segment("text")
                    .effective_style(None, &config.defaults);
                if style.bg == Color::Default {
                    style.bg = if config.defaults.bg != Color::Default {
                        config.defaults.bg.clone()
                    } else {
                        Color::Xterm(0)
                    };
                }
                SegmentText {
                    text: paint(&expand_env(t), &style),
                    style,
                    padded: false,
                }
            }
        })
        .collect()
}

/// Renders one segment. The returned `text` is already colored ANSI; `style` feeds
/// inter-segment separators and block backgrounds.
fn render_segment(
    config: &Config,
    name: &str,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    right: bool,
    dir_budget: Option<usize>,
) -> SegmentText {
    let seg = config.segment(name);
    let mut style = seg.effective_style(None, &config.defaults);
    // Leading space is added when the effective background is non-transparent (inheritance
    // chain below); transparent segments get none—matching p10k lean, no space at line start.
    // Effective segment background: the segment's own → global `defaults.bg` (p10k's
    // `POWERLEVEL9K_BACKGROUND`) → transparent when neither (the terminal's own color).
    // A forced black background used to draw black blocks on non-black terminals and made the
    // right-column separators misread as "different background" and draw arrows. The
    // `effective_style` inheritance already does this layer; the bg here is the result.
    let has_real_bg = style.bg != Color::Default;
    let text = match name {
        "dir" => dir_seg_text(config, info, seg, &style, dir_budget),
        "vcs" => {
            let branch = icon_str(config, "branch").unwrap_or_default();
            let commit = icon_str(config, "commit").unwrap_or_default();
            if seg.content.is_some() {
                paint(&value_of(seg.content.as_deref(), String::new()), &style)
            } else {
                vcs_text(vcs, &branch, &commit, seg, &style)
            }
        }
        "status" => {
            let ok = icon_str(config, "ok").unwrap_or_default();
            let err = icon_str(config, "error").unwrap_or_default();
            status_text(info, &ok, &err, seg, &style)
        }
        "prompt_char" => {
            let state = prompt_state(config, Some(info));
            style = seg.effective_style(state, &config.defaults);
            paint(seg.char_for(state, "❯"), &style)
        }
        "time" => paint(&time_text(seg), &style),
        "command_execution_time" => {
            let threshold = match seg.prop("threshold-seconds") {
                Some(crate::config::Prop::Int(n)) => (*n as f64).max(0.0),
                _ => 3.0,
            };
            if info.exec_seconds >= threshold {
                // p10k's PRECISION defaults to 2 decimals and FORMAT to multi-level (with
                // spaces).
                let precision = match seg.prop("precision") {
                    Some(crate::config::Prop::Int(n)) => (*n).clamp(0, 6) as usize,
                    _ => 2,
                };
                let hms = matches!(
                    seg.prop("format"),
                    Some(crate::config::Prop::Str(s)) if s == "H:M:S"
                );
                paint(&format_duration(info.exec_seconds, precision, hms), &style)
            } else {
                paint("", &style)
            }
        }
        "background_jobs" => {
            if info.jobs > 0 {
                paint(&info.jobs.to_string(), &style)
            } else {
                paint("", &style)
            }
        }
        "context" => paint(&context_text(), &style),
        "user" => paint(&user_name(), &style),
        "host" => paint(&host_text(), &style),
        "root_indicator" => paint(&root_indicator_text(), &style),
        "date" => paint(&date_text(seg), &style),
        "symfony2_version" => paint(&symfony2_version_text(&info.cwd), &style),
        // p10k symfony2_tests: the test ratio selects the GOOD/AVG/BAD state
        // (p10k's internal default colors cyan/yellow/red = 6/3/1).
        "symfony2_tests" => {
            let text = symfony2_tests_text(&info.cwd);
            if text.is_empty() {
                paint("", &style)
            } else {
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
                let st = if seg.states.contains_key(state) {
                    seg.effective_style(Some(state), &config.defaults)
                } else {
                    Style {
                        fg: Color::Xterm(default_fg),
                        ..style.clone()
                    }
                };
                paint(&text, &st)
            }
        }
        // Toolchain version segments: run `cmd --version` and parse the version; shown only
        // when the command exists.
        "go_version" => paint(&go_version(), &style),
        "rust_version" => paint(&rust_version(), &style),
        "node_version" => paint(&node_version(), &style),
        "php_version" => paint(&php_version(), &style),
        "java_version" => paint(&java_version(), &style),
        "dotnet_version" => paint(&dotnet_version(), &style),
        "swift_version" => paint(&swift_version(), &style),
        "terraform_version" => paint(&terraform_version(), &style),
        "cpu_arch" => paint(&cpu_arch(), &style),
        // Environment managers and the *env family: read env vars or ancestor version files;
        // shown only when active.
        "virtualenv" => paint(&virtualenv_text(&info.cwd), &style),
        "anaconda" => paint(&anaconda_text(&info.cwd), &style),
        "pyenv" => paint(&pyenv_text(&info.cwd), &style),
        "nodeenv" => paint(&nodeenv_text(&info.cwd), &style),
        "nodenv" => paint(&nodenv_text(&info.cwd), &style),
        "nvm" => paint(&nvm_text(&info.cwd), &style),
        "rbenv" => paint(&rbenv_text(&info.cwd), &style),
        "chruby" => paint(&chruby_text(&info.cwd), &style),
        "rvm" => paint(&rvm_text(&info.cwd), &style),
        "goenv" => paint(&goenv_text(&info.cwd), &style),
        "jenv" => paint(&jenv_text(&info.cwd), &style),
        "phpenv" => paint(&phpenv_text(&info.cwd), &style),
        "luaenv" => paint(&luaenv_text(&info.cwd), &style),
        "plenv" => paint(&plenv_text(&info.cwd), &style),
        "scalaenv" => paint(&scalaenv_text(&info.cwd), &style),
        "perlbrew" => paint(&perlbrew_text(&info.cwd), &style),
        // System resource segments: read /proc, /sys or df; hidden when the data source is
        // missing.
        "load" => paint(&load(), &style),
        "ram" => paint(&ram(), &style),
        "swap" => paint(&swap(), &style),
        "disk_usage" => paint(&disk_usage(&info.cwd), &style),
        "battery" => paint(&battery(), &style),
        // Cloud/k8s segments: read cloud config or env vars; hidden when the config is
        // missing.
        "aws" => paint(&aws_text(), &style),
        "azure" => paint(&azure_text(), &style),
        "gcloud" => paint(&gcloud_text(), &style),
        "kubecontext" => paint(&kubecontext_text(&info.cwd), &style),
        "terraform" => paint(&terraform_text(&info.cwd), &style),
        // Network segments: local IP/VPN/WiFi (synchronous), public IP (async query cache).
        "ip" => paint(&ip_text(), &style),
        "vpn_ip" => paint(&vpn_ip_text(), &style),
        "wifi" => paint(&wifi_text(), &style),
        "public_ip" => paint(&public_ip_text(), &style),
        // Remaining: virtualization / container / dir permissions / history scope / language
        // stacks / package managers.
        "detect_virt" => paint(&detect_virt(), &style),
        "toolbox" => paint(&toolbox_text(), &style),
        "dir_writable" => paint(&dir_writable_text(&info.cwd), &style),
        "per_directory_history" => paint(&per_directory_history_text(), &style),
        "haskell_stack" => paint(&haskell_stack_text(), &style),
        "package" => paint(&package_text(&info.cwd), &style),
        "asdf" => paint(&asdf_text(&info.cwd), &style),
        "fvm" => paint(&fvm_text(&info.cwd), &style),
        // Cloud sub-segments / frameworks / todos: shown only when the command exists or a
        // file matches.
        "google_app_cred" => paint(&google_app_cred_text(), &style),
        "aws_eb_env" => paint(&aws_eb_env_text(), &style),
        "laravel_version" => paint(&laravel_version_text(&info.cwd), &style),
        "rspec_stats" => paint(&rspec_stats_text(&info.cwd), &style),
        "todo" => paint(&todo_text(), &style),
        "taskwarrior" => paint(&taskwarrior_text(), &style),
        "dropbox" => paint(&dropbox_text(), &style),
        // Environment indicator segments: content comes from env vars; an unmet condition →
        // empty text + no icon (the default is given per condition) and the whole segment is
        // hidden.
        "ssh" | "xplr" | "midnight_commander" | "vim_shell" | "direnv" | "chezmoi_shell" => {
            paint("", &style)
        }
        "proxy" => paint(&proxy_text(), &style),
        "docker_machine" => paint(&env_var("DOCKER_MACHINE_NAME").unwrap_or_default(), &style),
        "openfoam" => paint(&openfoam_text(), &style),
        "ranger" => paint(&level_text("RANGER_LEVEL"), &style),
        "yazi" => paint(&level_text("YAZI_LEVEL"), &style),
        "nnn" => paint(&level_text("NNNLVL"), &style),
        "lf" => paint(&level_text("LF_LEVEL"), &style),
        "nix_shell" => paint(&nix_shell_text(), &style),
        "history" => paint(&info.history.to_string(), &style),
        "vi_mode" => paint(&vi_mode_text(seg), &style),
        _ => paint(&value_of(seg.content.as_deref(), String::new()), &style),
    };
    // Segment icon: config `icon` (auto-overrides the applicable mode by char class) →
    // otherwise the built-in default table.
    // The space after the icon (p10k `LEFT_MIDDLE_WHITESPACE`) is added only when the segment
    // has both an icon and content—an icon-only segment (e.g. os_icon) gets none, or it would
    // stack with the trailing padding into two spaces.
    // Segments with empty text: only "icon is content" segments (env indicators, os badges)
    // draw the icon; the rest hide entirely—matching p10k: no vcs icon outside a git repo, no
    // gear when jobs=0.
    let icon = resolve_icon(config, name, vcs, &info.cwd);
    // Icon color: `visual-identifier-color` (p10k `SEG_VISUAL_IDENTIFIER_COLOR`) wins; the vcs
    // segment falls back to `clean-foreground` when unset—the icon indicates the repo and p10k
    // also defaults it to green (icons do not inherit the segment default, or the default
    // theme would render them in the terminal default color).
    let icon_style = if seg.prop("visual-identifier-color").is_some() {
        prop_style(seg, &style, "visual-identifier-color")
    } else if name == "vcs" {
        prop_style(seg, &style, "clean-foreground")
    } else {
        style.clone()
    };
    let icon_text = match icon {
        Some(ic) if text.is_empty() => {
            if icon_is_content(name) {
                paint(&ic, &icon_style)
            } else {
                String::new()
            }
        }
        // The space between icon and content (p10k `LEFT_MIDDLE_WHITESPACE` /
        // `RIGHT_MIDDLE_WHITESPACE`) sits between them: left column has the icon first →
        // icon + space; right column has it last → space + icon. The reverse would give
        // "content icon " on the right.
        Some(ic) if right => paint(&format!(" {ic}"), &icon_style),
        Some(ic) => paint(&format!("{ic} "), &icon_style),
        None => String::new(),
    };
    // Attach text slots: left (before the icon) / middle (between icon and content, requires
    // both) / right (after the content). Concatenated only, never separate blocks; fg defaults
    // to the segment style. text-middle always sits right between icon and content: with the
    // left column's icon first → middle after the icon; with the right column's icon last →
    // middle between content and icon.
    // Segment prefix/suffix (p10k `SEG_PREFIX`/`SEG_SUFFIX`, e.g. vcs `on `, exec `took `):
    // drawn at the very start/end of the segment, colored with the segment style.
    // p10k segment whitespace (`LEFT_LEFT_WHITESPACE` / `LEFT_RIGHT_WHITESPACE`, one space each
    // by default): a space around the segment content so both sides of a separator
    // (subsep/arrow) have whitespace instead of touching. Segments without a background get no
    // leading space—matching p10k lean, nothing at line start.
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
    if right {
        body.push_str(&text);
        body.push_str(&middle);
        body.push_str(&icon_text);
    } else {
        body.push_str(&icon_text);
        body.push_str(&middle);
        body.push_str(&text);
    }
    if let Some(r) = &seg.text_right {
        body.push_str(&paint_attach(&style, r));
    }
    if let Some(s) = &seg.suffix {
        body.push_str(&paint(s, &style));
    }
    // A conditional segment with an unmet condition has nothing to show → hide it entirely,
    // leaving no side whitespace.
    if body.is_empty() {
        return SegmentText {
            text: String::new(),
            style,
            padded: false,
        };
    }
    let mut out = String::new();
    // Leading space: always for segments with a background; unconditional for right-column
    // segments, which are always preceded by a separator, and p10k's separator is followed by
    // a left_space. Transparent segments at the start of the left column get none.
    if has_real_bg || right {
        out.push_str(&paint(" ", &style));
    }
    out.push_str(&body);
    out.push_str(&paint(" ", &style));
    SegmentText {
        text: out,
        style,
        padded: true,
    }
}

/// Colors attach text: fg defaults to the segment style, when set it overrides the foreground
/// (bg/bold still inherit the segment style).
fn paint_attach(style: &Style, a: &AttachText) -> String {
    let mut st = style.clone();
    if let Some(fg) = &a.fg {
        st.fg = fg.clone();
    }
    paint(&a.text, &st)
}

/// The built-in icon chars of a segment across the three font tiers.
/// `nerdfont-complete` and `nerdfont-fontconfig` are the same case branch with identical
/// glyphs in p10k's source (`internal/icons.zsh`), so they merge into one `nerdfont` tier;
/// `compatible` uses standard Unicode + Powerline fonts and `ascii` is plain ASCII.
/// The icon data lives in the single [`icon_default`] table.
#[derive(Clone, Copy)]
struct IconEntry {
    nerdfont: &'static str,
    compatible: &'static str,
    ascii: &'static str,
}

fn icon_by_mode(e: &IconEntry, mode: &crate::config::IconMode) -> String {
    match mode {
        crate::config::IconMode::NerdfontComplete | crate::config::IconMode::NerdfontFontconfig => {
            e.nerdfont
        }
        crate::config::IconMode::Compatible => e.compatible,
        crate::config::IconMode::Ascii => e.ascii,
    }
    .to_string()
}

/// Icon table construction helper: three-tier chars → [`IconEntry`].
const fn icon(n: &'static str, c: &'static str, a: &'static str) -> IconEntry {
    IconEntry {
        nerdfont: n,
        compatible: c,
        ascii: a,
    }
}

/// Icon name → default chars for the three font tiers.
/// Segment rendering resolves icon names via [`segment_icon_key`];
/// users can override them by icon name in the top-level `icon{}`.
fn icon_default(key: &str) -> Option<IconEntry> {
    match key {
        "folder" => Some(icon("\u{f07c}", "", "")), // 
        "git" => Some(icon("\u{f1d3}", "", "")),
        "commit" => Some(icon("\u{e821}", "", "")), // p10k VCS_COMMIT_ICON    // 
        "time" => Some(icon("\u{f017}", "", "")),   // clock
        "date" => Some(icon("\u{f073}", "", "")),   // calendar
        // p10k EXECUTION_TIME_ICON: U+F252 (hourglass) in the nerdfont tier,
        // empty in compatible/ascii since p10k itself has none.
        "execution-time" => Some(icon("\u{f252}", "", "")),
        // p10k RULER_CHAR: `─` in the nerdfont/compatible tiers, `-` in ascii.
        "ruler" => Some(icon("\u{2500}", "\u{2500}", "-")),
        "background-jobs" => Some(icon("\u{f013}", "\u{2699}", "%%")), // gear ⚙ %%
        "go" => Some(icon("\u{e626}", "Go", "go")),
        "rust" => Some(icon("\u{e7a8}", "R", "rust")),
        "node" => Some(icon("\u{e617}", "Node", "node")),
        "php" => Some(icon("\u{e608}", "php", "php")),
        "java" => Some(icon("\u{e738}", "\u{2615}", "java")), // ☕
        "dotnet" => Some(icon("\u{e77f}", ".NET", ".net")),
        "terraform" => Some(icon("\u{f1bb}", "tf", "tf")),
        "cpu-arch" => Some(icon("\u{e266}", "arch", "arch")),
        "python" => Some(icon("\u{e73c}", "Py", "py")),
        "ruby" => Some(icon("\u{f219}", "Ruby", "rb")),
        "lua" => Some(icon("\u{e620}", "lua", "lua")),
        "perl" => Some(icon("\u{e769}", "perl", "perl")),
        "scala" => Some(icon("\u{e737}", "scala", "scala")),
        "load" => Some(icon("\u{f080}", "L", "cpu")),
        "ram" => Some(icon("\u{f0e4}", "RAM", "ram")),
        "swap" => Some(icon("\u{f464}", "SWP", "swap")),
        "battery" => Some(icon("\u{f240}", "\u{1F50B}", "battery")), // 🔋
        "disk" => Some(icon("\u{f0a0}", "hdd", "disk")),
        "aws" => Some(icon("\u{f270}", "AWS", "aws")),
        "azure" => Some(icon("\u{fd03}", "\u{2601}", "az")), // ☁
        "gcloud" => Some(icon("\u{f7b7}", "G", "gcloud")),
        "kube" => Some(icon("\u{2388}", "\u{2388}", "kube")), // ⎈
        "network" => Some(icon("\u{f50d}", "IP", "ip")),
        "vpn" => Some(icon("\u{f023}", "vpn", "vpn")),
        "wifi" => Some(icon("\u{f1eb}", "WiFi", "wifi")),
        "public-ip" => Some(icon("\u{f0ac}", "IP", "ip")),
        "toolbox" => Some(icon("\u{e20f}", "\u{2b22}", "toolbox")), // ⬢
        "lock" => Some(icon("\u{f023}", "\u{e0a2}", "!w")),
        "history" => Some(icon("\u{f1da}", "hist", "hist")),
        "haskell" => Some(icon("\u{e61f}", "hs", "hs")),
        "package" => Some(icon("\u{f8d6}", "pkg", "pkg")),
        "flutter" => Some(icon("F", "F", "flutter")),
        "aws-eb" => Some(icon("\u{f1bd}", "\u{1F331}", "eb")), // 🌱
        "laravel" => Some(icon("\u{e73f}", "", "")),
        "test" => Some(icon("\u{f188}", "", "")),
        // p10k SYMFONY_ICON: U+E757 in the nerdfont tier, `SF` in the others.
        "symfony" => Some(icon("\u{e757}", "SF", "SF")),
        "todo" => Some(icon("\u{2611}", "\u{2206}", "todo")), // ☑ ∆
        "taskwarrior" => Some(icon("\u{f4a0}", "task", "task")),
        "dropbox" => Some(icon("\u{f16b}", "Dropbox", "dropbox")),
        "ssh" => Some(icon("\u{f489}", "ssh", "ssh")),
        "proxy" => Some(icon("\u{2194}", "\u{2194}", "proxy")), // ↔
        "server" => Some(icon("\u{f0ae}", "", "")),
        "ranger" => Some(icon("\u{f00b}", "\u{2b50}", "ranger")), // ⭐
        "yazi" => Some(icon("\u{f00b}", "\u{2b50}", "yazi")),
        "nnn" => Some(icon("nnn", "nnn", "nnn")),
        "lf" => Some(icon("lf", "lf", "lf")),
        "nix" => Some(icon("\u{f313}", "nix", "nix")),
        "xplr" => Some(icon("xplr", "xplr", "xplr")),
        "mc" => Some(icon("mc", "mc", "mc")),
        "vim" => Some(icon("\u{e62b}", "vim", "vim")),
        "direnv" => Some(icon("\u{25bc}", "\u{25bc}", "direnv")), // ▼
        "chezmoi" => Some(icon("\u{f015}", "Chez", "chezmoi")),
        // status OK/ERROR and the vcs branch (p10k OK_ICON/FAIL_ICON/VCS_BRANCH_ICON).
        "ok" => Some(icon("\u{f00c}", "\u{2714}", "ok")),
        "error" => Some(icon("\u{f00d}", "\u{2718}", "err")),
        "branch" => Some(icon("\u{f126}", "@", "")),
        _ => None,
    }
}

/// Segment → the icon name it references by default. Conditional segments (shown only when
/// the environment allows) evaluate their condition here; unmet → None (no icon, which hides
/// the segment together with its empty text).
fn segment_icon_key(name: &str) -> Option<&'static str> {
    match name {
        "dir" => Some("folder"),
        "vcs" => Some("git"),
        "time" => Some("time"),
        "date" => Some("date"),
        "command_execution_time" => Some("execution-time"),
        "symfony2_version" => Some("symfony"),
        "symfony2_tests" => Some("test"),
        "background_jobs" => Some("background-jobs"),
        "go_version" | "goenv" => Some("go"),
        "rust_version" => Some("rust"),
        "node_version" | "nodeenv" | "nodenv" | "nvm" => Some("node"),
        "php_version" | "phpenv" => Some("php"),
        "java_version" | "jenv" => Some("java"),
        "dotnet_version" => Some("dotnet"),
        "terraform_version" | "terraform" => Some("terraform"),
        "cpu_arch" => Some("cpu-arch"),
        "virtualenv" | "anaconda" | "pyenv" => Some("python"),
        "rbenv" | "chruby" | "rvm" => Some("ruby"),
        "luaenv" => Some("lua"),
        "plenv" | "perlbrew" => Some("perl"),
        "scalaenv" => Some("scala"),
        "load" => Some("load"),
        "ram" => Some("ram"),
        "swap" => Some("swap"),
        "battery" => Some("battery"),
        "disk_usage" => Some("disk"),
        "aws" => Some("aws"),
        "azure" => Some("azure"),
        "gcloud" | "google_app_cred" => Some("gcloud"),
        "kubecontext" => Some("kube"),
        "ip" => Some("network"),
        "vpn_ip" => Some("vpn"),
        "wifi" => Some("wifi"),
        "public_ip" => Some("public-ip"),
        "toolbox" => (!toolbox_text().is_empty()).then_some("toolbox"),
        "dir_writable" => (!dir_writable_text(&current_dir()).is_empty()).then_some("lock"),
        "per_directory_history" => (!per_directory_history_text().is_empty()).then_some("history"),
        "haskell_stack" => (!haskell_stack_text().is_empty()).then_some("haskell"),
        "package" => (!package_text(&current_dir()).is_empty()).then_some("package"),
        "fvm" => (!fvm_text(&current_dir()).is_empty()).then_some("flutter"),
        "aws_eb_env" => (!aws_eb_env_text().is_empty()).then_some("aws-eb"),
        "laravel_version" => {
            (!laravel_version_text(&current_dir()).is_empty()).then_some("laravel")
        }
        "rspec_stats" => (!rspec_stats_text(&current_dir()).is_empty()).then_some("test"),
        "todo" => (!todo_text().is_empty()).then_some("todo"),
        "taskwarrior" => (!taskwarrior_text().is_empty()).then_some("taskwarrior"),
        "dropbox" => (!dropbox_text().is_empty()).then_some("dropbox"),
        // Environment indicator segments: the icon is conditional too; unmet → None.
        "ssh" => env().ssh.then_some("ssh"),
        "proxy" => env_has_proxy().then_some("proxy"),
        "docker_machine" => env_var("DOCKER_MACHINE_NAME").is_some().then_some("server"),
        "ranger" => level_icon("RANGER_LEVEL", "\u{f00b}")
            .is_some()
            .then_some("ranger"),
        "yazi" => level_icon("YAZI_LEVEL", "\u{f00b}")
            .is_some()
            .then_some("yazi"),
        "nnn" => level_icon("NNNLVL", "nnn").is_some().then_some("nnn"),
        "lf" => level_icon("LF_LEVEL", "lf").is_some().then_some("lf"),
        "nix_shell" => in_nix_shell().then_some("nix"),
        "xplr" => env_var("XPLR_PID").is_some().then_some("xplr"),
        "midnight_commander" => env_var("MC_TMPDIR").is_some().then_some("mc"),
        "vim_shell" => env_var("VIMRUNTIME").is_some().then_some("vim"),
        "direnv" => env_var("DIRENV_DIR").is_some().then_some("direnv"),
        "chezmoi_shell" => env_var("CHEZMOI").is_some().then_some("chezmoi"),
        _ => None,
    }
}

/// Resolves an icon name: top-level `icon{}` overrides (auto-detected category via all plus
/// exact nf/compat/ascii tiers) → otherwise the built-in default table (per mode).
fn icon_str(config: &Config, key: &str) -> Option<String> {
    if let Some(ov) = config.icon_overrides.get(key) {
        let exact = match config.mode {
            crate::config::IconMode::NerdfontComplete
            | crate::config::IconMode::NerdfontFontconfig => ov.nf.as_ref(),
            crate::config::IconMode::Compatible => ov.compat.as_ref(),
            crate::config::IconMode::Ascii => ov.ascii.as_ref(),
        };
        if let Some(c) = exact {
            return Some(c.clone());
        }
        if let Some(a) = &ov.all
            && all_covers(a, &config.mode)
        {
            return Some(a.clone());
        }
    }
    let e = icon_default(key)?;
    Some(icon_by_mode(&e, &config.mode))
}

/// Auto-detection for the `all` field: NF Private Use chars land in the nf tier only,
/// standard ones in nf+compat, plain ASCII in all three.
fn all_covers(s: &str, mode: &crate::config::IconMode) -> bool {
    match mode {
        crate::config::IconMode::NerdfontComplete | crate::config::IconMode::NerdfontFontconfig => {
            true
        }
        crate::config::IconMode::Compatible => {
            s.chars().all(|c| !(0xE000..=0xF8FF).contains(&(c as u32)))
        }
        crate::config::IconMode::Ascii => s.is_ascii(),
    }
}

/// Default icon resolution for segment rendering: os is dynamic, vcs remote takes priority,
/// the rest go segment → icon name → table lookup.
fn resolve_icon(config: &Config, name: &str, vcs: Option<&GitStatus>, cwd: &str) -> Option<String> {
    if name == "os" || name == "os_icon" {
        return Some(os_icon(&config.mode));
    }
    if name == "vcs" {
        if let Some(v) = vcs {
            for (domain, ic) in &config.vcs_remote_icons {
                if !domain.is_empty() && v.remote_url.contains(domain) {
                    return Some(ic.clone());
                }
            }
        }
        return icon_str(config, "git");
    }
    // p10k `DIR_SHOW_WRITABLE`: when not writable (under v3 also non-existent) the dir icon
    // becomes a lock.
    if name == "dir" {
        if dir_writable_state(config.segment("dir"), cwd).is_some() {
            return icon_str(config, "lock");
        }
        // p10k `DIR_CLASSES`: the matched rule's icon; an empty icon in p10k means no icon at
        // all, not a fallback to the default folder icon (measured: with HOME's icon='', p10k
        // draws nothing).
        if let Some(c) = dir_class_match(config, cwd) {
            return if c.icon.is_empty() {
                None
            } else {
                Some(c.icon.clone())
            };
        }
    }
    let key = segment_icon_key(name)?;
    icon_str(config, key)
}

/// Matches cwd against `dir-classes` in order and returns the first hit (p10k `DIR_CLASSES`).
fn dir_class_match<'a>(config: &'a Config, cwd: &str) -> Option<&'a crate::config::DirClass> {
    let home = std::env::var("HOME").unwrap_or_default();
    config
        .dir_classes
        .iter()
        .find(|c| glob_match_path(&c.pattern, cwd, &home))
}

/// p10k's `DIR_CLASSES` matches `$PWD` with zsh extended globs; p11k uses a simplified glob:
/// `~` expands to $HOME, `*` / `?` / `[…]` do not cross `/`, `**` crosses directories, and
/// everything else is literal. A pattern ending in `/` (`~/work/`) also matches all subdirs.
fn glob_match_path(pattern: &str, path: &str, home: &str) -> bool {
    let pat = match pattern.strip_prefix('~') {
        Some(rest) => format!("{home}{rest}"),
        None => pattern.to_string(),
    };
    if pat.ends_with('/') {
        return glob_match(&format!("{pat}**"), path);
    }
    glob_match(&pat, path)
}

/// Simplified glob: `**` crosses `/`, `*` / `?` / `[…]` do not.
fn glob_match(pat: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => (0..=t.len()).any(|i| go(&p[2..], &t[i..])),
            Some('*') => (0..=t.len())
                .take_while(|i| *i == 0 || t[i - 1] != '/')
                .any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && t[0] != '/' && go(&p[1..], &t[1..]),
            Some('[') => {
                let Some(close) = p.iter().position(|c| *c == ']') else {
                    return !t.is_empty() && t[0] == '[' && go(&p[1..], &t[1..]);
                };
                if t.is_empty() || t[0] == '/' {
                    return false;
                }
                let set: Vec<char> = p[1..close].to_vec();
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        if t[0] >= set[i] && t[0] <= set[i + 2] {
                            hit = true;
                        }
                        i += 3;
                    } else {
                        if set[i] == t[0] {
                            hit = true;
                        }
                        i += 1;
                    }
                }
                hit && go(&p[close + 1..], &t[1..])
            }
            Some(c) => !t.is_empty() && t[0] == *c && go(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// Whether a repo whose workdir is `workdir` must be ignored (p10k
/// `VCS_DISABLED_WORKDIR_PATTERN`). p10k tests `[[ $VCS_STATUS_WORKDIR == $~pattern ]]`, so
/// the pattern is a zsh pattern and an unset one disables nothing.
pub(crate) fn workdir_is_disabled(pattern: &str, workdir: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    workdir_matches(pattern, workdir, &home)
}

/// `workdir_is_disabled` with an explicit home directory.
fn workdir_matches(pattern: &str, workdir: &str, home: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    pattern_alternatives(pattern)
        .iter()
        .any(|p| glob_match_path(p, workdir, home))
}

/// Rewrites a zsh pattern as the equivalent set of globs, expanding top-level `|` and every
/// `(...)` group: `~(|/foo)|/bar/baz/*` becomes `~`, `~/foo` and `/bar/baz/*`.
fn pattern_alternatives(pat: &str) -> Vec<String> {
    split_top_level(pat, '|')
        .into_iter()
        .flat_map(|branch| expand_groups(&branch))
        .collect()
}

/// Splits on `delim` at the top level, ignoring `|` inside `(...)` and `[...]`.
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut bracket = false;
    for c in s.chars() {
        match c {
            '[' if !bracket => bracket = true,
            ']' if bracket => bracket = false,
            '(' if !bracket => depth += 1,
            ')' if !bracket && depth > 0 => depth -= 1,
            _ => {}
        }
        if c == delim && depth == 0 && !bracket {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Expands the first top-level `(...)` group, then any group that follows it. An unbalanced
/// `(` is left alone, like a literal.
fn expand_groups(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0usize;
    let mut bracket = false;
    let mut open = 0usize;
    for (i, c) in chars.iter().enumerate() {
        match *c {
            '[' if !bracket => bracket = true,
            ']' if bracket => bracket = false,
            '(' if !bracket => {
                if depth == 0 {
                    open = i;
                }
                depth += 1;
            }
            ')' if !bracket && depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let prefix: String = chars[..open].iter().collect();
                    let inner: String = chars[open + 1..i].iter().collect();
                    let suffix: String = chars[i + 1..].iter().collect();
                    return pattern_alternatives(&inner)
                        .into_iter()
                        .flat_map(|alt| expand_groups(&format!("{prefix}{alt}{suffix}")))
                        .collect();
                }
            }
            _ => {}
        }
    }
    vec![s.to_string()]
}

/// p10k `DIR_SHOW_WRITABLE` values: `#true`=1, `"v2"`=2, `"v3"`=3 (as in p10k only these are
/// valid; p11k additionally accepts an integer). Returns 0 for no check.
fn dir_show_writable(seg: &crate::config::Segment) -> i64 {
    match seg.prop("show-writable") {
        Some(crate::config::Prop::Bool(true)) => 1,
        Some(crate::config::Prop::Int(n)) => (*n).clamp(0, 3),
        Some(crate::config::Prop::Str(s)) => match s.as_str() {
            "true" => 1,
            "v2" => 2,
            "v3" => 3,
            _ => 0,
        },
        _ => 0,
    }
}

/// Returns the matching state name when the directory is not writable (or missing under v3),
/// otherwise None. p10k tests `[[ -w $PWD ]]`; access(2) matches that here.
fn dir_writable_state(seg: &crate::config::Segment, cwd: &str) -> Option<&'static str> {
    let mode = dir_show_writable(seg);
    if mode <= 0 {
        return None;
    }
    let bytes = std::ffi::CString::new(cwd).ok()?;
    let writable = unsafe { libc::access(bytes.as_ptr(), libc::W_OK) } == 0;
    if writable {
        return None;
    }
    if mode > 2 && !std::path::Path::new(cwd).exists() {
        Some("NON_EXISTENT")
    } else {
        Some("NOT_WRITABLE")
    }
}

/// Segments whose icon is the content: the icon is drawn even with empty text (env
/// indicators, os badges). For every other segment the icon is decoration, and empty text
/// hides the whole segment.
fn icon_is_content(name: &str) -> bool {
    matches!(
        name,
        "ssh"
            | "xplr"
            | "midnight_commander"
            | "vim_shell"
            | "direnv"
            | "chezmoi_shell"
            | "os"
            | "os_icon"
    )
}

thread_local! {
    static OS_ICON: RefCell<Option<IconEntry>> = const { RefCell::new(None) };
}

/// os icon: uname family + /etc/os-release ID to match the distro (matching p10k
/// `_p9k_set_os`), returned per font tier by mode.
fn os_icon(mode: &crate::config::IconMode) -> String {
    OS_ICON.with(|c| {
        let e = *c.borrow_mut().get_or_insert_with(detect_os_icon);
        icon_by_mode(&e, mode)
    })
}

#[derive(Clone)]
struct EnvInfo {
    user: String,
    host: String,
    ssh: bool,
    root: bool,
}

thread_local! {
    static ENV: RefCell<Option<EnvInfo>> = const { RefCell::new(None) };
}

fn env() -> EnvInfo {
    ENV.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(detect_env());
        }
        c.borrow().as_ref().unwrap().clone()
    })
}

fn detect_env() -> EnvInfo {
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok())
        .unwrap_or_default();
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("HOST").ok())
        .unwrap_or_default();
    let ssh = ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    let root = unsafe { libc::geteuid() } == 0;
    EnvInfo {
        user,
        host,
        ssh,
        root,
    }
}

fn user_name() -> String {
    env().user
}

/// context: SSH → `user@host`; local root → `user`; local regular user → empty (hidden).
fn context_text() -> String {
    let e = env();
    if e.ssh {
        format!("{}@{}", e.user, e.host)
    } else if e.root {
        e.user
    } else {
        String::new()
    }
}

fn host_text() -> String {
    let e = env();
    if e.ssh || e.root {
        e.host
    } else {
        String::new()
    }
}

fn root_indicator_text() -> String {
    if env().root {
        "#".into()
    } else {
        String::new()
    }
}

/// Reads an env var; missing or empty → None.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Level segments (ranger/nnf/lf/yazi): value 0 or empty → empty (not inside that program).
fn level_text(name: &str) -> String {
    env_var(name).filter(|v| v != "0").unwrap_or_default()
}

fn level_icon(name: &str, icon: &str) -> Option<String> {
    env_var(name).filter(|v| v != "0").map(|_| icon.into())
}

fn env_has_proxy() -> bool {
    ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
        .any(|k| env_var(k).is_some())
}

/// proxy segment: host:port of the first non-empty proxy (scheme/user@/path stripped).
fn proxy_text() -> String {
    for key in ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
    {
        let Some(v) = env_var(key) else { continue };
        let v = v.split_once("://").map(|(_, rest)| rest).unwrap_or(&v);
        let v = v.rsplit('@').next().unwrap_or(v);
        let v = v.split(['/', '?']).next().unwrap_or(v);
        if !v.is_empty() {
            return v.to_string();
        }
    }
    String::new()
}

/// openfoam segment: p10k shows `OF: <version>`.
fn openfoam_text() -> String {
    env_var("WM_PROJECT_VERSION")
        .map(|v| format!("OF: {v}"))
        .unwrap_or_default()
}

// nix_shell: as in p10k only pure/impure in `IN_NIX_SHELL` count; any other value is inactive.
fn nix_shell_text() -> String {
    env_var("IN_NIX_SHELL")
        .filter(|v| v == "pure" || v == "impure")
        .unwrap_or_default()
}

fn in_nix_shell() -> bool {
    env_var("IN_NIX_SHELL").is_some_and(|v| v == "pure" || v == "impure")
}

// System resource segments: read /proc and /sys (battery), or run `df`; hidden when the data
// source is missing.
fn human_bytes(bytes: u64) -> String {
    let mut val = bytes as f64;
    let mut i = 0;
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    while val >= 1024.0 && i < UNITS.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{val}B")
    } else {
        format!("{val:.1}{}", UNITS[i])
    }
}

/// Reads one field of /proc/meminfo (KiB).
fn meminfo_kb(name: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(&format!("{name}:"))
                .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        })
}

/// Load: current system load (the first value of /proc/loadavg).
fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
        .unwrap_or_default()
}

/// Free memory (MemAvailable, human readable).
fn ram() -> String {
    meminfo_kb("MemAvailable")
        .map(|kb| human_bytes(kb * 1024))
        .unwrap_or_default()
}

/// Used swap.
fn swap() -> String {
    let total = meminfo_kb("SwapTotal").unwrap_or(0);
    let free = meminfo_kb("SwapFree").unwrap_or(0);
    let used = total.saturating_sub(free);
    if used == 0 {
        String::new()
    } else {
        human_bytes(used * 1024)
    }
}

/// Used percentage of the partition holding the current directory.
fn disk_usage(cwd: &str) -> String {
    run_cmd("df", &["-P", cwd])
        .and_then(|s| {
            s.lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(4))
                .map(|v| v.trim_end_matches('%').to_string())
        })
        .unwrap_or_default()
}

/// Battery: percentage + status (e.g. `Charging 87%`); hidden when no battery file exists.
fn battery() -> String {
    let bat = std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .find(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        });
    let Some(bat) = bat else { return String::new() };
    let bat = bat.path();
    let cap = std::fs::read_to_string(bat.join("capacity"))
        .ok()
        .map(|s| s.trim().to_string());
    let status = std::fs::read_to_string(bat.join("status"))
        .ok()
        .map(|s| s.trim().to_string());
    match (cap, status) {
        (Some(c), Some(s)) => format!("{s} {c}%"),
        (Some(c), None) => format!("{c}%"),
        _ => String::new(),
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn current_dir() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// AWS segment: current profile (env `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`).
fn aws_text() -> String {
    env_var("AWS_PROFILE")
        .or_else(|| env_var("AWS_DEFAULT_PROFILE"))
        .unwrap_or_default()
}

/// gcloud segment: current configuration name (contents of the active_config file).
fn gcloud_text() -> String {
    let dir = env_var("CLOUDSDK_CONFIG").unwrap_or_else(|| format!("{}/.config/gcloud", home()));
    std::fs::read_to_string(format!("{dir}/active_config"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// azure segment: default subscription name (the name with `isDefault: true` in
/// azureProfile.json).
fn azure_text() -> String {
    let dir = env_var("AZURE_CONFIG_DIR").unwrap_or_else(|| format!("{}/.azure", home()));
    let Some(s) = std::fs::read_to_string(format!("{dir}/azureProfile.json")).ok() else {
        return String::new();
    };
    let mut rest = s.as_str();
    while let Some(pos) = rest.find("\"isDefault\":") {
        // Whitespace is allowed after `isDefault`, then check for true.
        if rest[pos + "\"isDefault\":".len()..]
            .trim_start()
            .starts_with("true")
        {
            let before = &rest[..pos];
            if let Some(n) = before.rfind("\"name\"") {
                let after = &before[n + "\"name\"".len()..];
                if let Some(v) = after.split('"').nth(1) {
                    return v.to_string();
                }
            }
        }
        rest = &rest[pos + 1..];
    }
    String::new()
}

/// kubecontext segment: current context (from `$KUBECONFIG` or `~/.kube/config`).
fn kubecontext_text(_cwd: &str) -> String {
    let path = env_var("KUBECONFIG").unwrap_or_else(|| format!("{}/.kube/config", home()));
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("current-context:")
                    .map(|v| v.trim().to_string())
            })
        })
        .unwrap_or_default()
}

/// terraform segment: current workspace (from `$TF_DATA_DIR`/`.terraform/environment`).
fn terraform_text(cwd: &str) -> String {
    let dir = env_var("TF_DATA_DIR").unwrap_or_else(|| ".terraform".into());
    std::fs::read_to_string(std::path::Path::new(cwd).join(dir).join("environment"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// First non-loopback local IPv4 (`ip -4 addr show`, skipping 127.0.0.1).
fn ip_text() -> String {
    run_cmd("ip", &["-4", "addr", "show"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("inet ") && !l.contains("127.0.0.1"))
        .find_map(|l| l.split("inet ").nth(1)?.split('/').next())
        .unwrap_or_default()
        .to_string()
}

/// VPN interface IP (the IPv4 of tailscale/wg/tun/zt).
fn vpn_ip_text() -> String {
    let out = run_cmd("ip", &["addr", "show"]).unwrap_or_default();
    let mut ifname = String::new();
    for line in out.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("inet ") {
            if is_vpn_if(&ifname) {
                let ip = rest.split('/').next().unwrap_or("");
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        } else if let Some(name) = line.split(": ").nth(1).and_then(|s| s.split(':').next()) {
            ifname = name.to_string();
        }
    }
    String::new()
}

fn is_vpn_if(name: &str) -> bool {
    ["tailscale", "wg", "tun", "zt"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// WiFi: interface name + link quality from /proc/net/wireless.
fn wifi_text() -> String {
    std::fs::read_to_string("/proc/net/wireless")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.contains(':')) // data line (`iface: ...`)
                .map(|l| l.to_string())
        })
        .and_then(|l| {
            let mut p = l.split_whitespace();
            let ifn = p.next()?.trim_end_matches(':').to_string();
            let _ = p.next(); // flags
            let q = p.next()?; // link quality
            Some(format!("{ifn} {}", q.trim_end_matches('.')))
        })
        .unwrap_or_default()
}

/// Public IP: curl query (cached by run_cmd; synchronous on the first call, cached afterwards).
fn public_ip_text() -> String {
    run_cmd("curl", &["-s", "--max-time", "4", "https://v4.ident.me/"]).unwrap_or_default()
}

fn detect_virt() -> String {
    run_cmd("systemd-detect-virt", &[])
        .filter(|v| v != "none")
        .unwrap_or_default()
}

fn toolbox_text() -> String {
    env_var("P9K_TOOLBOX_NAME")
        .or_else(|| {
            std::fs::read_to_string("/run/.containerenv")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("name=").map(|v| v.trim().to_string()))
                })
        })
        .unwrap_or_default()
}

fn dir_writable_text(cwd: &str) -> String {
    use std::os::unix::fs::MetadataExt;
    let writable = std::fs::metadata(cwd)
        .map(|m| m.mode() & 0o222 != 0)
        .unwrap_or(true);
    if writable { String::new() } else { "!".into() }
}

fn per_directory_history_text() -> String {
    match env_var("PER_DIRECTORY_HISTORY_TOGGLE").as_deref() {
        Some("global") => "global".into(),
        Some(_) => "local".into(),
        None => String::new(),
    }
}

fn haskell_stack_text() -> String {
    run_cmd("stack", &["--version"])
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .map(|v| v.trim_end_matches(',').to_string())
        })
        .unwrap_or_default()
}

/// Finds `filename` in the cwd's ancestors and returns its full contents.
fn find_up_content(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            return Some(c);
        }
        dir = dir.parent()?;
    }
}

/// package segment: `name`/`version` from package.json.
fn package_text(cwd: &str) -> String {
    let Some(content) = find_up_content(cwd, "package.json") else {
        return String::new();
    };
    let name = json_str_field(&content, "\"name\":");
    let version = json_str_field(&content, "\"version\":");
    match (name, version) {
        (Some(n), Some(v)) => format!("{n}@{}", v.trim_start_matches('v')),
        (Some(n), None) => n,
        _ => String::new(),
    }
}

/// Extracts the value of a `"key": "value"` field from a JSON string.
fn json_str_field(content: &str, key: &str) -> Option<String> {
    let pos = content.find(key)?;
    let rest = &content[pos + key.len()..];
    let rest = rest
        .trim_start()
        .strip_prefix(':')
        .unwrap_or(rest)
        .trim_start();
    Some(rest.strip_prefix('"')?.split('"').next()?.to_string())
}

/// vi_mode segment: current zsh edit mode → name; empty when not zsh (unreported).
fn vi_mode_text(seg: &Segment) -> String {
    // The text is configurable (p10k `VI_INSERT/COMMAND/VISUAL/OVERWRITE_MODE_STRING`).
    let sym = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    let mode = CURRENT_VI_MODE
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    match mode.as_str() {
        "vicmd" => sym("normal", "NORMAL"),
        "viins" => sym("insert", "INSERT"),
        "vis" | "viopp" => sym("visual", "VISUAL"),
        "viowr" => sym("overwrite", "OVERWRITE"),
        "" | "main" => String::new(), // unreported or emacs (main): hidden
        other => other.to_string(),
    }
}

/// asdf segment: first plugin version line of .tool-versions (simplified, first line only).
fn asdf_text(cwd: &str) -> String {
    find_up_version(cwd, ".tool-versions").unwrap_or_default()
}

/// fvm segment: Flutter version detected from .fvm/flutter_sdk (or its parent dir).
fn fvm_text(cwd: &str) -> String {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let sdk = dir.join(".fvm").join("flutter_sdk");
        if sdk.exists() {
            // Derived from the version path (e.g. …/versions/3.22.0).
            let real = std::fs::canonicalize(&sdk).unwrap_or(sdk);
            if let Some(v) = real.to_string_lossy().split("/versions/").nth(1) {
                return v.split('/').next().unwrap_or("").to_string();
            }
            return "fvm".into();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    String::new()
}

/// Finds the cwd ancestor directory containing `filename`.
fn find_up_dir(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        if dir.join(filename).exists() {
            return Some(dir.to_string_lossy().into_owned());
        }
        dir = dir.parent()?;
    }
}

/// project_id of the GCP service account JSON.
fn google_app_cred_text() -> String {
    let Some(path) = env_var("GOOGLE_APPLICATION_CREDENTIALS") else {
        return String::new();
    };
    let Some(content) = std::fs::read_to_string(&path).ok() else {
        return String::new();
    };
    json_str_field(&content, "\"project_id\"").unwrap_or_default()
}

/// Elastic Beanstalk environment name (the `* `-prefixed current line of eb list).
fn aws_eb_env_text() -> String {
    run_cmd("eb", &["list"])
        .unwrap_or_default()
        .lines()
        .find(|l| l.trim_start().starts_with("* "))
        .map(|l| l.trim_start_matches("* ").trim().to_string())
        .unwrap_or_default()
}

/// Laravel version: runs `php artisan --version` when an ancestor contains artisan.
fn laravel_version_text(cwd: &str) -> String {
    let Some(dir) = find_up_dir(cwd, "artisan") else {
        return String::new();
    };
    run_cmd("php", &[&format!("{dir}/artisan"), "--version"])
        .and_then(|s| s.split_whitespace().nth(2).map(|v| v.to_string()))
        .unwrap_or_default()
}

fn count_rs(dir: &std::path::Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_rs(&p);
            } else if p.extension().map(|x| x == "rb").unwrap_or(false) {
                n += 1;
            }
        }
    }
    n
}

/// RSpec coverage: ratio of .rb files in app to those in spec.
fn rspec_stats_text(cwd: &str) -> String {
    let app = count_rs(&std::path::Path::new(cwd).join("app"));
    let spec = count_rs(&std::path::Path::new(cwd).join("spec"));
    let total = app + spec;
    if total == 0 {
        return String::new();
    }
    format!("RSpec: {}%", app * 100 / total)
}

fn todo_text() -> String {
    run_cmd("todo.sh", &["-p", "ls"])
        .and_then(|s| s.lines().last().map(|l| l.to_string()))
        .unwrap_or_default()
}

fn taskwarrior_text() -> String {
    run_cmd("task", &["+PENDING", "count"]).unwrap_or_default()
}

fn dropbox_text() -> String {
    run_cmd("dropbox-cli", &["filestatus", "."])
        .and_then(|s| s.lines().next().map(|l| l.to_string()))
        .unwrap_or_default()
}

/// date segment: formats the current date per `date-format` (strftime), defaulting to p10k's
/// `%d.%m.%y`.
fn date_text(seg: &crate::config::Segment) -> String {
    let fmt = match seg.prop("date-format") {
        Some(crate::config::Prop::Str(s)) => s.as_str(),
        _ => "%d.%m.%y",
    };
    strftime_now(fmt)
}

fn strftime_now(fmt: &str) -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let cfmt = match std::ffi::CString::new(fmt) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let mut buf = [0u8; 128];
    let n = unsafe {
        libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            cfmt.as_ptr(),
            &tm,
        )
    };
    if n == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

// Toolchain version segments: run `cmd --version`, cache the output and parse the version;
// shown only when the command exists.
thread_local! {
    static CMD_CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    let key = format!("{cmd} {}", args.join(" "));
    CMD_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(&key) {
            return v.clone();
        }
        let out = std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                // Some commands (e.g. java) write the version to stderr; fall back when stdout
                // is empty.
                let s = String::from_utf8_lossy(&o.stdout);
                if s.trim().is_empty() {
                    String::from_utf8_lossy(&o.stderr).trim().to_string()
                } else {
                    s.trim().to_string()
                }
            });
        c.borrow_mut().insert(key.clone(), out.clone());
        out
    })
}

fn go_version() -> String {
    // "go version go1.27.0-X:nodwarf5 linux/amd64" → "go1.27.0"
    run_cmd("go", &["version"])
        .and_then(|s| {
            Some(
                s.split_whitespace()
                    .nth(2)?
                    .split('-')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

fn rust_version() -> String {
    // "rustc 1.97.1 (…)" → "1.97.1"
    run_cmd("rustc", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

fn node_version() -> String {
    // "v26.8.1" → "26.8.1"
    run_cmd("node", &["--version"])
        .map(|s| s.trim_start_matches('v').to_string())
        .unwrap_or_default()
}

fn php_version() -> String {
    // "PHP 8.2.0 (cli)" → "8.2.0"
    run_cmd("php", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

fn java_version() -> String {
    // '"17.0.9" ...' → "17.0.9" (cut at the first `-`)
    run_cmd("java", &["-fullversion"])
        .and_then(|s| {
            let s = s.split('"').nth(1)?;
            Some(s.split('-').next().unwrap_or(s).to_string())
        })
        .unwrap_or_default()
}

fn dotnet_version() -> String {
    run_cmd("dotnet", &["--version"]).unwrap_or_default()
}

fn swift_version() -> String {
    // "Apple Swift version 5.9 (…)" → take the first word starting with a digit
    run_cmd("swift", &["--version"])
        .and_then(|s| {
            Some(
                s.split_whitespace()
                    .find(|w| {
                        w.chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                    })?
                    .trim_matches('.')
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

fn terraform_version() -> String {
    // "Terraform v1.15.9\non linux_amd64…" → "1.15.9" (first line only)
    run_cmd("terraform", &["--version"])
        .and_then(|s| {
            Some(
                s.lines()
                    .next()?
                    .trim_start_matches("Terraform v")
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

fn cpu_arch() -> String {
    std::fs::read_to_string("/proc/sys/kernel/arch")
        .ok()
        .or_else(|| run_cmd("uname", &["-m"]))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// Environment manager and *env family segments: current version from an env var or an
// ancestor `.X-version` file; shown only when active.

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// Finds `filename` in cwd ancestors and returns its first non-empty line (version file).
fn find_up_version(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            let first = c.lines().next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
        dir = dir.parent()?;
    }
}

fn env_or_file(env: &str, file: &str, cwd: &str) -> Option<String> {
    env_var(env).or_else(|| find_up_version(cwd, file))
}

fn virtualenv_text(_cwd: &str) -> String {
    env_var("VIRTUAL_ENV")
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

fn anaconda_text(_cwd: &str) -> String {
    env_var("CONDA_PREFIX")
        .or_else(|| env_var("CONDA_ENV_PATH"))
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

/// p10k `prompt_symfony2_version`: reads the `app/bootstrap.php.cache` line containing
/// ` VERSION ` and keeps digits and dots only.
fn symfony2_version_text(cwd: &str) -> String {
    let path = std::path::Path::new(cwd).join("app/bootstrap.php.cache");
    let Ok(src) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    for line in src.lines() {
        if !line.contains(" VERSION ") {
            continue;
        }
        return line
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
    }
    String::new()
}

/// p10k `prompt_symfony2_tests`: when `src`, `app` and `app/AppKernel.php` exist, computes
/// the share of `src/**/*.php` paths containing `Tests` (p10k's SF2 test ratio).
/// Returns the ratio as text with two decimals, shaped like `SF2: 12.34%`.
fn symfony2_tests_text(cwd: &str) -> String {
    let root = std::path::Path::new(cwd);
    if !root.join("src").is_dir()
        || !root.join("app").is_dir()
        || !root.join("app/AppKernel.php").is_file()
    {
        return String::new();
    }
    let mut all = 0usize;
    let mut tests = 0usize;
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "php") {
                all += 1;
                if path.to_string_lossy().contains("Tests") {
                    tests += 1;
                }
            }
        }
    }
    if all == 0 {
        return String::new();
    }
    let code = all - tests;
    if code == 0 {
        return String::new();
    }
    let ratio = 100.0 * tests as f64 / code as f64;
    format!("SF2: {ratio:.2}%")
}

fn pyenv_text(cwd: &str) -> String {
    env_or_file("PYENV_VERSION", ".python-version", cwd)
        .map(|v| v.trim_start_matches("python-").to_string())
        .unwrap_or_default()
}

fn nodeenv_text(_cwd: &str) -> String {
    env_var("NODE_VIRTUAL_ENV")
        .map(|v| format!("[{}]", basename(&v)))
        .unwrap_or_default()
}

fn nodenv_text(cwd: &str) -> String {
    env_or_file("NODENV_VERSION", ".node-version", cwd).unwrap_or_default()
}

fn nvm_text(_cwd: &str) -> String {
    // Simplified: an existing NVM_DIR is enough to show the current node version.
    env_var("NVM_DIR")
        .map(|_| node_version())
        .unwrap_or_default()
}

fn rbenv_text(cwd: &str) -> String {
    env_or_file("RBENV_VERSION", ".ruby-version", cwd)
        .map(|v| v.trim_start_matches("ruby-").to_string())
        .unwrap_or_default()
}

fn chruby_text(_cwd: &str) -> String {
    env_var("RUBY_ENGINE").unwrap_or_default()
}

fn rvm_text(_cwd: &str) -> String {
    env_var("GEM_HOME")
        .filter(|g| g.contains("rvm"))
        .map(|g| basename(&g))
        .unwrap_or_default()
}

fn goenv_text(cwd: &str) -> String {
    env_or_file("GOENV_VERSION", ".go-version", cwd)
        .map(|v| v.trim_start_matches("go-").to_string())
        .unwrap_or_default()
}

fn jenv_text(cwd: &str) -> String {
    env_or_file("JENV_VERSION", ".java-version", cwd).unwrap_or_default()
}

fn phpenv_text(cwd: &str) -> String {
    env_or_file("PHPENV_VERSION", ".php-version", cwd).unwrap_or_default()
}

fn luaenv_text(cwd: &str) -> String {
    env_or_file("LUAENV_VERSION", ".lua-version", cwd).unwrap_or_default()
}

fn plenv_text(cwd: &str) -> String {
    env_or_file("PLENV_VERSION", ".perl-version", cwd).unwrap_or_default()
}

fn scalaenv_text(cwd: &str) -> String {
    env_or_file("SCALAENV_VERSION", ".scala-version", cwd).unwrap_or_default()
}

fn perlbrew_text(_cwd: &str) -> String {
    env_var("PERLBREW_PERL")
        .map(|v| v.trim_start_matches("perl-").to_string())
        .unwrap_or_default()
}

fn detect_os_icon() -> IconEntry {
    let uname = std::env::consts::OS;
    if uname != "linux" {
        return match uname {
            "macos" => icon("\u{f179}", "OSX", "mac"),   // 
            "windows" => icon("\u{f17a}", "WIN", "win"), // 
            _ => icon("\u{f17c}", "Lx", "linux"),        // default linux icon
        };
    }
    // Linux: read the ID from /etc/os-release (substring match, matching p10k case *arch*
    // etc.).
    let id = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("ID=")).map(|l| {
                l.trim_start_matches("ID=")
                    .trim()
                    .trim_matches('"')
                    .to_string()
            })
        })
        .unwrap_or_default();
    // The three-tier text abbreviations match p10k's compatible/ascii branches.
    if id.contains("arch") {
        icon("\u{f303}", "Arc", "arch")
    } else if id.contains("ubuntu") {
        icon("\u{f31b}", "Ubu", "ubuntu")
    } else if id.contains("debian") {
        icon("\u{f306}", "Deb", "debian")
    } else if id.contains("fedora") {
        icon("\u{f30a}", "Fed", "fedora")
    } else if id.contains("gentoo") {
        icon("\u{f30d}", "Gen", "gentoo")
    } else if id.contains("nixos") {
        icon("\u{f313}", "Nix", "nixos")
    } else if id.contains("manjaro") {
        icon("\u{f312}", "Man", "manjaro")
    } else if id.contains("mint") {
        icon("\u{f30e}", "LMi", "mint")
    } else if id.contains("alpine") {
        icon("\u{f300}", "Alp", "alpine")
    } else if id.contains("void") {
        icon("\u{f32e}", "Vo", "void")
    } else if id.contains("artix") {
        icon("\u{f31f}", "Art", "artix")
    } else if id.contains("opensuse") || id.contains("suse") {
        icon("\u{f314}", "OSu", "suse")
    } else {
        icon("\u{f17c}", "Lx", "linux")
    }
}

/// `time` segment text: reads the `time-format` prop; `"12h"` → `HH:MM:SS AM/PM`, 24h by
/// default.
fn time_text(seg: &crate::config::Segment) -> String {
    match seg.prop("time-format") {
        Some(crate::config::Prop::Str(s)) if s == "12h" => now_hhmmss_12h(),
        _ => now_hhmmss(),
    }
}

/// Current time HH:MM:SS (libc localtime).
fn now_hhmmss() -> String {
    let tm = local_time();
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// 12-hour `HH:MM:SS AM/PM` (matching p10k `%I:%M:%S %p`, hours 01–12).
fn now_hhmmss_12h() -> String {
    let tm = local_time();
    let hour12 = tm.tm_hour % 12;
    let hour12 = if hour12 == 0 { 12 } else { hour12 };
    let ampm = if tm.tm_hour < 12 { "AM" } else { "PM" };
    format!("{hour12:02}:{:02}:{:02} {ampm}", tm.tm_min, tm.tm_sec)
}

fn local_time() -> libc::tm {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    tm
}

/// Command duration formatting (matching p10k `prompt_command_execution_time`):
/// - `<60s`: `precision` 0 → rounded integer `Ns`; >0 → `N.NNs` (p10k default 2).
/// - `>=60s`: `Xs` / `Xm Ys` / `Xh Ym Zs` / `Xd Xh Ym Zs` (**with spaces**, as in p10k);
///   with `format` set to `H:M:S` instead `MM:SS` / `0H:MM:SS` / `H:MM:SS`.
fn format_duration(secs: f64, precision: usize, hms: bool) -> String {
    if secs < 60.0 {
        if precision == 0 {
            format!("{}s", (secs + 0.5) as u64)
        } else {
            format!("{:.*}s", precision, secs)
        }
    } else {
        let d = (secs + 0.5) as u64;
        if hms {
            let (h, m, s) = (d / 3600, d / 60 % 60, d % 60);
            if d >= 36000 {
                format!("{h}:{m:02}:{s:02}")
            } else if d >= 3600 {
                format!("0{h}:{m:02}:{s:02}")
            } else {
                format!("{m:02}:{s:02}")
            }
        } else {
            let mut text = format!("{}s", d % 60);
            if d >= 60 {
                text = format!("{}m {text}", d / 60 % 60);
                if d >= 3600 {
                    text = format!("{}h {text}", d / 3600 % 24);
                    if d >= 86400 {
                        text = format!("{}d {text}", d / 86400);
                    }
                }
            }
            text
        }
    }
}

/// `dir` segment text: shortening (truncate_to_unique) plus per-component coloring by class.
/// - Anchor (`~`/current dir/marker ancestor): `ANCHOR` state
/// - Shortened: `SHORTENED` state
/// - Normal: segment default; the `/` separator keeps its own color (not the component's).
fn dir_seg_text(
    config: &Config,
    info: &HeaderInfo,
    seg: &crate::config::Segment,
    default: &Style,
    dir_budget: Option<usize>,
) -> String {
    let shorten = shorten_len(seg);
    let cwd = std::path::Path::new(&info.cwd);
    let home = std::env::var("HOME").ok();
    let home = home.as_deref().map(std::path::Path::new);
    let bool_prop = |name: &str| matches!(seg.prop(name), Some(crate::config::Prop::Bool(true)));
    // p10k `DIR_PATH_ABSOLUTE`: ignore $HOME and show the absolute path; accordingly, home
    // cannot be used as a prefix when splitting, or the full path could not be reconstructed.
    let absolute = bool_prop("path-absolute");
    let parts = crate::dir_shorten::shorten(
        cwd,
        if absolute { None } else { home },
        &dir_shorten_opts(seg, shorten, dir_budget),
    );
    let mut s = String::new();
    let is_home = !absolute && home.map(|h| cwd.starts_with(h)).unwrap_or(false);
    // p10k `DIR_OMIT_FIRST_CHARACTER`: drop the leading `/` of an absolute path (still `/`
    // when cwd=`/`).
    let omit_first = bool_prop("omit-first-character");
    // Home prefix abbreviation (p10k `HOME_FOLDER_ABBREVIATION`, default `~`).
    let abbrev = match seg.prop("home-abbreviation") {
        Some(crate::config::Prop::Str(a)) => a.clone(),
        _ => "~".to_string(),
    };
    // The path separator's own color (p10k `DIR_PATH_SEPARATOR_FOREGROUND`), segment style by
    // default.
    let sep_style = prop_style(seg, default, "path-separator-foreground");
    // p10k `DIR_SHOW_WRITABLE`: when not writable / non-existent the whole segment's state
    // becomes NOT_WRITABLE / NON_EXISTENT (users can color both via `state`).
    let writable_state = dir_writable_state(seg, &info.cwd);
    let is_root = info.cwd == "/";
    if is_root {
        // The root directory has no components; p10k shows a single `/` here.
        s.push_str(&paint("/", &sep_style));
    } else if is_home {
        let st = seg.effective_style(Some("ANCHOR"), &config.defaults);
        s.push_str(&paint(&abbrev, &st));
        if !parts.is_empty() {
            s.push_str(&paint("/", &sep_style));
        }
    } else if !parts.is_empty() && (!omit_first || is_root) {
        // Leading `/` of an absolute path.
        s.push_str(&paint("/", &sep_style));
    }
    // p10k `DIR_PATH_HIGHLIGHT_{FOREGROUND,BOLD}`: the last component (the current directory)
    // is colored on its own.
    let highlight_bold = bool_prop("path-highlight-bold");
    let last = parts.len().saturating_sub(1);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str(&paint("/", &sep_style)); // separator
        }
        let state = match part.class {
            crate::dir_shorten::Class::Anchor => Some("ANCHOR"),
            crate::dir_shorten::Class::Shortened => Some("SHORTENED"),
            crate::dir_shorten::Class::Normal => None,
        };
        // Not-writable/non-existent takes precedence over ANCHOR/SHORTENED (p10k also fixes
        // the state before picking a color); with `dir-classes` the matched class state name is
        // applied first and the not-writable suffix appended
        // (p10k: WORK → WORK_NOT_WRITABLE / WORK_NON_EXISTENT).
        let mut st = match writable_state {
            Some(s) => {
                let named = dir_class_match(config, &info.cwd)
                    .map(|c| format!("{}_{}", c.state, s))
                    .filter(|n| seg.states.contains_key(n))
                    .or_else(|| Some(s.to_string()));
                seg.effective_style(named.as_deref(), &config.defaults)
            }
            None => match dir_class_match(config, &info.cwd) {
                Some(c) if !c.state.is_empty() => {
                    seg.effective_style(Some(&c.state), &config.defaults)
                }
                _ => seg.effective_style(state, &config.defaults),
            },
        };
        if i == last {
            st = prop_style(seg, &st, "path-highlight-foreground");
            if highlight_bold {
                st.bold = true;
            }
        }
        s.push_str(&paint(&part.text, &st));
    }
    let s = if seg.content.is_some() {
        value_of(seg.content.as_deref(), s)
    } else {
        s
    };
    // p10k `DIR_HYPERLINK`: wrap the directory in an OSC 8 hyperlink (absolute paths only).
    if bool_prop("hyperlink") && info.cwd.starts_with('/') {
        format!(
            "\u{1b}]8;;file://{}\u{7}{}\u{1b}]8;;\u{7}",
            url_escape(&info.cwd),
            s
        )
    } else {
        s
    }
}

/// p10k `_p9k_url_escape`: everything outside `[a-zA-Z0-9"/:_.-!'()~]` is escaped as `%XX`.
fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'"' | b'/' | b':' | b'_' | b'.' | b'-' | b'!' | b'\'' | b'(' | b')' | b'~'
            );
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads the dir segment's `shorten-dir-length` (keep the last N levels, default 1 as in p10k).
fn shorten_len(seg: &crate::config::Segment) -> usize {
    if let Some(crate::config::Prop::Int(n)) = seg.prop("shorten-dir-length") {
        (*n).clamp(1, 20) as usize
    } else {
        1
    }
}

/// dir segment shortening options: `shorten-strategy` / `shorten-delimiter` /
/// `shorten-folder-marker` (matching p10k's `SHORTEN_*`).
fn dir_shorten_opts(
    seg: &crate::config::Segment,
    length: usize,
    budget: Option<usize>,
) -> crate::dir_shorten::Opts {
    let str_prop = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    crate::dir_shorten::Opts {
        strategy: crate::dir_shorten::Strategy::parse(&str_prop("shorten-strategy", "")),
        length,
        // The p10k engine's default ellipsis is `…`; `truncate_to_unique` does not emit it
        // (consistent with `SHORTEN_DELIMITER=` in p10k's default config).
        delimiter: str_prop("shorten-delimiter", "\u{2026}"),
        marker: str_prop("shorten-folder-marker", ""),
        budget,
    }
}

/// Expands env vars: `${VAR}` or `$VAR` → the value; undefined → empty string.
fn expand_env(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' && i + 1 < b.len() {
            if b[i + 1] == '{' {
                if let Some(rel) = b[i + 2..].iter().position(|&c| c == '}') {
                    let name: String = b[i + 2..i + 2 + rel].iter().collect();
                    out.push_str(&std::env::var(&name).unwrap_or_default());
                    i = i + 2 + rel + 1;
                    continue;
                }
            } else if b[i + 1].is_ascii_alphanumeric() || b[i + 1] == '_' {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                    j += 1;
                }
                let name: String = b[i + 1..j].iter().collect();
                out.push_str(&std::env::var(&name).unwrap_or_default());
                i = j;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// A foreground-color style prop on a segment → style (segment style by default).
fn prop_style(seg: &Segment, style: &Style, prop: &str) -> Style {
    match seg.prop(prop) {
        Some(crate::config::Prop::Int(n)) => Style {
            fg: Color::Xterm(i64::clamp(*n, 0, 255) as u8),
            ..style.clone()
        },
        _ => style.clone(),
    }
}

/// vcs segment text, colored piecewise as p10k does: branch name/branch icon/ahead/behind/
/// stash use `clean-foreground`, staged/unstaged counts use `modified-foreground`, the
/// conflict count uses `conflicted-foreground` (falling back to modified), untracked uses
/// `untracked-foreground` (falling back to the segment style). The returned string is colored.
fn vcs_text(
    vcs: Option<&GitStatus>,
    branch_icon: &str,
    commit_icon: &str,
    seg: &Segment,
    style: &Style,
) -> String {
    let Some(v) = vcs else { return String::new() };
    let clean = prop_style(seg, style, "clean-foreground");
    let modified = prop_style(seg, style, "modified-foreground");
    let conflicted = prop_style(seg, &modified, "conflicted-foreground");
    let untracked = prop_style(seg, style, "untracked-foreground");
    // p10k's `meta` (%246F gray): the `@` of `@hash`, the `#` of `#tag`, and the remote branch
    // name.
    let meta = prop_style(seg, style, "meta-foreground");
    let mut s = String::new();
    // p10k `SHOW_CHANGESET`: when explicitly enabled, or when there is no local branch
    // (detached HEAD), show the first `changeset-hash-length` chars of the commit (default 8).
    // p10k's `SHOW_CHANGESET` (explicitly enabled, on the piecewise path) draws `icon + hash`.
    let show_changeset = matches!(
        seg.prop("show-changeset"),
        Some(crate::config::Prop::Bool(true))
    );
    let hash_len = match seg.prop("changeset-hash-length") {
        Some(crate::config::Prop::Int(n)) if *n > 0 => *n as usize,
        _ => 8,
    };
    let mut has_commit = false;
    if show_changeset && !v.commit.is_empty() {
        let hash: String = v.commit.chars().take(hash_len).collect();
        let mut t = String::new();
        if !commit_icon.is_empty() {
            t.push_str(commit_icon);
            t.push(' ');
        }
        t.push_str(&hash);
        s.push_str(&paint(&t, &clean));
        has_commit = true;
    }
    if !v.branch.is_empty() {
        let mut b = String::new();
        if has_commit {
            b.push(' ');
        }
        if !branch_icon.is_empty() {
            b.push_str(branch_icon);
            b.push(' ');
        }
        b.push_str(&shorten_branch(&v.branch, seg));
        s.push_str(&paint(&b, &clean));
    } else if !v.tag.is_empty() {
        // p10k: `${meta}#${clean}${tag}`; tag names over 32 chars shorten like branch names.
        s.push_str(&paint("#", &meta));
        s.push_str(&paint(&shorten_branch(&v.tag, seg), &clean));
    } else if !v.commit.is_empty() {
        // p10k: `${meta}@${clean}${VCS_STATUS_COMMIT[1,8]}` (detached HEAD).
        let hash: String = v.commit.chars().take(hash_len).collect();
        s.push_str(&paint("@", &meta));
        s.push_str(&paint(&hash, &clean));
    }
    // p10k:`if [[ -n ${VCS_STATUS_REMOTE_BRANCH:#$VCS_STATUS_LOCAL_BRANCH} ]]`
    // → `${meta}:${clean}${remote_branch}`, directly after the branch name (no space).
    if !v.remote_branch.is_empty() && v.remote_branch != v.branch {
        s.push_str(&paint(":", &meta));
        s.push_str(&paint(&v.remote_branch, &clean));
    }
    // Count symbols can be overridden via props (in p10k these chars are hardcoded in the
    // config's formatting functions; exposing them lets users swap them without touching the
    // engine).
    let sym = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    let mut parts: Vec<(String, &Style)> = Vec::new();
    // Order and symbols match the vcs formatting function in p10k's generated config:
    //   wip → ⇣behind⇡ahead → ⇠push_behind⇢push_ahead → *stashes → <action>
    //   → ~conflicted → +staged → !unstaged → ?untracked → ─
    // No space between ahead/behind, nor between the push pair (p10k has `⇣42⇡42`).
    // p10k: a standalone word wip/WIP in the commit summary inserts a `wip` (modified color).
    if has_wip_word(&v.commit_summary) {
        parts.push(("wip".to_string(), &modified));
    }
    if v.ahead > 0 || v.behind > 0 {
        let mut ab = String::new();
        if v.behind > 0 {
            ab.push_str(&format!("{}{}", sym("behind-symbol", "⇣"), v.behind));
        }
        if v.ahead > 0 {
            ab.push_str(&format!("{}{}", sym("ahead-symbol", "⇡"), v.ahead));
        }
        parts.push((ab, &clean));
    }
    // p10k: `${clean}⇠N` / `${clean}⇢N` (behind/ahead of the push remote).
    if v.push_ahead > 0 || v.push_behind > 0 {
        let mut pb = String::new();
        if v.push_behind > 0 {
            pb.push_str(&format!(
                "{}{}",
                sym("push-behind-symbol", "⇠"),
                v.push_behind
            ));
        }
        if v.push_ahead > 0 {
            pb.push_str(&format!(
                "{}{}",
                sym("push-ahead-symbol", "⇢"),
                v.push_ahead
            ));
        }
        parts.push((pb, &clean));
    }
    if v.stashes > 0 {
        parts.push((format!("{}{}", sym("stash-symbol", "*"), v.stashes), &clean));
    }
    // p10k: `[[ -n $VCS_STATUS_ACTION ]] && res+=" ${conflicted}${VCS_STATUS_ACTION}"`
    // — an in-progress operation such as merge/rebase is drawn in the conflicted color before
    // the conflict count.
    if !v.action.is_empty() {
        parts.push((v.action.clone(), &conflicted));
    }
    if v.conflicted > 0 {
        parts.push((
            format!("{}{}", sym("conflicted-symbol", "~"), v.conflicted),
            &conflicted,
        ));
    }
    if v.staged > 0 {
        parts.push((
            format!("{}{}", sym("staged-symbol", "+"), v.staged),
            &modified,
        ));
    }
    if v.unstaged > 0 {
        parts.push((
            format!("{}{}", sym("unstaged-symbol", "!"), v.unstaged),
            &modified,
        ));
    }
    if v.untracked > 0 {
        parts.push((
            format!("{}{}", sym("untracked-symbol", "?"), v.untracked),
            &untracked,
        ));
    }
    // p10k: `(( VCS_STATUS_HAS_UNSTAGED == -1 )) && res+=" ${modified}─"`:
    // with `max-index-size-dirty` set and the index larger than it the dirty scan is skipped
    // and the unstaged count is unknown (this is the rule the gitstatus plugin uses to set
    // HAS_UNSTAGED to -1).
    let dirty_cap = match seg.prop("max-index-size-dirty") {
        Some(crate::config::Prop::Int(n)) => Some(*n),
        _ => None,
    };
    if let Some(cap) = dirty_cap
        && cap >= 0
        && v.index_size > cap as usize
    {
        parts.push(("─".to_string(), &modified));
    }
    if !parts.is_empty() {
        // Spaces are colored with the segment style: a bare space is drawn with the terminal
        // default background and would show a black seam in a backgrounded segment (classic
        // etc.).
        let sep = paint(" ", style);
        s.push_str(&sep);
        let joined: Vec<String> = parts.iter().map(|(t, st)| paint(t, st)).collect();
        s.push_str(&joined.join(&sep));
    }
    s
}

/// p10k's `[[ $VCS_STATUS_COMMIT_SUMMARY == (|*[^[:alnum:]])(wip|WIP)(|[^[:alnum:]]*) ]]`:
/// `wip` / `WIP` in the commit summary delimited by non-alphanumerics (or the string ends).
fn has_wip_word(summary: &str) -> bool {
    let bytes: Vec<char> = summary.chars().collect();
    let is_word = |c: char| c.is_alphanumeric();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let cand: String = bytes[i..i + 3].iter().collect();
        if cand == "wip" || cand == "WIP" {
            let before_ok = i == 0 || !is_word(bytes[i - 1]);
            let after_ok = i + 3 == bytes.len() || !is_word(bytes[i + 3]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Branch name shortening (p10k `VCS_SHORTEN_*`): only when both `shorten-length` and
/// `shorten-min-length` are set and the name is longer than both; `shorten-strategy` picks
/// `truncate_middle` (first N + ellipsis + last N) or `truncate_from_right` (first N +
/// ellipsis, default).
fn shorten_branch(branch: &str, seg: &Segment) -> String {
    let num = |name: &str| -> Option<usize> {
        match seg.prop(name) {
            Some(crate::config::Prop::Int(n)) if *n > 0 => Some(*n as usize),
            _ => None,
        }
    };
    let (Some(len), Some(min)) = (num("shorten-length"), num("shorten-min-length")) else {
        return branch.to_string();
    };
    let chars: Vec<char> = branch.chars().collect();
    if chars.len() <= min || chars.len() <= len {
        return branch.to_string();
    }
    let delim = match seg.prop("shorten-delimiter") {
        Some(crate::config::Prop::Str(s)) => s.clone(),
        _ => "\u{2026}".to_string(),
    };
    let head: String = chars[..len].iter().collect();
    let middle = matches!(
        seg.prop("shorten-strategy"),
        Some(crate::config::Prop::Str(s)) if s == "truncate_middle"
    );
    if middle {
        let tail: String = chars[chars.len() - len..].iter().collect();
        format!("{head}{delim}{tail}")
    } else {
        format!("{head}{delim}")
    }
}

/// Exit code status. The OK/ERROR icon chars are passed in by the caller, resolved from the
/// `ok`/`error` icon names (users can override them in the top-level `icon{}`); colors come
/// from `ok-foreground`/`error-foreground`. With `verbose #false` success is not shown
/// (matching p10k's `STATUS_OK`).
fn status_text(info: &HeaderInfo, ok: &str, err: &str, seg: &Segment, style: &Style) -> String {
    match info.exit_code {
        None => String::new(),
        Some(0) => {
            let verbose = matches!(seg.prop("verbose"), Some(crate::config::Prop::Bool(true)));
            if verbose {
                paint(ok, &prop_style(seg, style, "ok-foreground"))
            } else {
                String::new()
            }
        }
        Some(n) => {
            let text = if err.is_empty() {
                n.to_string()
            } else {
                format!("{err} {n}")
            };
            paint(&text, &prop_style(seg, style, "error-foreground"))
        }
    }
}

fn value_of(content: Option<&str>, default: impl Into<String>) -> String {
    content.map(String::from).unwrap_or(default.into())
}

/// Assembles one line as ANSI: left segments (joined by `sub`/`segment`, closed by `end`) +
/// right-aligned right segments.
fn assemble_row(
    left: &[SegmentText],
    right: &[SegmentText],
    cols: usize,
    seps: &crate::config::Separators,
    right_indent: usize,
) -> String {
    let mut out = String::new();
    // Start cap of the left column's first segment (left triangle, drawn before the leftmost
    // segment).
    if !seps.left_tail.is_empty()
        && let Some(first) = left.iter().find(|s| !s.text.is_empty())
        && first.style.bg != Color::Default
    {
        out.push_str(&arrow(
            &seps.left_tail,
            first.style.bg.clone(),
            Color::Default,
        ));
    }
    let mut prev_bg = Color::Default;
    let mut prev_padded = false;
    let mut has_left = false;
    for s in left {
        if s.text.is_empty() {
            continue;
        }
        // Inter-segment separation (p10k's rule): previous segment has a background → draw a
        // separator (same bg → sub, different color / current has none → segment); previous
        // has none → a space. Segments carry their trailing padding (see render_segment), so no
        // extra space is added, avoiding doubles (p10k uses one space between them).
        if has_left {
            let prev_has = prev_bg != Color::Default;
            if prev_has {
                let same = s.style.bg != Color::Default && s.style.bg == prev_bg;
                let ch = if same { &seps.sub } else { &seps.segment };
                if !ch.is_empty() {
                    if same {
                        // Same-background fine line: use `sub-foreground` when set (p10k's
                        // sep_color, a uniform gray), otherwise the next segment's foreground.
                        let st = match &seps.sub_foreground {
                            Some(fg) => Style {
                                fg: fg.clone(),
                                ..s.style.clone()
                            },
                            None => s.style.clone(),
                        };
                        out.push_str(&paint(ch, &st));
                    } else {
                        out.push_str(&arrow(ch, prev_bg.clone(), s.style.bg.clone()));
                    }
                } else if !prev_padded {
                    out.push(' ');
                }
            } else if !prev_padded {
                out.push(' ');
            }
        }
        out.push_str(&s.text); // text is already colored, do not recolor
        prev_bg = s.style.bg.clone();
        prev_padded = s.padded;
        has_left = true;
    }
    // End cap of the left column (after the last segment, its own background pointing to the
    // line end).
    if has_left && !seps.end.is_empty() && prev_bg != Color::Default {
        out.push_str(&arrow(&seps.end, prev_bg.clone(), Color::Default));
    }
    let mut right_str = String::new();
    let parts: Vec<&SegmentText> = right.iter().filter(|s| !s.text.is_empty()).collect();
    if !parts.is_empty() {
        // Start cap of the right column (left triangle ): foreground = the right segment's **background** color (pointing at it); drawn over the gap.
        if !seps.right_start.is_empty() {
            right_str.push_str(&arrow(
                &seps.right_start,
                parts[0].style.bg.clone(),
                Color::Default,
            ));
        }
        for (i, s) in parts.iter().enumerate() {
            if i > 0 {
                // Inter-segment separation on the right: same color → right_sub (the segment
                // foreground drawn on the segment background), different → right_segment
                // (left triangle, foreground (the triangle block) = the current (right)
                // segment's bg, background = the previous (left) segment's bg;
                // mirroring the left segment `` (fg = previous bg, bg = current bg)).
                let prev_bg = parts[i - 1].style.bg.clone();
                if prev_bg != Color::Default {
                    let same = s.style.bg != Color::Default && s.style.bg == prev_bg;
                    if same {
                        if !seps.right_sub.is_empty() {
                            let st = match &seps.right_sub_foreground {
                                Some(fg) => Style {
                                    fg: fg.clone(),
                                    ..s.style.clone()
                                },
                                None => s.style.clone(),
                            };
                            right_str.push_str(&paint(&seps.right_sub, &st));
                        }
                    } else if !seps.right_segment.is_empty() {
                        right_str.push_str(&arrow(
                            &seps.right_segment,
                            s.style.bg.clone(),
                            prev_bg,
                        ));
                    }
                }
            }
            right_str.push_str(&s.text);
        }
        // Closing cap of the right column's last segment (right triangle, drawn after the
        // rightmost segment).
        if !seps.right_tail.is_empty()
            && let Some(last) = parts.last()
            && last.style.bg != Color::Default
        {
            right_str.push_str(&arrow(
                &seps.right_tail,
                last.style.bg.clone(),
                Color::Default,
            ));
        }
        let lw = display_width(&out);
        let rw = display_width(&right_str);
        // Right alignment: gap chars fill the span from the left segments to the right
        // column's start. The right column keeps `right_indent` columns from the right edge—
        // zsh's `ZLE_RPROMPT_INDENT` defaults to 1, so p10k's right column always has one cell
        // before the window border; drawing flush would make it "fit" one column earlier than
        // p10k and reach the last cell (many terminals wrap there).
        let edge = cols.saturating_sub(right_indent);
        if edge > rw {
            let start = edge - rw; // right column start (0-based)
            if start > lw {
                let gap_char = if seps.gap.is_empty() { " " } else { &seps.gap };
                let gap_str = gap_char.repeat(start - lw);
                // gap foreground (p10k MULTILINE_FIRST_PROMPT_GAP_FOREGROUND);
                // unset → emitted as-is (terminal default color).
                match &seps.gap_foreground {
                    Some(fg) => out.push_str(&paint(
                        &gap_str,
                        &Style {
                            fg: fg.clone(),
                            ..Style::default()
                        },
                    )),
                    None => out.push_str(&gap_str),
                }
            }
        }
        out.push_str(&right_str);
    }
    out
}

/// Draws a separator arrow: `fg` is its foreground (joining the previous background), `bg` the
/// background.
fn arrow(ch: &str, fg: Color, bg: Color) -> String {
    let mut s = String::new();
    match fg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(&n)).unwrap(),
    }
    match bg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(&n)).unwrap(),
    }
    s.push_str(ch);
    s.push_str("\x1b[0m");
    s
}

/// Colors text with a style (foreground + background block + bold; plain text, no separators).
fn paint(text: &str, style: &Style) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    match &style.fg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(n)).unwrap(),
    }
    match &style.bg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(n)).unwrap(),
    }
    if style.bold {
        s.push_str("\x1b[1m");
    }
    s.push_str(text);
    s.push_str("\x1b[0m");
    s
}

/// Maps the standard 16 color names to 256-color indexes.
fn named_256(name: &str) -> u8 {
    match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "brightblack" => 8,
        "brightred" => 9,
        "brightgreen" => 10,
        "brightyellow" => 11,
        "brightblue" => 12,
        "brightmagenta" => 13,
        "brightcyan" => 14,
        "brightwhite" => 15,
        _ => 7,
    }
}

/// Display width (ANSI stripped, per Unicode width: emoji/CJK wide chars count 2, combining/
/// zero-width count 0). Used by theme to compute how many rows the terminal wraps content
/// wider than cols into.
pub fn display_width_of(s: &str) -> usize {
    display_width(s)
}

fn display_width(s: &str) -> usize {
    let mut w = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_esc = true;
            continue;
        }
        w += UnicodeWidthChar::width(c).unwrap_or(0);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(cwd: &str, code: Option<i32>) -> HeaderInfo {
        HeaderInfo {
            exit_code: code,
            cwd: cwd.to_string(),
            exec_seconds: 0.0,
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

    #[test]
    fn segment_prefix_and_suffix_render() {
        // p10k's SEG_PREFIX/SEG_SUFFIX (vcs `on `, exec `took `):
        // drawn at the very start/end of the segment.
        let mut cfg = Config::default();
        cfg.layout.left = vec![vec![Element::Seg("custom".into())]];
        let s = crate::config::Segment {
            content: Some("main".into()),
            prefix: Some("on ".into()),
            suffix: Some("!".into()),
            ..Default::default()
        };
        cfg.segments.insert("custom".into(), s);
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
        assert!(h.contains("\u{f00c}"));
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
    fn input_prefix_width_matches_visual() {
        // With the frame last-prefix ╰─, the prefix is "╰─❯ " (width 4); placeholders are
        // generated at this width.
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  last-prefix \"╰─\"\n}",
        )
        .unwrap();
        let p = input_prefix(&cfg, None);
        assert_eq!(
            p.width, 4,
            "╰─❯ plus space should be width 4, got width={} text={:?}",
            p.width, p.text
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
    fn disabled_workdir_pattern_follows_p10k() {
        // p10k's VCS_DISABLED_WORKDIR_PATTERN: the pattern is a zsh pattern matched against
        // the repo's workdir, `~` is $HOME, and an unset pattern disables nothing.
        assert!(!workdir_matches("", "/home/u/repo", "/home/u"));
        assert!(workdir_matches("~", "/home/u", "/home/u"));
        // `~` alone is not a prefix match: $HOME/.git stays visible.
        assert!(!workdir_matches("~", "/home/u/.git", "/home/u"));
        assert!(!workdir_matches("~", "/tmp/repo", "/home/u"));
        // Alternation, and a group with an empty branch (p10k's own README example).
        assert_eq!(
            pattern_alternatives("~(|/foo)|/bar/baz/*"),
            ["~", "~/foo", "/bar/baz/*"]
        );
        assert!(workdir_matches("~(|/foo)", "/home/u/foo", "/home/u"));
        assert!(!workdir_matches("~(|/foo)", "/home/u/bar", "/home/u"));
        assert!(workdir_matches("/tmp|/var", "/var", "/home/u"));
        // A pattern ending in `/` covers the subdirectories; `|` inside `[...]` is literal.
        assert!(workdir_matches("~/work/", "/home/u/work/a/b", "/home/u"));
        assert_eq!(pattern_alternatives("[a|b]"), ["[a|b]"]);
        assert_eq!(pattern_alternatives("a(|b)c"), ["ac", "abc"]);
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
        // status_text takes the ok/err icon chars; the default chars themselves come from the
        // icon{} table / built-in defaults.
        let mut seg = Segment::default();
        seg.props
            .insert("verbose".into(), crate::config::Prop::Bool(true));
        let style = Style::default();
        assert!(
            status_text(&info("/tmp", Some(0)), "\u{f00c}", "\u{f00d}", &seg, &style)
                .contains("\u{f00c}"),
            "OK should show the ok icon"
        );
        assert!(
            status_text(&info("/tmp", Some(1)), "ok", "err", &seg, &style).contains("err 1"),
            "ERROR should show the err icon + exit code"
        );
        // Icon defaults follow mode: folder has a glyph in nf and is empty in compat/ascii;
        // go has text in all three tiers.
        let mk = |m: &str| {
            Config::parse(&format!(
                "mode \"{m}\"\nlayout {{ left {{ line {{ dir #true }} }} }}"
            ))
            .unwrap()
        };
        assert_eq!(
            icon_str(&mk("nerdfont-complete"), "folder").unwrap(),
            "\u{f07c}"
        );
        assert_eq!(icon_str(&mk("compatible"), "folder").unwrap(), "");
        assert_eq!(icon_str(&mk("ascii"), "folder").unwrap(), "");
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

    #[test]
    fn prompt_char_states_must_match_width() {
        // Normal and ERROR chars are equal width (both single chars) → validation passes.
        let ok_cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\"❯\" {\n  state ERROR char=\"✘\"\n} }",
        )
        .unwrap();
        assert!(
            check_prompt_char_widths(&ok_cfg).is_ok(),
            "equal widths should pass validation"
        );
        // Unequal widths (✘✘ double width) → validation errors.
        let bad_cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\"❯\" {\n  state ERROR char=\"✘✘\"\n} }",
        )
        .unwrap();
        assert!(
            check_prompt_char_widths(&bad_cfg).is_err(),
            "unequal widths should be rejected"
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
        let ii = h.find("\u{f07c}").expect("folder icon should render");
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
        // The row width must never exceed cols: the placeholder protocol assumes one terminal
        // row per line, and an overflow wrap would break the header row count (and mis-count
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
        // drawn rather than overflowing. An overflowing row breaks the placeholder protocol
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
            !locked.contains("\u{f07c}"),
            "the folder icon should not be shown as well, got {locked:?}"
        );
        // A writable directory behaves as usual.
        let normal = render("/tmp", "show-writable=v3");
        assert!(normal.contains("\u{f07c}"), "got {normal:?}");
        assert!(!normal.contains("\u{f023}"), "got {normal:?}");
        // Unset means no check at all.
        let off = render("/proc", "");
        assert!(off.contains("\u{f07c}"), "got {off:?}");
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
            "\u{f07c} /"
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
        // Only Projects/crates are unique among their siblings, so only they shorten once long
        // enough. The fixture dir names are fixed, making the columns saved per level
        // deterministic: tmp saves 2, p11k-width-fixture 17, Projects 7, powerlevel11k 12,
        // crates 5 (cumulative 2/19/26/38/43).
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
        let wide = dir_text(200, &cwd);
        assert!(
            wide.contains("Projects/"),
            "wide row should not shorten, got {wide:?}"
        );
        assert!(
            wide.contains("crates/"),
            "wide row should not shorten, got {wide:?}"
        );
        // 20 cols short: the first three levels (including Projects) shorten, powerlevel11k and
        // crates remain.
        let mid = dir_text(full_w - 20, &cwd);
        assert!(
            !mid.contains("Projects") && mid.contains("crates"),
            "medium width should shorten only down to Projects, got {mid:?}"
        );
        // 40 cols short: crates shortens as well.
        let narrow = dir_text(full_w - 40, &cwd);
        assert!(
            !narrow.contains("Projects") && !narrow.contains("crates"),
            "very narrow should shorten both levels, got {narrow:?}"
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
}
