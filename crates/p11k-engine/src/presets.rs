//! 内置预设主题：用 [`Config`] 结构体构造，不硬编码 KDL 字符串。
//!
//! 视觉对齐 p10k 官方 config（`config/p10k-{lean,classic,rainbow,pure}.zsh`）。
//! 颜色变体参数化：classic 段底四档、classic/rainbow/lean 帧四档、lean 256/8 色、
//! pure original/snazzy。向导按用户答案调构造函数并改字段；`--preset`/引擎回退用
//! [`build`] 的默认参数。引擎能力未覆盖的 p10k 参数不搬。

use crate::config::{
    Color, Config, Element, Frame, FramePiece, Prop, Segment, Separators, StateSpec, Style,
};

/// classic 段背景四档（Lightest/Light/Dark/Darkest）。
pub const BG_COLORS: [u8; 4] = [240, 238, 236, 234];
/// classic 子分隔符四档。
pub const SEP_COLORS: [u8; 4] = [248, 246, 244, 242];
/// classic 前缀文字四档。
pub const PREFIX_COLORS: [u8; 4] = [250, 248, 246, 244];
/// 帧四档（classic/rainbow/lean 的 frame 与连接线共用）。
pub const FRAME_COLORS: [u8; 4] = [244, 242, 240, 238];

fn x(n: u8) -> Color {
    Color::Xterm(n)
}

// ---- 段构造函数（四套预设共享）----

fn dir_seg(fg: u8, short: u8, anchor: u8, anchor_bold: bool, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s.props.insert(
        "shorten-strategy".into(),
        Prop::Str("truncate_to_unique".into()),
    );
    s.props.insert("shorten-dir-length".into(), Prop::Int(1));
    s.states.insert(
        "SHORTENED".into(),
        StateSpec {
            style: Style {
                fg: x(short),
                ..Style::default()
            },
            char: None,
        },
    );
    s.states.insert(
        "ANCHOR".into(),
        StateSpec {
            style: Style {
                fg: x(anchor),
                bold: anchor_bold,
                ..Style::default()
            },
            char: None,
        },
    );
    s
}

fn vcs_seg(fg: Option<u8>, clean: u8, modified: u8, untracked: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    if let Some(f) = fg {
        s.style.fg = x(f);
    }
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s.props
        .insert("clean-foreground".into(), Prop::Int(clean as i64));
    s.props
        .insert("modified-foreground".into(), Prop::Int(modified as i64));
    s.props
        .insert("untracked-foreground".into(), Prop::Int(untracked as i64));
    s
}

fn status_seg(ok: u8, err: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s.props.insert("ok-foreground".into(), Prop::Int(ok as i64));
    s.props
        .insert("error-foreground".into(), Prop::Int(err as i64));
    s.props.insert("verbose".into(), Prop::Bool(true));
    s
}

fn exec_seg(fg: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s.props.insert("threshold-seconds".into(), Prop::Int(3));
    s.props.insert("precision".into(), Prop::Int(0));
    s
}

fn jobs_seg(fg: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s.props.insert("verbose".into(), Prop::Bool(false));
    s
}

fn time_seg(fg: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
    s
}

fn os_icon_seg(fg: u8, bg: u8) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    s.style.bg = x(bg);
    s
}

fn prompt_char_seg(ok: u8, err: u8) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(ok);
    s.states.insert(
        "ERROR".into(),
        StateSpec {
            style: Style {
                fg: x(err),
                ..Style::default()
            },
            char: None,
        },
    );
    s
}

// ---- 帧 / 分隔符 ----

/// powerline 箭头帧（classic/rainbow）：首行 `╭─`，中间 `├─`，输入行 `╰─`。
/// 帧级 fg 给所有块上色（对齐 p10k `%<frame_color>F╭─`）。
fn powerline_frame(color: u8) -> Frame {
    let mut f = Frame::default();
    f.style.fg = x(color);
    f.first_prefix = FramePiece {
        text: "╭─".into(),
        style: None,
    };
    f.first_suffix = FramePiece {
        text: "─╮".into(),
        style: None,
    };
    f.newline_prefix = FramePiece {
        text: "├─".into(),
        style: None,
    };
    f.newline_suffix = FramePiece {
        text: "─┤".into(),
        style: None,
    };
    f.last_prefix = FramePiece {
        text: "╰─".into(),
        style: None,
    };
    f
}

fn powerline_separators() -> Separators {
    Separators {
        segment: "\u{e0b0}".into(),
        sub: "\u{e0b1}".into(),
        end: String::new(),
        right_start: "\u{e0b2}".into(),
        right_segment: "\u{e0b2}".into(),
        right_sub: "\u{e0b3}".into(),
        left_tail: String::new(),
        right_tail: String::new(),
        gap: " ".into(),
    }
}

