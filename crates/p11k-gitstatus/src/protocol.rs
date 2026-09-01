//! gitstatus wire protocol.
//!
//! Requests flow on the daemon's stdin, responses on stdout, as byte
//! streams. Every message ends with MSG_SEP; fields within a message are
//! separated by FIELD_SEP. There is no length prefix and no escaping layer
//! (message contents never contain the separators, see [`safe_print`]).
//!
//! SafePrint behavior is pinned to the official prebuilt binaries
//! (v1.5.4 linux-x86_64, Alpine musl + GCC 9.3.0 static-pie): control
//! characters map to `'?'`; bytes >0x7F pass through unchanged. The original
//! source's `c > 127 || std::isprint(c)` is UB for negative c, so local
//! self-built binaries can differ — differential tests must use the official
//! prebuilt binaries.

/// Field separator: ASCII 31 (US).
pub const FIELD_SEP: u8 = 0x1f;

/// Message separator / terminator: ASCII 30 (RS).
pub const MSG_SEP: u8 = 0x1e;

/// Special handshake id. The zsh side sends `\x1ehello\x1f\x1e` and the
/// daemon replies `\x1ehello\x1f0\x1e` (a "non-repo" response with id hello).
pub const HELLO_ID: &str = "}hello";

/// One request, on the wire (terminated by MSG_SEP):
///
/// ```text
/// <id>\x1f[:]<dir>\x1f<diff>\x1e
/// ```
///
/// - **id**: arbitrary bytes; the response's first field must echo it
///   verbatim. `Vec<u8>` because it need not be UTF-8.
/// - **dir**: absolute path. A leading `:` means the rest is a direct
///   GIT_DIR (from_dotgit: open that .git directly, no upward search).
///   May be empty (handshake). `Vec<u8>`: paths have no encoding contract.
/// - **diff**: optional third field, single byte `'0'` or `'1'`.
///   `'1'` = skip index comparison. Note the original's internal `bool diff`
///   has the opposite meaning of the wire byte (wire `'0'` → internal true).
pub struct Request {
    /// Echoed verbatim as the response's first field.
    pub id: Vec<u8>,
    /// Absolute directory path; without the leading `:` when `dir_is_gitdir`.
    pub dir: Vec<u8>,
    /// Whether `dir` was given with a `:` prefix (use `dir` as GIT_DIR).
    pub dir_is_gitdir: bool,
    /// Wire third field `'1'` = skip index comparison.
    pub skip_index: bool,
}

/// Parse one request (without the trailing MSG_SEP).
///
/// Malformed requests panic: the original `VERIFY` aborts on failure, which
/// terminates the daemon — the same behavior is kept.
pub fn parse_request(bytes: &[u8]) -> Request {
    if bytes.is_empty() {
        panic!("malformed request: empty message");
    }
    let mut parts = bytes.split(|&b| b == FIELD_SEP);
    let id = parts.next().expect("split yields at least one part");
    let dir_field = parts.next().expect("malformed request: missing dir field");
    let (dir, dir_is_gitdir) = match dir_field.split_first() {
        Some((b':', rest)) => (rest, true),
        _ => (dir_field, false),
    };
    let skip_index = match parts.next() {
        None => false,
        Some(b"1") => true,
        Some(b"0") => false,
        Some(_) => panic!("malformed request: bad diff field"),
    };
    assert!(parts.next().is_none(), "malformed request: too many fields");
    Request {
        id: id.to_vec(),
        dir: dir.to_vec(),
        dir_is_gitdir,
        skip_index,
    }
}

