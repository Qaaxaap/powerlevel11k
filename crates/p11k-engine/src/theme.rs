//! 主题绘制：引擎在 prompt 钩子时刻向真实终端输出主题 header。
//!
//! 分工：shell 只给一个占位 prompt（宽度 = 引擎前缀宽度），
//! 让 zle 的几何自洽。引擎不解析 pty 输出的 ANSI，只做字节透传 + 光标定位：
//! - `render_header_cfg`：在占位符透传之前画多行 header（行内容由
//!   [`crate::render::render_header_lines`] 按 KDL 配置生成）。
//! - `render_prompt`：在占位符透传之后，用 `\r` + 前缀覆盖占位符。前缀可见
//!   宽度与占位符恒等，zle 重绘列偏移对齐。

use std::io::{self, Write};

/// prompt 窗口需要的信息，由宣告行（`h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`）解析而来。
pub struct HeaderInfo {
    pub exit_code: Option<i32>,
    pub cwd: String,
    /// 上一条命令耗时(秒,引擎计时:回车 → 本次 precmd);首 prompt 为 0。
    pub exec_seconds: f64,
    /// 后台任务数。
    pub jobs: usize,
    /// zsh 历史命令号(`HISTCMD`),bash/fish 无则 0。
    pub history: usize,
}

/// 当前目录的 git 状态（由 p11k-gitstatus 的 API 计算，随 header 一起画）。
#[derive(Clone)]
pub struct GitStatus {
    /// 本地分支名（detached HEAD 时为空）。
    pub branch: String,
    pub staged: usize,
    pub unstaged: usize,
    pub conflicted: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    pub stashes: usize,
    /// tracking 远端 URL(用于按域名选 vcs 图标,如 github/archlinux)。
    pub remote_url: String,
}

/// 用 KDL 配置渲染 header(多行,行数由配置决定)。逐行清屏 + OSC133A;
/// 最后 `\r\n` 把光标送到输入行(跟随占位符,由 render_prompt 替换)。
pub fn render_header_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b]133;A\x07")?;
    let lines = crate::render::render_header_lines(config, info, vcs, cols);
    for line in lines {
        write!(out, "\r\x1b[K")?;
        out.write_all(line.as_bytes())?;
        write!(out, "\r\n")?;
    }
    Ok(())
}

/// 配置渲染 + 清屏(首次 precmd 用,把 instant header 刷新成真正状态)。
pub fn render_header_cleared_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b[2J\x1b[H")?;
    render_header_cfg(out, cols, config, info, vcs)
}

/// resize 后重画 header:保存输入行光标、上移到 header 行 1、重画、
/// 恢复。上移行数 = 配置 header 行数;不碰输入行(用户可能已在打字)。
pub fn redraw_header_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    let lines = crate::render::render_header_lines(config, info, vcs, cols);
    let n = lines.len().max(1);
    write!(out, "\x1b[s\x1b[{}A", n)?;
    render_header_cfg(out, cols, config, info, vcs)?;
    write!(out, "\x1b[u")?;
    Ok(())
}

/// 覆盖占位 prompt：保存光标、`\r` 回输入行行首画 `prefix`、恢复光标。
///
/// 触发时机：`p` 宣告（zle-line-init）——zle 已渲染完占位 prompt，光标停在
/// 占位符之后。`\e[s` 保存光标、`\r` 回列 0 逐列覆盖占位符字符、`\e[u` 恢复
/// 光标到原位。前缀只覆盖占位符那几列；buffer 非空时（resize 后补画）光标
/// 被恢复到 buffer 末尾，不与 zle 的光标模型冲突。
pub fn render_prompt(out: &mut dyn Write, prefix: &str) -> io::Result<()> {
    write!(out, "\x1b[s\r{prefix}\x1b[u")?;
    // OSC 133 B：prompt 结束标记，告知终端光标已停在输入位置，
    // 关窗不再弹"有程序在运行"的确认框（对齐 p10k _p9k_prompt_suffix）。
    write!(out, "\x1b]133;B\x07")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_prompt_starts_with_cr() {
        let mut out = Vec::new();
        render_prompt(&mut out, "❯ ").unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.starts_with("\x1b[s\r❯ "),
            "应保存光标、回行首画前缀覆盖占位符、再恢复光标"
        );
        assert!(
            s.ends_with("\x1b]133;B\x07"),
            "应以 OSC 133 B 标记结尾（告知终端 prompt 就绪）"
        );
    }
}