/// 空配置骨架（mode 默认 nerdfont-complete，vcs 远端图标用内置表）。
fn base() -> Config {
    Config::default()
}

// ---- 四套预设 ----

/// lean：单行、无框无箭头、透明底。`colors_8` 给 8 色降级版（对齐 p10k lean-8colors）。
pub fn lean(colors_8: bool) -> Config {
    let (
        dir,
        short,
        anchor,
        anchor_bold,
        vcs_clean,
        vcs_mod,
        vcs_unt,
        st_ok,
        st_err,
        exec,
        jobs,
        pc_ok,
        pc_err,
    ) = if colors_8 {
        (4, 4, 4, false, 2, 3, 2, 2, 1, 3, 1, 2, 1)
    } else {
        (31, 103, 39, true, 76, 178, 39, 70, 160, 101, 70, 76, 196)
    };

    let mut cfg = base();
    cfg.layout.left = vec![vec![Element::Seg("dir".into()), Element::Seg("vcs".into())]];
    cfg.layout.right = vec![vec![
        Element::Seg("status".into()),
        Element::Seg("command_execution_time".into()),
        Element::Seg("background_jobs".into()),
    ]];
    cfg.segments
        .insert("dir".into(), dir_seg(dir, short, anchor, anchor_bold, None));
    cfg.segments.insert(
        "vcs".into(),
        vcs_seg(None, vcs_clean, vcs_mod, vcs_unt, None),
    );
    cfg.segments
        .insert("status".into(), status_seg(st_ok, st_err, None));
    cfg.segments
        .insert("command_execution_time".into(), exec_seg(exec, None));
    cfg.segments
        .insert("background_jobs".into(), jobs_seg(jobs, None));
    cfg.segments
        .insert("prompt_char".into(), prompt_char_seg(pc_ok, pc_err));
    cfg
}

/// classic：多行框 + powerline 箭头，段统一深底。`color` 是四档（1..=4）。
pub fn classic(color: usize) -> Config {
    let i = color.clamp(1, 4) - 1;
    let bg = BG_COLORS[i];
    let frame = FRAME_COLORS[i];

    let mut cfg = base();
    cfg.layout.left = vec![vec![
        Element::Seg("os_icon".into()),
        Element::Seg("dir".into()),
        Element::Seg("vcs".into()),
    ]];
    cfg.layout.right = vec![vec![
        Element::Seg("status".into()),
        Element::Seg("command_execution_time".into()),
        Element::Seg("background_jobs".into()),
    ]];
    cfg.separators = powerline_separators();
    cfg.frame = powerline_frame(frame);
    cfg.segments.insert("os_icon".into(), os_icon_seg(255, bg));
    cfg.segments
        .insert("dir".into(), dir_seg(31, 103, 39, true, Some(bg)));
    cfg.segments
        .insert("vcs".into(), vcs_seg(None, 76, 178, 76, Some(bg)));
    cfg.segments
        .insert("status".into(), status_seg(70, 160, Some(bg)));
    cfg.segments
        .insert("command_execution_time".into(), exec_seg(248, Some(bg)));
    cfg.segments
        .insert("background_jobs".into(), jobs_seg(37, Some(bg)));
    cfg.segments
        .insert("prompt_char".into(), prompt_char_seg(76, 196));
    cfg
}

/// rainbow：classic 结构但每段彩色底（对齐 p10k rainbow）。`color` 是帧四档。
pub fn rainbow(color: usize) -> Config {
    let i = color.clamp(1, 4) - 1;
    let frame = FRAME_COLORS[i];

    let mut cfg = base();
    cfg.layout.left = vec![vec![
        Element::Seg("os_icon".into()),
        Element::Seg("dir".into()),
        Element::Seg("vcs".into()),
    ]];
    cfg.layout.right = vec![vec![
        Element::Seg("status".into()),
        Element::Seg("command_execution_time".into()),
        Element::Seg("background_jobs".into()),
    ]];
    cfg.separators = powerline_separators();
    cfg.frame = powerline_frame(frame);
    // 彩虹底：os 白、dir 蓝、vcs 绿、status 黑、exec 黄、jobs 黑。
    cfg.segments.insert("os_icon".into(), os_icon_seg(232, 7));
    cfg.segments
        .insert("dir".into(), dir_seg(254, 250, 255, true, Some(4)));
    cfg.segments
        .insert("vcs".into(), vcs_seg(None, 2, 3, 2, Some(2)));
    cfg.segments
        .insert("status".into(), status_seg(2, 3, Some(0)));
    cfg.segments
        .insert("command_execution_time".into(), exec_seg(0, Some(3)));
    cfg.segments
        .insert("background_jobs".into(), jobs_seg(6, Some(0)));
    cfg.segments
        .insert("prompt_char".into(), prompt_char_seg(76, 196));
    cfg
}

