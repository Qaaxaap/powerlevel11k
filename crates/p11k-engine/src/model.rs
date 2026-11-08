//! Prompt model: the state the renderer draws from and the shell/engine produce.
//!
//! `HeaderInfo` is parsed from the announcement line and from the git worker's result. Keeping
//! it out of the terminal writer lets the renderer depend on the model, and the writer only
//! writes.

/// Information the prompt window needs, parsed from the announcement line
/// (`h\t<exit>\t<cwd>[\t<jobs>][\t<history>][\t<pipestatus>]`).
pub struct HeaderInfo {
    pub exit_code: Option<i32>,
    pub cwd: String,
    /// Duration of the previous command in seconds (timed by the engine: Enter → this precmd); 0 for the first prompt.
    pub exec_seconds: f64,
    /// Milliseconds that the shell measured for the previous command. Fish uses
    /// `CMD_DURATION`. Zsh uses preexec and precmd. Bash uses DEBUG and PROMPT_COMMAND.
    /// Pwsh uses the history. The value is `None` when the shell has no value. The engine
    /// then falls back to its own Enter → precmd timing.
    pub exec_ms: Option<u64>,
    pub jobs: usize,
    /// Command number: zsh sends `HISTCMD`, pwsh sends the last history entry's
    /// `HistoryInfo.Id` (also a monotonically increasing session number); bash/fish
    /// have no such column and use 0.
    pub history: usize,
    /// Exit code of every command in the pipeline (`$pipestatus` in zsh/fish,
    /// `$PIPESTATUS` in bash), comma-separated on the wire. Empty when the shell has
    /// none (pwsh). Drives the extended status states, see `prompt_state`.
    pub pipestatus: Vec<i32>,
}

/// git status of the current directory (computed by the p11k-gitstatus API, drawn with the header).
#[derive(Clone)]
pub struct GitStatus {
    /// Local branch name (empty on detached HEAD).
    pub branch: String,
    /// Commit oid of HEAD (p10k `SHOW_CHANGESET` takes its first N digits).
    pub commit: String,
    pub staged: usize,
    pub unstaged: usize,
    pub conflicted: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    /// Ahead/behind of the push remote (p10k `VCS_STATUS_PUSH_COMMITS_{AHEAD,BEHIND}`).
    pub push_ahead: usize,
    pub push_behind: usize,
    pub stashes: usize,
    /// Operation in progress (merge/rebase/..., p10k `VCS_STATUS_ACTION`); empty = none.
    pub action: String,
    /// Tag HEAD points at (p10k `VCS_STATUS_TAG`); shown as `#tag` when there is no local branch.
    pub tag: String,
    /// upstream branch name (p10k `VCS_STATUS_REMOTE_BRANCH`); shown as `local:remote`
    /// when it differs from the local branch name.
    pub remote_branch: String,
    /// Title of the last commit (p10k `VCS_STATUS_COMMIT_SUMMARY`): shows the `wip`
    /// marker when it contains wip/WIP.
    pub commit_summary: String,
    /// Number of index entries (p10k `VCS_STATUS_INDEX_SIZE`): comparing it against
    /// `max-index-size-dirty` reveals whether the dirty stats were skipped (when skipped the
    /// unstaged count is unknown and `─` is drawn).
    pub index_size: usize,
    /// tracking remote URL (used to pick the vcs icon by domain, e.g. github/archlinux).
    pub remote_url: String,
}
