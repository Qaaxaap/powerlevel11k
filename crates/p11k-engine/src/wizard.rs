//! `p11k configure` —— 交互配置向导。
//!
//! 对齐 p10k `p10k configure` 暴露给用户的问题集：字体检测（三个字形人眼确认）、
//! 风格、字符集、颜色变体、时间、分隔符/端符、行数、连接线、帧、间距、图标、
//! 前缀、transient。答案直接改 [`Config`] 字段，最后序列化成 KDL 落盘。
//! 键位：`q` 退出（什么都不写）、`r` 从头再来、数字/字母选择。

use crate::config::{Color, Config, Element, Frame, IconMode, Prop, Segment, Separators};
use crate::presets::{self, PresetKind};
use crate::theme::HeaderInfo;
use std::io::{self, Write};
use std::path::PathBuf;

/// 一步的答案：答案值 / 从头再来 / 退出。
enum Step<T> {
    Answer(T),
    Restart,
    Quit,
}

/// 把任意 `Step<T>` 的 Restart/Quit 变体转成 `Step<U>`（Answer 不会走到这里）。
fn early<T, U>(s: Step<T>) -> Step<U> {
    match s {
        Step::Answer(_) => unreachable!("early 只处理 Restart/Quit"),
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
    // ① 字体检测 → 图标模式。
    let mode = match ask_font(out)? {
        Step::Answer(m) => m,
        s => return Ok(early(s)),
    };
    // ② 风格。
    let kind = match ask_style(out)? {
        Step::Answer(k) => k,
        s => return Ok(early(s)),
    };
    // ③ 颜色变体（按风格分支）→ 构建带颜色参数的 Config。
    let mut cfg = match ask_color(out, kind, &mode)? {
        Step::Answer(c) => c,
        s => return Ok(early(s)),
    };
    // ④ 字符集（非 ascii 时问，可显式切 ASCII）。
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
    // ⑤ pure 的非永久内容位置。
    if kind == PresetKind::Pure {
        match ask_use_rprompt(out, &mut cfg)? {
            Step::Answer(()) => {}
            s => return Ok(early(s)),
        }
    }
    // ⑥ 时间。
    match ask_time(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑦ 分隔符/端符（仅 classic/rainbow）。
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
    // ⑧ 行数。
    match ask_num_lines(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑨ 连接线（仅两行且 classic/rainbow）。
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
    // ⑩ 间距。
    match ask_empty_line(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑪ 图标。
    match ask_extra_icons(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑫ 前缀。
    match ask_prefixes(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑬ transient。
    match ask_transient(out, &mut cfg)? {
        Step::Answer(()) => {}
        s => return Ok(early(s)),
    }
    // ⑭ 预览 + 确认。
    preview(out, &cfg)?;
    match ask_yn(out, "就用这个主题？", "是，写入配置", "否", None)? {
        Step::Answer(true) => {}
        Step::Answer(false) => return Ok(Step::Quit),
        Step::Restart => return Ok(Step::Restart),
        Step::Quit => return Ok(Step::Quit),
    }
    // ⑮ 落盘（已存在则先问覆盖）。
    write_config(out, &cfg).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Step::Answer(()))
}

// ---- UI 基础 ----

fn clear(out: &mut io::Stdout) {
    let _ = out.write_all(b"\x1b[2J\x1b[H");
    let _ = out.flush();
}

/// 写一行（raw 终端 OPOST 关闭，`\n` 不回列首，显式补 `\r`）。
fn line(out: &mut io::Stdout, s: &str) -> io::Result<()> {
    out.write_all(s.as_bytes())?;
    out.write_all(b"\r\n")?;
    out.flush()
}

/// 读单键。`q`→Quit、`r`→Restart、其余原样返回；Ctrl-C/Esc 当 Quit。
fn key() -> io::Result<Option<u8>> {
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
            return Ok(None);
        }
        return Ok(Some(b[0]));
    }
}

fn goodbye(out: &mut io::Stdout) -> anyhow::Result<()> {
    let _ = line(out, "");
    let _ = line(out, "已退出，什么都没写。");
    Ok(())
}

/// 当前目录 + 一个让 exec/status 都可见的样例状态。
fn sample_info() -> HeaderInfo {
    HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
        exec_seconds: 3.5,
        jobs: 1,
        history: 0,
    }
}

fn header_on(cfg: &Config) -> bool {
    !(cfg.layout.left.is_empty() && cfg.layout.right.is_empty())
}

/// 样例 git 状态：让预览里的 vcs 段像真实 prompt（p10k wizard 也用假数据）。
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

/// 渲染当前配置的完整 prompt 预览（header 行 + 输入行），供逐选项预览用。
fn preview_lines(cfg: &Config) -> Vec<String> {
    let cols = crate::tty_size().map(|(_, c, ..)| c as usize).unwrap_or(80);
    let vcs = sample_vcs();
    let mut lines = crate::render::render_header_lines(cfg, &sample_info(), Some(&vcs), cols);
    // 输入行：引擎画的 prompt_char 前缀（后面是 shell 的 buffer）。
    lines.push(crate::render::input_prefix(cfg, None).text);
    lines
}

// ---- 问题集 ----

/// 字体检测：diamond(⮀⮂) + lock()/quotes(❯❮) 三个字形，落到三档 mode。
fn ask_font(out: &mut io::Stdout) -> io::Result<Step<IconMode>> {
    let diamond = match ask_yn(
        out,
        "这个看起来像菱形（旋转的正方形）吗？",
        "是",
        "否",
        Some("--->  \u{e0b2}\u{e0b0}  <---"),
    )? {
        Step::Answer(b) => b,
        s => return Ok(early(s)),
    };
    if diamond {
        match ask_yn(
            out,
            "这个看起来像锁吗？",
            "是",
            "否",
            Some("--->  \u{f023}  <---"),
        )? {
            Step::Answer(true) => Ok(Step::Answer(IconMode::NerdfontComplete)),
            Step::Answer(false) => Ok(Step::Answer(IconMode::Compatible)),
            s => Ok(early(s)),
        }
    } else {
        match ask_yn(
            out,
            "这个看起来像 >< 但更高更胖吗？",
            "是",
            "否",
            Some("--->  \u{276f}\u{276e}  <---"),
        )? {
            Step::Answer(true) => Ok(Step::Answer(IconMode::Compatible)),
            Step::Answer(false) => Ok(Step::Answer(IconMode::Ascii)),
            s => Ok(early(s)),
        }
    }
}

/// 风格：四套预设各带一份 live prompt 预览。
fn ask_style(out: &mut io::Stdout) -> io::Result<Step<PresetKind>> {
    let titles = PresetKind::ALL.map(|k| k.title());
    let i = ask_choice_preview(out, "提示符风格", &titles, |i| {
        preview_lines(&presets::build(PresetKind::ALL[i]))
    })?;
    match i {
        Step::Answer(i) => Ok(Step::Answer(PresetKind::ALL[i])),
        s => Ok(early(s)),
    }
}

/// 按风格 + 颜色档索引构建配置（预览与最终结果同源）。
fn build_with_color(kind: PresetKind, i: usize) -> Config {
    match kind {
        PresetKind::Lean => presets::lean(i == 1),
        PresetKind::Classic => presets::classic(i + 1),
        PresetKind::Rainbow => presets::rainbow(i + 1),
        PresetKind::Pure => presets::pure(i == 1),
    }
}

/// 颜色变体（按风格分支）：lean 256/8、classic 四档、rainbow 帧四档、pure 两套。
fn ask_color(out: &mut io::Stdout, kind: PresetKind, mode: &IconMode) -> io::Result<Step<Config>> {
    const FOUR: [&str; 4] = [
        "最浅（Lightest）",
        "浅（Light）",
        "深（Dark）",
        "最深（Darkest）",
    ];
    let (title, options): (&str, &[&str]) = match kind {
        PresetKind::Lean => ("提示符颜色", &["256 色", "8 色（兼容终端）"]),
        PresetKind::Classic => ("提示符颜色", &FOUR),
        PresetKind::Rainbow => ("边框颜色", &FOUR),
        PresetKind::Pure => ("提示符颜色", &["Original", "Snazzy"]),
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

/// 字符集：Unicode / ASCII（预览反映 ASCII 档下图标/分隔符的替换）。
fn ask_charset(out: &mut io::Stdout, cfg: &Config, mode: &IconMode) -> io::Result<Step<bool>> {
    let i = ask_choice_preview(out, "字符集", &["Unicode", "ASCII"], |i| {
        let mut c = cfg.clone();
        c.mode = if i == 1 {
            IconMode::Ascii
        } else {
            mode.clone()
        };
        preview_lines(&c)
    })?;
    match i {
        Step::Answer(i) => Ok(Step::Answer(i == 1)),
        s => Ok(early(s)),
    }
}

/// pure 的非永久内容（exec/context/virtualenv）位置。
fn ask_use_rprompt(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "非永久内容位置",
        &["左侧", "右侧"],
        cfg,
        |c, i| {
            // 先把 exec 从两栏都摘掉，再按选择放回左/右。
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

/// 时间：不显示 / 12 小时制 / 24 小时制。
fn ask_time(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "显示当前时间？",
        &["不显示", "12 小时制", "24 小时制"],
        cfg,
        |c, i| match i {
            0 => remove_segment(c, "time"),
            _ => {
                add_to_right(c, "time");
                let fmt = if i == 1 { "12h" } else { "24h" };
                let t = c.segments.entry("time".into()).or_insert_with(|| {
                    let mut s = Segment::default();
                    s.style.fg = Color::Xterm(66);
                    s
                });
                t.props.insert("time-format".into(), Prop::Str(fmt.into()));
            }
        },
    )
}

/// 分隔符（segment/sub）：Angled/Vertical/Slanted/Round。
fn ask_separators(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "分隔符",
        &[
            "尖角（Angled）",
            "竖直（Vertical）",
            "斜（Slanted）",
            "圆角（Round）",
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

/// 端符 heads（end/right-start）：Flat/Blurred/Sharp/Slanted/Round。
fn ask_heads(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "端符（heads）",
        &[
            "平（Flat）",
            "渐变（Blurred）",
            "尖（Sharp）",
            "斜（Slanted）",
            "圆（Round）",
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

/// 端符 tails（left-tail/right-tail）。
fn ask_tails(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "端符（tails）",
        &[
            "平（Flat）",
            "渐变（Blurred）",
            "尖（Sharp）",
            "斜（Slanted）",
            "圆（Round）",
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

/// 行数：一行（无 header）/ 两行。
fn ask_num_lines(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "提示符高度",
        &["一行（只有输入行）", "两行（信息行 + 输入行）"],
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

/// 连接线：Disconnected/Dotted/Solid。
fn ask_gap_char(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "连接线",
        &["断开（空格）", "点线", "实线"],
        cfg,
        |c, i| {
            c.separators.gap = match i {
                0 => " ",
                1 => "·",
                _ => "─",
            }
            .into();
            // 有填充字符时给个灰前景（对齐 p10k 的
            // MULTILINE_FIRST_PROMPT_GAP_FOREGROUND=240）；断开则清掉。
            c.separators.gap_foreground = if i == 0 {
                None
            } else {
                Some(Color::Xterm(240))
            };
        },
    )
}

/// 帧：无/左/右/全。
fn ask_frame(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    use crate::config::FramePiece;
    ask_apply(
        out,
        "提示符边框",
        &["无边框", "只有左侧", "只有右侧", "完整边框"],
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

/// 间距：Compact/Sparse。
fn ask_empty_line(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "提示符间距",
        &["紧凑（无空行）", "稀疏（prompt 之间空一行）"],
        cfg,
        |c, i| c.layout.prompt_add_newline = usize::from(i == 1),
    )
}

/// 图标：Few/Many。
fn ask_extra_icons(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "图标",
        &["少（几乎无图标）", "多（完整图标）"],
        cfg,
        |c, i| {
            if i == 0 {
                // 对齐 p10k 的 few：os 徽标整个不画，dir/vcs/branch/exec/time
                // 图标清空。vcs 的远端图标（github/gitlab）走的是
                // `vcs-remote-icons`，优先于 `icon{ git }`，所以要一并清掉，
                // 否则 Few 下远端仓库仍带着 github 图标（p10k 是把
                // VCS_VISUAL_IDENTIFIER_EXPANSION 整体置空）。exec 图标对应
                // p10k 的 COMMAND_EXECUTION_TIME_VISUAL_IDENTIFIER_EXPANSION，
                // p10k 在 Few 下同样置空。
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

/// 前缀连词：Concise（裸值）/ Fluent（带 on/took/at）。
fn ask_prefixes(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    ask_apply(
        out,
        "提示符措辞",
        &[
            "简洁（Concise）：只写值，如 master 5s",
            "流畅（Fluent）：加连词，如 on master took 5s",
        ],
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

/// transient：y/n。
fn ask_transient(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let b = ask_yn(
        out,
        "启用 transient prompt？（命令提交后 header 折叠成单行 ❯，仅 zsh）",
        "是",
        "否",
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

// ---- 通用问答 ----

/// n 选 1，每个选项下方渲染它的 live prompt 预览（对齐 p10k wizard 的
/// add_prompt：每项都带一份当前配置下的完整 prompt）。
fn ask_choice_preview(
    out: &mut io::Stdout,
    title: &str,
    options: &[&str],
    preview: impl Fn(usize) -> Vec<String>,
) -> io::Result<Step<usize>> {
    loop {
        clear(out);
        line(out, title)?;
        line(out, "")?;
        for (i, label) in options.iter().enumerate() {
            line(out, &format!("({})  {}", i + 1, label))?;
            for l in preview(i) {
                line(out, &l)?;
            }
            line(out, "")?;
        }
        line(out, "(r)  从头再来")?;
        match key()? {
            Some(b'q') => return Ok(Step::Quit),
            Some(b'r') => return Ok(Step::Restart),
            Some(k @ b'1'..=b'9') => {
                let i = (k - b'1') as usize;
                if i < options.len() {
                    return Ok(Step::Answer(i));
                }
            }
            _ => {}
        }
    }
}

/// 带预览的问答骨架：克隆当前配置 → 应用第 i 个选项 → 渲染预览；
/// 选中后把同一份修改应用到真实配置（预览与结果同源，不会漂移）。
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

/// 是/否：y/n。`sample` 可选，展示一行字形样本。
fn ask_yn(
    out: &mut io::Stdout,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> io::Result<Step<bool>> {
    loop {
        clear(out);
        line(out, title)?;
        if let Some(s) = sample {
            line(out, "")?;
            line(out, s)?;
        }
        line(out, "")?;
        line(out, &format!("(y)  {yes}"))?;
        line(out, "")?;
        line(out, &format!("(n)  {no}"))?;
        line(out, "(r)  从头再来")?;
        match key()? {
            Some(b'q') => return Ok(Step::Quit),
            Some(b'r') => return Ok(Step::Restart),
            Some(b'y') => return Ok(Step::Answer(true)),
            Some(b'n') => return Ok(Step::Answer(false)),
            _ => {}
        }
    }
}

// ---- 配置编辑辅助 ----

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

// ---- 预览 / 落盘 ----

fn preview(out: &mut io::Stdout, cfg: &Config) -> io::Result<()> {
    clear(out);
    line(out, "预览（当前目录 + 退出码）:")?;
    line(out, "")?;
    for l in preview_lines(cfg) {
        line(out, &l)?;
    }
    line(out, "")?;
    Ok(())
}

fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("p11k").join("p11k.kdl")
}

fn write_config(out: &mut io::Stdout, cfg: &Config) -> anyhow::Result<()> {
    let path = default_path();
    if path.exists() {
        match ask_yn(
            out,
            &format!("配置文件已存在：{}，覆盖？", path.display()),
            "是",
            "否",
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
    let p = path.display();
    line(out, "")?;
    line(out, &format!("主题已写入：{p}"))?;
    line(out, "")?;
    line(out, "安装到 shell rc（选你用的那个）:")?;
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
    line(out, "")?;
    line(
        out,
        "引擎即主题：先去掉原本的 ZSH_THEME/主题设置，否则会叠加。",
    )?;
    Ok(())
}