/// pure：单行极简，无框无箭头，透明底。`snazzy` 选 Snazzy 真彩，否则 Original。
pub fn pure(snazzy: bool) -> Config {
    // 对齐 p10k pure 的 pure_original / pure_snazzy 调色板。
    let (grey, red, yellow, blue, magenta) = if snazzy {
        (
            x(242),
            Color::Rgb(255, 92, 87),
            Color::Rgb(243, 249, 157),
            Color::Rgb(87, 199, 255),
            Color::Rgb(255, 106, 193),
        )
    } else {
        (x(242), x(1), x(3), x(4), x(5))
    };

    let mut cfg = base();
    cfg.layout.left = vec![vec![
        Element::Seg("context".into()),
        Element::Seg("dir".into()),
        Element::Seg("vcs".into()),
        Element::Seg("command_execution_time".into()),
    ]];
    // p10k pure 无右栏。
    cfg.layout.right = Vec::new();

    let mut context = Segment::default();
    context.style.fg = grey.clone();
    cfg.segments.insert("context".into(), context);

    let mut dir = dir_seg(0, 0, 0, false, None);
    dir.style.fg = blue.clone();
    dir.states.get_mut("SHORTENED").unwrap().style.fg = grey.clone();
    dir.states.get_mut("ANCHOR").unwrap().style.fg = blue.clone();
    cfg.segments.insert("dir".into(), dir);

    let mut vcs = Segment::default();
    vcs.style.fg = grey.clone();
    cfg.segments.insert("vcs".into(), vcs);

    let mut exec = exec_seg(0, None);
    exec.style.fg = yellow.clone();
    cfg.segments.insert("command_execution_time".into(), exec);

    let mut pc = prompt_char_seg(0, 0);
    pc.style.fg = magenta.clone();
    pc.states.get_mut("ERROR").unwrap().style.fg = red.clone();
    cfg.segments.insert("prompt_char".into(), pc);
    cfg
}

// ---- 预设清单 ----

/// 预设身份（名字/标题/默认参数构建）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    Lean,
    Classic,
    Rainbow,
    Pure,
}

impl PresetKind {
    pub fn name(self) -> &'static str {
        match self {
            PresetKind::Lean => "lean",
            PresetKind::Classic => "classic",
            PresetKind::Rainbow => "rainbow",
            PresetKind::Pure => "pure",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            PresetKind::Lean => "Lean —— 紧凑，无框无箭头，默认风格",
            PresetKind::Classic => "Classic —— 多行框 + powerline 箭头，经典配色",
            PresetKind::Rainbow => "Rainbow —— classic 结构，每段彩色底",
            PresetKind::Pure => "Pure —— 单行极简，还原 p10k pure",
        }
    }

    /// 全部预设，顺序即向导展示顺序。
    pub const ALL: [PresetKind; 4] = [
        PresetKind::Lean,
        PresetKind::Classic,
        PresetKind::Rainbow,
        PresetKind::Pure,
    ];
}

/// 按名字查预设身份；未知返回 None。
pub fn by_name(name: &str) -> Option<PresetKind> {
    PresetKind::ALL.iter().copied().find(|k| k.name() == name)
}

/// 用默认参数构建（`--preset` / 引擎回退用）。
pub fn build(kind: PresetKind) -> Config {
    match kind {
        PresetKind::Lean => lean(false),
        PresetKind::Classic => classic(2),
        PresetKind::Rainbow => rainbow(2),
        PresetKind::Pure => pure(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_builds_and_serializes() {
        for kind in PresetKind::ALL {
            let cfg = build(kind);
            let out = cfg.to_kdl();
            let reparsed = crate::config::Config::parse(&out).unwrap();
            assert_eq!(cfg, reparsed, "{:?} round-trip 失败", kind.name());
        }
    }

    #[test]
    fn classic_color_shades_differ() {
        let light = classic(1);
        let dark = classic(4);
        assert_ne!(
            light.segment("dir").style.bg,
            dark.segment("dir").style.bg,
            "四档段底色应不同"
        );
        assert_ne!(light.frame.style.fg, dark.frame.style.fg);
    }

    #[test]
    fn pure_snazzy_differs_from_original() {
        assert_ne!(pure(false), pure(true));
    }
}
