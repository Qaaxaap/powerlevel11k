//! 主题绘制：引擎在 prompt 钩子时刻向真实终端输出主题 header。
//!
//! 分工：shell 给一个多行占位 prompt，让 shell 的几何把 header 行也算进去。
//! 引擎不解析 pty 输出的 ANSI，只做字节透传 + 光标定位：
//! - `prompt_start`：`h` 宣告时先发 OSC133A（早于 shell 的多行占位）。
//! - `redraw_header_cfg`：占位透传后上移 header 行数回填真实 header（行内容
//!   由 [`crate::render::render_header_lines`] 按 KDL 配置生成），并恢复光标。
//! - `render_prompt`：用 `\r` + 前缀覆盖占位符。前缀可见宽度与占位符恒等，
//!   shell 重绘列偏移对齐。
//! - `render_header_cfg`：仅 instant header（引擎尚未 spawn shell）用，直接画
//!   完整 header + 前缀。

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
    /// HEAD 的 commit oid（p10k `SHOW_CHANGESET` 取它的前 N 位）。
    pub commit: String,
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

/// OSC 133 A：prompt 开始标记(shell integration)。占位协议下由引擎在 `h`
/// 宣告时单独输出(早于 shell 渲染的多行占位),回填/重画 header 时不再重复。
pub fn prompt_start(out: &mut dyn Write) -> io::Result<()> {
    write!(out, "\x1b]133;A\x07")
}

/// 逐行输出 header 内容:每行 `\r\x1b[K` 清行 + 内容 + `\r\n`,把光标推进到
/// 下一行行首。不含 OSC133A——它由 [`prompt_start`] 在占位之前单独发。
fn header_body(out: &mut dyn Write, lines: &[String]) -> io::Result<()> {
    for line in lines {
        write!(out, "\r\x1b[K")?;
        out.write_all(line.as_bytes())?;
        write!(out, "\r\n")?;
    }
    Ok(())
}

/// 用 KDL 配置渲染完整 header(多行,行数由配置决定):OSC133A + 逐行内容。
/// 仅 instant header(引擎尚未 spawn shell、占位还没出现)用;正常 prompt
/// 周期改走「占位先行 + redraw_header_cfg 回填」。
pub fn render_header_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    prompt_start(out)?;
    let lines = crate::render::render_header_lines(config, info, vcs, cols);
    header_body(out, &lines)
}

/// 回填/重画 header:保存输入行光标、上移到 header 行 1、逐行重画、恢复。
/// 上移行数 = 配置 header 行数;不碰输入行。
/// 占位协议下,shell 已先渲染出 header 行数的空行占位,这里把真实内容覆盖上去,
/// 最后 `\x1b[u` 回到输入行光标。resize 重画与 `p` 宣告后的首次回填复用同一段。
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
    header_body(out, &lines)?;
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