/// Fixed 0-based indices of the response data fields.
///
/// Wire layout: first field is **id**, second is **1/0** (1 = repo).
/// Only when that flag is 1 do the [`field::COUNT`] data fields follow,
/// joined by FIELD_SEP, message terminated by MSG_SEP. Field order is fixed
/// (the Print order of gitstatus.cc:86-177).
pub mod field {
    /// Absolute workdir path.
    pub const WORKDIR: usize = 0;
    /// 40-hex SHA of the current commit.
    pub const COMMIT: usize = 1;
    /// Local branch name (empty on detached HEAD).
    pub const LOCAL_BRANCH: usize = 2;
    /// Upstream branch name.
    pub const REMOTE_BRANCH: usize = 3;
    /// Upstream remote name.
    pub const REMOTE_NAME: usize = 4;
    /// Upstream remote URL.
    pub const REMOTE_URL: usize = 5;
    /// In-progress operation (rebase/merge/...), empty = none.
    pub const ACTION: usize = 6;
    /// Number of index entries (decimal string).
    pub const INDEX_SIZE: usize = 7;
    /// Staged change count (capped by -s).
    pub const NUM_STAGED: usize = 8;
    /// Unstaged change count (capped by -u).
    pub const NUM_UNSTAGED: usize = 9;
    /// Conflicted file count (capped by -c).
    pub const NUM_CONFLICTED: usize = 10;
    /// Untracked file count (capped by -d).
    pub const NUM_UNTRACKED: usize = 11;
    /// Commits ahead of upstream.
    pub const COMMITS_AHEAD: usize = 12;
    /// Commits behind upstream.
    pub const COMMITS_BEHIND: usize = 13;
    /// Stash count.
    pub const STASHES: usize = 14;
    /// Closest tag on the current commit (empty if none).
    pub const TAG: usize = 15;
    /// Unstaged deleted file count.
    pub const NUM_UNSTAGED_DELETED: usize = 16;
    /// Staged new file count.
    pub const NUM_STAGED_NEW: usize = 17;
    /// Staged deleted file count.
    pub const NUM_STAGED_DELETED: usize = 18;
    /// Push remote name.
    pub const PUSH_REMOTE_NAME: usize = 19;
    /// Push remote URL.
    pub const PUSH_REMOTE_URL: usize = 20;
    /// Push commits ahead.
    pub const PUSH_COMMITS_AHEAD: usize = 21;
    /// Push commits behind.
    pub const PUSH_COMMITS_BEHIND: usize = 22;
    /// Number of files with the skip-worktree bit.
    pub const NUM_SKIP_WORKTREE: usize = 23;
    /// Number of files with the assume-unchanged bit.
    pub const NUM_ASSUME_UNCHANGED: usize = 24;
    /// Commit message encoding (e.g. "utf-8").
    pub const COMMIT_ENCODING: usize = 25;
    /// Commit message summary (truncated by -z, SafePrint-escaped).
    pub const COMMIT_SUMMARY: usize = 26;
    /// Total data field count. Non-repo responses (flag 0) carry none.
    pub const COUNT: usize = 27;
}

/// A complete response (repo / non-repo).
pub struct Response {
    /// The request id, echoed verbatim (SafePrint applied on write).
    pub id: Vec<u8>,
    /// true = repo (fields valid); false = non-repo (no data fields).
    pub is_repo: bool,
    /// Valid only when `is_repo`; always [`field::COUNT`] long. Numeric
    /// fields hold decimal strings. `Vec<u8>`: names/paths need not be UTF-8.
    pub fields: Option<[Vec<u8>; field::COUNT]>,
}

/// Serialize a [`Response`] to wire bytes (including the trailing MSG_SEP).
///
/// ```text
/// safe_print(id) \x1f 1 [\x1f safe_print(f1) ... \x1f safe_print(f27)] \x1e
/// ```
///
/// A non-repo response is just `safe_print(id) \x1f 0 \x1e`.
pub fn serialize_response(r: &Response) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend(safe_print(&r.id));
    out.push(FIELD_SEP);
    out.push(if r.is_repo { b'1' } else { b'0' });
    if r.is_repo {
        for f in r.fields.as_ref().expect("is_repo implies fields") {
            out.push(FIELD_SEP);
            out.extend(safe_print(f));
        }
    }
    out.push(MSG_SEP);
    out
}

/// SafePrint escaping: printable ASCII (0x20..=0x7E) and bytes >0x7F pass
/// through; everything else (control chars <0x20, DEL 0x7F) becomes `'?'`.
/// Byte-wise, not Unicode-aware.
pub fn safe_print(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&c| if c >= 0x20 && c != 0x7f { c } else { b'?' })
        .collect()
}
