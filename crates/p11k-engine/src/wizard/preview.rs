//! Sample prompt state and the rendered preview lines the questions show.

use crate::config::Config;
use crate::model::HeaderInfo;

/// Current directory + sample state that makes the exec/status segments render content.
pub(super) fn sample_info() -> HeaderInfo {
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

pub(super) fn header_on(cfg: &Config) -> bool {
    !(cfg.layout.left.is_empty() && cfg.layout.right.is_empty())
}

/// Sample git status: makes the preview's vcs segment look like a real prompt (p10k's wizard uses fake data too).
pub(super) fn sample_vcs() -> crate::model::GitStatus {
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
pub(super) fn preview_lines(cfg: &Config) -> Vec<String> {
    let cols = crate::state::tty_size()
        .map(|(_, c, ..)| c as usize)
        .unwrap_or(80);
    let vcs = sample_vcs();
    let mut lines = crate::render::render_header_lines(cfg, &sample_info(), Some(&vcs), cols);
    // Input line: the prompt_char prefix the engine draws (the shell's buffer follows).
    lines.push(crate::render::input_prefix(cfg, None, crate::model::ViMode::Unreported).text);
    lines
}
