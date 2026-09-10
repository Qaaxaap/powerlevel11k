//! 内置预设主题：用 [`Config`] 结构体构造，不硬编码 KDL 字符串。
//!
//! 视觉对齐 p10k 官方 config（`config/p10k-{lean,classic,rainbow,pure}.zsh`）。
//! 颜色变体参数化：classic 段底四档、classic/rainbow/lean 帧四档、lean 256/8 色、
//! pure original/snazzy。向导按用户答案调构造函数并改字段；`--preset`/引擎回退用
//! [`build`] 的默认参数。引擎能力未覆盖的 p10k 参数不搬。

use crate::config::{
    Color, Config, Element, Frame, FramePiece, Prop, Segment, Separators, StateSpec, Style,
};
use crate::i18n::msgid;

/// classic 段背景四档（Lightest/Light/Dark/Darkest）。
pub const BG_COLORS: [u8; 4] = [240, 238, 236, 234];
/// classic `sub` 细线四档（p10k wizard 的 `sep_color`）。
pub const SEP_COLORS: [u8; 4] = [248, 246, 244, 242];
/// 帧四档（classic/rainbow 的 frame 线条颜色）。
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

fn vcs_seg(
    fg: Option<u8>,
    clean: u8,
    modified: u8,
    conflicted: u8,
    untracked: u8,
    meta: u8,
    bg: Option<u8>,
) -> Segment {
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
    // conflicted 在 p10k 的格式化函数里是独立的 `local conflicted`,
    // 与 staged/unstaged 用的 modified 不同色（classic/lean 红 196）。
    s.props
        .insert("conflicted-foreground".into(), Prop::Int(conflicted as i64));
    s.props
        .insert("untracked-foreground".into(), Prop::Int(untracked as i64));
    // meta 是 p10k 格式化函数里的 `local meta`:detached HEAD 的 `@`、
    // 标签的 `#` 用它（classic/lean 246 灰、rainbow 7 白）。
    s.props
        .insert("meta-foreground".into(), Prop::Int(meta as i64));
    // 分支名超 32 字符 → 前 12 + … + 后 12（对齐 p10k 各 config 里硬编码的
    // `(( $#branch > 32 )) && branch[13,-13]="…"`）。
    s.props.insert("shorten-length".into(), Prop::Int(12));
    s.props.insert("shorten-min-length".into(), Prop::Int(32));
    s.props.insert(
        "shorten-strategy".into(),
        Prop::Str("truncate_middle".into()),
    );
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

fn os_icon_seg(fg: u8, bg: Option<u8>) -> Segment {
    let mut s = Segment::default();
    s.style.fg = x(fg);
    if let Some(b) = bg {
        s.style.bg = x(b);
    }
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

/// `sub` 细线的前景色（p10k wizard 的 `sep_color`，随 classic 颜色档变）。
/// rainbow 不设（p10k 的 rainbow subsep 不嵌颜色，跟段底渐变）。
fn powerline_separators(sub_foreground: Option<u8>) -> Separators {
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
        gap_foreground: None,
        sub_foreground: sub_foreground.map(x),
        right_sub_foreground: sub_foreground.map(x),
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
        vcs_conf,
        vcs_unt,
        vcs_meta,
        st_ok,
        st_err,
        exec,
        jobs,
        pc_ok,
        pc_err,
    ) = if colors_8 {
        // p10k lean-8colors:clean 2 / modified 3 / untracked 4(蓝) / conflicted 1,
        // meta 用默认前景(%f,这里给 0)。
        (4, 4, 4, false, 2, 3, 1, 4, 0, 2, 1, 3, 1, 2, 1)
    } else {
        // p10k lean:clean 76 / modified 178 / untracked 39(蓝) / conflicted 196,
        // meta 246(灰)。
        (
            31, 103, 39, true, 76, 178, 196, 39, 246, 70, 160, 101, 70, 76, 196,
        )
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
        vcs_seg(None, vcs_clean, vcs_mod, vcs_conf, vcs_unt, vcs_meta, None),
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
    cfg.separators = powerline_separators(Some(SEP_COLORS[i]));
    cfg.frame = powerline_frame(frame);
    // 底色逐段写（p10k classic 靠全局 `POWERLEVEL9K_BACKGROUND=238` 继承；
    // p11k 的 `defaults.bg` 同时是帧字符/text 的回退终点，写在那里会把 `╭─`
    // `╰─` 这些**帧**也染上底色，而 p10k 的帧是透明的）。新增段（wizard 的
    // time 等）由 wizard 按当前风格补上同样的底色。
    cfg.segments
        .insert("os_icon".into(), os_icon_seg(255, Some(bg)));
    cfg.segments
        .insert("dir".into(), dir_seg(31, 103, 39, true, Some(bg)));
    // untracked=39(蓝):p10k classic/lean 的 vcs 格式化函数里 `local
    // untracked='%39F'`,与 POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND(76,只给
    // vcs_info 回退路径用)不是一回事,实际渲染出来是蓝色。
    // conflicted=196(红):p10k classic `local conflicted='%196F'`。
    cfg.segments
        .insert("vcs".into(), vcs_seg(None, 76, 178, 196, 39, 246, Some(bg)));
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
    // rainbow 的细线不嵌颜色（p10k 同款，跟段底渐变）。
    cfg.separators = powerline_separators(None);
    cfg.frame = powerline_frame(frame);
    // 彩虹底：os 白、dir 蓝、vcs 绿、status 黑、exec 黄、jobs 黑。
    cfg.segments
        .insert("os_icon".into(), os_icon_seg(232, Some(7)));
    cfg.segments
        .insert("dir".into(), dir_seg(254, 250, 255, true, Some(4)));
    // p10k rainbow 的 vcs 格式化函数:clean/modified/untracked 全是 `%0F`(黑,
    // 段底是绿 2),conflicted `%1F`(红)。之前传 2/3/2,绿底绿字把分支和 ?N 画没了。
    cfg.segments
        .insert("vcs".into(), vcs_seg(None, 0, 0, 1, 0, 7, Some(2)));
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

    /// 向导里的风格名（对齐 p10k `ask_style` 的说法）。
    pub fn title(self) -> &'static str {
        match self {
            PresetKind::Lean => msgid("Lean."),
            PresetKind::Classic => msgid("Classic."),
            PresetKind::Rainbow => msgid("Rainbow."),
            PresetKind::Pure => msgid("Pure."),
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
    fn classic_colors_every_segment_and_keeps_frame_transparent() {
        // p10k classic 的底色来自全局 `POWERLEVEL9K_BACKGROUND`，段继承它、帧字符
        // 仍透明。p11k 的 `defaults.bg` 同时是帧/text 的回退终点，所以底色必须逐段
        // 写：写进 defaults 会把 `╭─`/`╰─` 也染上底（实测过）。
        for (color, want) in [(1usize, 240u8), (2, 238), (3, 236), (4, 234)] {
            let c = classic(color);
            for seg in ["os_icon", "dir", "vcs", "status", "command_execution_time"] {
                assert_eq!(
                    c.segment(seg).style.bg,
                    x(want),
                    "第 {color} 档 {seg} 段底色应为 {want}"
                );
            }
            assert_eq!(
                c.defaults.bg,
                crate::config::Color::Default,
                "defaults 不该有 bg（否则帧字符会被染色）"
            );
            // 帧块样式：只有前景色，没有背景。
            let piece = c.frame_piece_style(&c.frame.first_prefix);
            assert_eq!(piece.bg, crate::config::Color::Default, "帧字符必须透明");
        }
        // lean/pure/rainbow 全局透明；rainbow 每段各自底色。
        assert_eq!(lean(false).defaults.bg, crate::config::Color::Default);
        assert_eq!(pure(false).defaults.bg, crate::config::Color::Default);
        assert_eq!(rainbow(1).defaults.bg, crate::config::Color::Default);
        assert_eq!(
            rainbow(1).segment("os_icon").style.bg,
            x(7),
            "rainbow 每段各有底色"
        );
    }

    #[test]
    fn pure_snazzy_differs_from_original() {
        assert_ne!(pure(false), pure(true));
    }

    #[test]
    fn wizard_template_kdl_writes_per_segment_background() {
        // wizard 生成的配置：底色逐段写、defaults 里没有 bg（帧才不会染色）。
        let kdl = classic(2).to_kdl();
        let dir_line = kdl
            .lines()
            .find(|l| l.trim_start().starts_with("dir "))
            .expect("应有 dir 段");
        assert!(
            dir_line.contains("bg=238"),
            "classic 第 2 档的段底色应为 238: {dir_line}"
        );
        assert!(
            !kdl.contains("defaults"),
            "不该写 defaults（会把帧字符也染色）:\n{kdl}"
        );
        // rainbow 相反：每段各有底色，time 段自己带 7。
        let rb = rainbow(1).to_kdl();
        assert!(rb.contains("bg=7"), "rainbow 的 os_icon 应有底色:\n{rb}");
    }

    #[test]
    fn vcs_colors_match_p10k_generated_configs() {
        use crate::config::Prop;
        let int = |c: &Config, seg: &str, key: &str| match c.segment(seg).prop(key) {
            Some(Prop::Int(n)) => Some(*n),
            _ => None,
        };
        // p10k lean/classic 的 vcs 格式化函数:clean %76F、modified %178F、
        // untracked %39F(蓝)、conflicted %196F(红)。untracked 与
        // POWERLEVEL9K_VCS_UNTRACKED_FOREGROUND(76,只给 vcs_info 回退路径)不同。
        for c in [lean(false), classic(1)] {
            assert_eq!(int(&c, "vcs", "clean-foreground"), Some(76));
            assert_eq!(int(&c, "vcs", "modified-foreground"), Some(178));
            assert_eq!(int(&c, "vcs", "untracked-foreground"), Some(39));
            assert_eq!(int(&c, "vcs", "conflicted-foreground"), Some(196));
        }
        // p10k lean-8colors:untracked 4(蓝)、conflicted 1(红)。
        let l8 = lean(true);
        assert_eq!(int(&l8, "vcs", "untracked-foreground"), Some(4));
        assert_eq!(int(&l8, "vcs", "conflicted-foreground"), Some(1));
        // p10k rainbow:vcs 段底是绿 2,文字全是黑 0(不能与底色同色,否则画没了)。
        let rb = rainbow(1);
        let vcs = rb.segment("vcs");
        assert_eq!(vcs.style.bg, x(2));
        assert_eq!(int(&rb, "vcs", "clean-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "modified-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "untracked-foreground"), Some(0));
        assert_eq!(int(&rb, "vcs", "conflicted-foreground"), Some(1));
    }

    #[test]
    fn vcs_branch_shortening_matches_p10k_hardcoded_rule() {
        // p10k 各 config 里写死 `(( $#branch > 32 )) && branch[13,-13]="…"`。
        use crate::config::Prop;
        let c = classic(1);
        let p = |k: &str| c.segment("vcs").prop(k).cloned();
        assert_eq!(p("shorten-length"), Some(Prop::Int(12)));
        assert_eq!(p("shorten-min-length"), Some(Prop::Int(32)));
        assert_eq!(
            p("shorten-strategy"),
            Some(Prop::Str("truncate_middle".into()))
        );
    }
}
