//! 内置预设主题(内嵌 KDL,单二进制可枚举 —— 配置向导据此列风格)。
//!
//! 视觉参照 p10k 四套官方 config(`config/p10k-{lean,classic,rainbow,pure}.zsh`)
//! 翻译成 KDL;引擎能力未覆盖的参数不搬。向导/`--preset` 从 [`PRESETS`] 选取。

/// 一套预设:机器名(`--preset`/向导用)、一句话标题、KDL 源。
pub struct Preset {
    pub name: &'static str,
    /// 展示给用户的一句话(向导选风格时用;引擎本体不读)。
    #[allow(dead_code)]
    pub title: &'static str,
    pub kdl: &'static str,
}

pub const LEAN: &str = r#"
layout {
    left {
        line { dir; vcs }
    }
    right {
        line { status; command_execution_time; background_jobs }
    }
    add-newline #true
}

segments {
    dir fg=39 shorten-strategy="truncate_to_unique" shorten-dir-length=1 {
        state SHORTENED fg=103
        state ANCHOR fg=39 bold=#true
    }
    vcs clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status ok-foreground=70 error-foreground=160 verbose=#true
    command_execution_time threshold-seconds=3 precision=0 fg=101
    background_jobs fg=70 verbose=#false
    prompt_char fg=76 {
        state ERROR fg=196
    }
}
"#;

/// classic:p10k classic —— 多行框 + powerline 箭头,段统一深底。
pub const CLASSIC: &str = r#"
layout {
    left {
        line { os_icon; dir; vcs }
    }
    right {
        line { status; command_execution_time; background_jobs }
    }
}

separators {
    segment "\u{e0b0}"
    sub     "\u{e0b1}"
    end     ""
    right-start "\u{e0b2}"
    right-segment "\u{e0b2}"
    right-sub    "\u{e0b3}"
    gap     " "
}

frame {
    first-prefix "╭─"
    first-suffix "─╮"
    newline-prefix "├─"
    newline-suffix "─┤"
    last-prefix "╰─"
}

segments {
    os_icon bg=31 fg=231
    dir bg=31 fg=231 shorten-strategy="truncate_to_unique" shorten-dir-length=1 {
        state SHORTENED fg=117
        state ANCHOR fg=231 bold=#true
    }
    vcs bg=237 fg=231 clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status bg=237 fg=231 ok-foreground=70 error-foreground=160 verbose=#true
    command_execution_time bg=237 fg=110 threshold-seconds=3 precision=0
    background_jobs bg=237 fg=110 verbose=#false
    prompt_char bg=237 fg=76 {
        state ERROR fg=196
    }
}
"#;

/// rainbow:p10k rainbow —— classic 结构但每段异色底(彩虹)。
pub const RAINBOW: &str = r#"
layout {
    left {
        line { os_icon; dir; vcs }
    }
    right {
        line { status; command_execution_time; background_jobs }
    }
}

separators {
    segment "\u{e0b0}"
    sub     "\u{e0b1}"
    end     ""
    right-start "\u{e0b2}"
    right-segment "\u{e0b2}"
    right-sub    "\u{e0b3}"
    gap     " "
}

frame {
    first-prefix "╭─"
    first-suffix "─╮"
    newline-prefix "├─"
    newline-suffix "─┤"
    last-prefix "╰─"
}

segments {
    os_icon bg=89 fg=231
    dir bg=31 fg=231 shorten-strategy="truncate_to_unique" shorten-dir-length=1 {
        state SHORTENED fg=117
        state ANCHOR fg=231 bold=#true
    }
    vcs bg=129 fg=231 clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status bg=29 fg=231 ok-foreground=70 error-foreground=160 verbose=#true
    command_execution_time bg=58 fg=231 threshold-seconds=3 precision=0
    background_jobs bg=52 fg=231 verbose=#false
    prompt_char bg=89 fg=76 {
        state ERROR fg=196
    }
}
"#;

/// pure:p10k pure —— 单行极简,无框无箭头,空格分隔。
pub const PURE: &str = r#"
layout {
    left {
        line { prompt_char; dir; vcs }
    }
    right {
        line { status; time }
    }
}

segments {
    dir fg=15 shorten-strategy="truncate_to_unique" shorten-dir-length=1 {
        state SHORTENED fg=244
        state ANCHOR fg=15 bold=#true
    }
    vcs fg=39 clean-foreground=76 modified-foreground=178 untracked-foreground=39
    status fg=70 ok-foreground=70 error-foreground=160 verbose=#true
    time fg=244
    prompt_char fg=76 {
        state ERROR fg=196
    }
}
"#;

/// 全部预设,顺序即向导展示顺序。
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "lean",
        title: "Lean —— 紧凑,无框无箭头,默认风格",
        kdl: LEAN,
    },
    Preset {
        name: "classic",
        title: "Classic —— 多行框 + powerline 箭头,经典配色",
        kdl: CLASSIC,
    },
    Preset {
        name: "rainbow",
        title: "Rainbow —— classic 结构,每段彩色底",
        kdl: RAINBOW,
    },
    Preset {
        name: "pure",
        title: "Pure —— 单行极简,还原 p10k pure",
        kdl: PURE,
    },
];

/// 按名字查预设(`lean`/`classic`/`rainbow`/`pure`);未知返回 None。
pub fn by_name(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}
