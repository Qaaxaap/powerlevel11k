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

impl<T> Step<T> {
    fn is_answer(&self) -> bool {
        matches!(self, Step::Answer(_))
    }
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
    let mut cfg = match ask_color(out, kind)? {
        Step::Answer(c) => c,
        s => return Ok(early(s)),
    };
    cfg.mode = mode;
    // ④ 字符集（非 ascii 时问，可显式切 ASCII）。
    if cfg.mode != IconMode::Ascii {
        match ask_charset(out)? {
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

/// 渲染当前配置的 header 预览行（供 style/最终预览用）。
fn preview_lines(cfg: &Config) -> Vec<String> {
    let cols = crate::tty_size().map(|(_, c, ..)| c as usize).unwrap_or(80);
    crate::render::render_header_lines(cfg, &sample_info(), None, cols)
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

/// 风格：四套预设各渲染一行 header 预览。
fn ask_style(out: &mut io::Stdout) -> io::Result<Step<PresetKind>> {
    loop {
        clear(out);
        line(out, "提示符风格")?;
        line(out, "")?;
        for (i, k) in PresetKind::ALL.iter().enumerate() {
            line(out, &format!("({})  {}", i + 1, k.title()))?;
            for l in preview_lines(&presets::build(*k)) {
                line(out, &l)?;
            }
            line(out, "")?;
        }
        line(out, "(r)  从头再来")?;
        match key()? {
            Some(b'q') => return Ok(Step::Quit),
            Some(b'r') => return Ok(Step::Restart),
            Some(k @ b'1'..=b'4') => return Ok(Step::Answer(PresetKind::ALL[(k - b'1') as usize])),
            _ => {}
        }
    }
}

/// 颜色变体（按风格分支）：lean 256/8、classic 四档、rainbow 帧四档、pure 两套。
fn ask_color(out: &mut io::Stdout, kind: PresetKind) -> io::Result<Step<Config>> {
    let cfg = match kind {
        PresetKind::Lean => {
            let i = ask_choice(out, "提示符颜色", &["256 色", "8 色（兼容终端）"])?;
            match i {
                Step::Answer(i) => presets::lean(i == 1),
                s => return Ok(early(s)),
            }
        }
        PresetKind::Classic => {
            let i = ask_choice(
                out,
                "提示符颜色",
                &[
                    "最浅（Lightest）",
                    "浅（Light）",
                    "深（Dark）",
                    "最深（Darkest）",
                ],
            )?;
            match i {
                Step::Answer(i) => presets::classic(i + 1),
                s => return Ok(early(s)),
            }
        }
        PresetKind::Rainbow => {
            let i = ask_choice(
                out,
                "边框颜色",
                &[
                    "最浅（Lightest）",
                    "浅（Light）",
                    "深（Dark）",
                    "最深（Darkest）",
                ],
            )?;
            match i {
                Step::Answer(i) => presets::rainbow(i + 1),
                s => return Ok(early(s)),
            }
        }
        PresetKind::Pure => {
            let i = ask_choice(out, "提示符颜色", &["Original", "Snazzy"])?;
            match i {
                Step::Answer(i) => presets::pure(i == 1),
                s => return Ok(early(s)),
            }
        }
    };
    Ok(Step::Answer(cfg))
}

/// 字符集：Unicode / ASCII。
fn ask_charset(out: &mut io::Stdout) -> io::Result<Step<bool>> {
    let i = ask_choice(out, "字符集", &["Unicode", "ASCII"])?;
    match i {
        Step::Answer(i) => Ok(Step::Answer(i == 1)),
        s => Ok(early(s)),
    }
}

/// pure 的非永久内容（exec/context/virtualenv）位置。
fn ask_use_rprompt(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(out, "非永久内容位置", &["左侧", "右侧"])?;
    match i {
        Step::Answer(0) => Ok(Step::Answer(())),
        Step::Answer(1) => {
            if let Some(row) = cfg.layout.left.first_mut() {
                row.retain(|e| !matches!(e, Element::Seg(s) if s == "command_execution_time"));
            }
            if !cfg
                .layout
                .right
                .iter()
                .flatten()
                .any(|e| matches!(e, Element::Seg(s) if s == "command_execution_time"))
            {
                if cfg.layout.right.is_empty() {
                    cfg.layout.right.push(Vec::new());
                }
                cfg.layout.right[0].push(Element::Seg("command_execution_time".into()));
            }
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 时间：不显示 / 12 小时制 / 24 小时制。
fn ask_time(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(out, "显示当前时间？", &["不显示", "12 小时制", "24 小时制"])?;
    match i {
        Step::Answer(0) => {
            remove_segment(cfg, "time");
            Ok(Step::Answer(()))
        }
        Step::Answer(1) | Step::Answer(2) => {
            let fmt = if matches!(i, Step::Answer(1)) {
                "12h"
            } else {
                "24h"
            };
            add_to_right(cfg, "time");
            let t = cfg.segments.entry("time".into()).or_insert_with(|| {
                let mut s = Segment::default();
                s.style.fg = Color::Xterm(66);
                s
            });
            t.props.insert("time-format".into(), Prop::Str(fmt.into()));
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 分隔符（segment/sub）：Angled/Vertical/Slanted/Round。
fn ask_separators(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "分隔符",
        &[
            "尖角（Angled）",
            "竖直（Vertical）",
            "斜（Slanted）",
            "圆角（Round）",
        ],
    )?;
    let (seg, sub, rseg, rsub) = match i {
        Step::Answer(0) => ("\u{e0b0}", "\u{e0b1}", "\u{e0b2}", "\u{e0b3}"),
        Step::Answer(1) => ("", "\u{2502}", "", "\u{2502}"),
        Step::Answer(2) => ("\u{e0bc}", "\u{2571}", "\u{e0ba}", "\u{2571}"),
        Step::Answer(3) => ("\u{e0b4}", "\u{e0b5}", "\u{e0b6}", "\u{e0b7}"),
        _ => unreachable!(),
    };
    match i {
        Step::Answer(_) => {
            cfg.separators.segment = seg.into();
            cfg.separators.sub = sub.into();
            cfg.separators.right_segment = rseg.into();
            cfg.separators.right_sub = rsub.into();
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 端符 heads（end/right-start）：Flat/Blurred/Sharp/Slanted/Round。
fn ask_heads(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "端符（heads）",
        &[
            "平（Flat）",
            "渐变（Blurred）",
            "尖（Sharp）",
            "斜（Slanted）",
            "圆（Round）",
        ],
    )?;
    let (end, rstart) = match i {
        Step::Answer(0) => ("", ""),
        Step::Answer(1) => ("\u{2593}\u{2592}\u{2591}", "\u{2591}\u{2592}\u{2593}"),
        Step::Answer(2) => ("\u{e0b0}", "\u{e0b2}"),
        Step::Answer(3) => ("\u{e0bc}", "\u{e0ba}"),
        Step::Answer(4) => ("\u{e0b4}", "\u{e0b6}"),
        _ => unreachable!(),
    };
    match i {
        Step::Answer(_) => {
            cfg.separators.end = end.into();
            cfg.separators.right_start = rstart.into();
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 端符 tails（left-tail/right-tail）。
fn ask_tails(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "端符（tails）",
        &[
            "平（Flat）",
            "渐变（Blurred）",
            "尖（Sharp）",
            "斜（Slanted）",
            "圆（Round）",
        ],
    )?;
    let (ltail, rtail) = match i {
        Step::Answer(0) => ("", ""),
        Step::Answer(1) => ("\u{2591}\u{2592}\u{2593}", "\u{2593}\u{2592}\u{2591}"),
        Step::Answer(2) => ("\u{e0b2}", "\u{e0b0}"),
        Step::Answer(3) => ("\u{e0ba}", "\u{e0bc}"),
        Step::Answer(4) => ("\u{e0b6}", "\u{e0b4}"),
        _ => unreachable!(),
    };
    match i {
        Step::Answer(_) => {
            cfg.separators.left_tail = ltail.into();
            cfg.separators.right_tail = rtail.into();
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 行数：一行（无 header）/ 两行。
fn ask_num_lines(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "提示符高度",
        &["一行（只有输入行）", "两行（信息行 + 输入行）"],
    )?;
    match i {
        Step::Answer(0) => {
            cfg.layout.left.clear();
            cfg.layout.right.clear();
            cfg.frame = Frame::default();
            cfg.separators = Separators::default();
            Ok(Step::Answer(()))
        }
        Step::Answer(1) => Ok(Step::Answer(())),
        s => Ok(early(s)),
    }
}

/// 连接线：Disconnected/Dotted/Solid。
fn ask_gap_char(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(out, "连接线", &["断开（空格）", "点线", "实线"])?;
    match i {
        Step::Answer(0) => cfg.separators.gap = " ".into(),
        Step::Answer(1) => cfg.separators.gap = "·".into(),
        Step::Answer(2) => cfg.separators.gap = "─".into(),
        s => return Ok(early(s)),
    }
    Ok(Step::Answer(()))
}

/// 帧：无/左/右/全。
fn ask_frame(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "提示符边框",
        &["无边框", "只有左侧", "只有右侧", "完整边框"],
    )?;
    let f = &mut cfg.frame;
    match i {
        Step::Answer(0) => {
            f.first_prefix = crate::config::FramePiece::default();
            f.first_suffix = crate::config::FramePiece::default();
            f.newline_prefix = crate::config::FramePiece::default();
            f.newline_suffix = crate::config::FramePiece::default();
            f.last_prefix = crate::config::FramePiece::default();
            f.last_suffix = crate::config::FramePiece::default();
        }
        Step::Answer(1) => {
            f.first_suffix = crate::config::FramePiece::default();
            f.newline_suffix = crate::config::FramePiece::default();
            f.last_suffix = crate::config::FramePiece::default();
        }
        Step::Answer(2) => {
            f.first_prefix = crate::config::FramePiece::default();
            f.newline_prefix = crate::config::FramePiece::default();
            f.last_prefix = crate::config::FramePiece::default();
        }
        Step::Answer(3) => {}
        s => return Ok(early(s)),
    }
    Ok(Step::Answer(()))
}

/// 间距：Compact/Sparse。
fn ask_empty_line(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(
        out,
        "提示符间距",
        &["紧凑（无空行）", "稀疏（prompt 之间空一行）"],
    )?;
    match i {
        Step::Answer(i) => {
            cfg.layout.prompt_add_newline = i == 1;
            Ok(Step::Answer(()))
        }
        s => Ok(early(s)),
    }
}

/// 图标：Few/Many。
fn ask_extra_icons(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(out, "图标", &["少（几乎无图标）", "多（完整图标）"])?;
    match i {
        Step::Answer(0) => {
            remove_segment(cfg, "os_icon");
            for key in ["folder", "git", "branch", "time"] {
                cfg.icon_overrides.insert(
                    key.into(),
                    crate::config::IconOverride {
                        all: Some(String::new()),
                        ..Default::default()
                    },
                );
            }
            Ok(Step::Answer(()))
        }
        Step::Answer(1) => Ok(Step::Answer(())),
        s => Ok(early(s)),
    }
}

/// 前缀：Concise/Fluent。
fn ask_prefixes(out: &mut io::Stdout, cfg: &mut Config) -> io::Result<Step<()>> {
    let i = ask_choice(out, "提示符语气", &["简洁（Concise）", "流畅（Fluent）"])?;
    match i {
        Step::Answer(0) => {
            for name in ["vcs", "command_execution_time", "time"] {
                if let Some(s) = cfg.segments.get_mut(name) {
                    s.prefix = None;
                }
            }
        }
        Step::Answer(1) => {
            set_prefix(cfg, "vcs", "on ");
            set_prefix(cfg, "command_execution_time", "took ");
            set_prefix(cfg, "time", "at ");
        }
        s => return Ok(early(s)),
    }
    Ok(Step::Answer(()))
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

/// n 选 1：数字键选择。
fn ask_choice(out: &mut io::Stdout, title: &str, options: &[&str]) -> io::Result<Step<usize>> {
    loop {
        clear(out);
        line(out, title)?;
        line(out, "")?;
        for (i, label) in options.iter().enumerate() {
            line(out, &format!("({})  {}", i + 1, label))?;
        }
        line(out, "")?;
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
