//! gitstatus 线上协议（IPC wire format）。
//!
//! 依据：romkatv/gitstatus v1.5.5 的 `serialization.h` / `request.cc` / `response.cc`。
//! 本模块只定义"字节长什么样"；解析与序列化的生产逻辑留待实现。
//!
//! # 传输层
//!
//! 请求走 daemon 的 **stdin**，响应走 daemon 的 **stdout**，均为字节流。
//! 每条消息以消息分隔符结尾，消息内字段以字段分隔符分隔。
//! 没有长度前缀，没有转义层（字符串内容里不出现分隔符，见 [`safe_print`]）。

/// 字段分隔符：ASCII 31 (US, Unit Separator)。
pub const FIELD_SEP: u8 = 0x1f;

/// 消息分隔符（消息终结符）：ASCII 30 (RS, Record Separator)。
pub const MSG_SEP: u8 = 0x1e;

/// 握手的特殊 id。zsh 侧发送 `\x1ehello\x1f\x1e` 探活并校验协议，
/// daemon 应回复 `\x1ehello\x1f0\x1e`（id=hello 的"非仓库"响应）。
pub const HELLO_ID: &str = "}hello";

/// 一条请求。线上格式（以 MSG_SEP 结尾）：
///
/// ```text
/// <id>\x1f[:]<dir>\x1f<diff>\x1e
/// ```
///
/// 字段语义（request.cc:45-56）：
///
/// - **id**：任意非空串，响应首字段必须原样回显。zsh 侧形如
///   `"1699999999.123 _p9k_vcs"`（`$EPOCHREALTIME` + 空格 + 回调函数名），
///   用以把异步响应路由回发起请求的回调。
/// - **dir**：目录绝对路径。若以 `:` 开头，表示其后是 **GIT_DIR 的直接
///   路径**（from_dotgit：直接打开该 .git，不向上搜索父目录）；否则是
///   普通工作目录路径，daemon 需要自己向上搜索 `.git`。
/// - **diff**：可选字段。`'1'` = 跳过 index 比较（不统计
///   staged/unstaged/untracked，只回元信息）；`'0'` 或缺省 = 全算。
pub struct Request {
    /// 请求 id，响应首字段原样回显。
    pub id: String,
    /// 目录绝对路径；`dir_is_gitdir` 为 true 时不含前导 `:`。
    pub dir: String,
    /// `dir` 是否以 `:` 前缀给出（= 直接把 `dir` 当 GIT_DIR 用）。
    pub dir_is_gitdir: bool,
    /// diff 字段：None=缺省，Some(true)='1'（跳过 index），Some(false)='0'。
    pub diff: Option<bool>,
}

/// 从字节流解析一条请求（不含末尾 MSG_SEP）。
///
/// 返回 `None` 表示读到 0 字节（EOF）——调用方应正常退出（exit 0）。
///
/// 实现要点：
/// - 按 FIELD_SEP 切分，恰好 2 或 3 个字段；多余/缺少字段的行为要与
///   原版一致（原版宽容处理：diff 缺失按 `'0'`）。
/// - dir 首字节为 `:` 时剥掉前缀并置 `dir_is_gitdir = true`。
/// - id 不允许为空（握手请求除外：id=`}hello`、dir 为空）。
pub fn parse_request(bytes: &[u8]) -> Option<Request> {
    let _ = bytes;
    todo!("实现：切分字段、处理 ':' 前缀与 diff 标志；EOF 返回 None")
}

/// 响应数据字段的固定顺序索引（0-based）。
///
/// 线上布局：首字段 **id**、次字段 **1/0**（1=是仓库，0=非仓库）。
/// 仅当次字段为 1 时跟随 [`field::COUNT`] 个数据字段，字段间以 FIELD_SEP
/// 分隔，整条消息以 MSG_SEP 结尾。字段顺序固定，见 gitstatus.cc:86-177
/// 的 Print 顺序（协议文档 options.cc:149-188）。
pub mod field {
    /// 工作目录绝对路径。
    pub const WORKDIR: usize = 0;
    /// 当前提交的 40 位十六进制 SHA。
    pub const COMMIT: usize = 1;
    /// 本地分支名（detached HEAD 时为空串）。
    pub const LOCAL_BRANCH: usize = 2;
    /// 上游分支名。
    pub const REMOTE_BRANCH: usize = 3;
    /// 上游 remote 名。
    pub const REMOTE_NAME: usize = 4;
    /// 上游 remote URL。
    pub const REMOTE_URL: usize = 5;
    /// 进行中的操作（rebase/merge/...），空串=无。
    pub const ACTION: usize = 6;
    /// index 条目总数（十进制字符串）。
    pub const INDEX_SIZE: usize = 7;
    /// staged 变更数（受 -s 上限约束）。
    pub const NUM_STAGED: usize = 8;
    /// unstaged 变更数（受 -u 上限约束）。
    pub const NUM_UNSTAGED: usize = 9;
    /// 冲突文件数（受 -c 上限约束）。
    pub const NUM_CONFLICTED: usize = 10;
    /// untracked 文件数（受 -d 上限约束）。
    pub const NUM_UNTRACKED: usize = 11;
    /// 领先上游的提交数。
    pub const COMMITS_AHEAD: usize = 12;
    /// 落后上游的提交数。
    pub const COMMITS_BEHIND: usize = 13;
    /// stash 数。
    pub const STASHES: usize = 14;
    /// 当前提交上的最近 tag 名（无则为空串）。
    pub const TAG: usize = 15;
    /// unstaged 中"被删除"的文件数。
    pub const NUM_UNSTAGED_DELETED: usize = 16;
    /// staged 中"新增"的文件数。
    pub const NUM_STAGED_NEW: usize = 17;
    /// staged 中"被删除"的文件数。
    pub const NUM_STAGED_DELETED: usize = 18;
    /// push remote 名。
    pub const PUSH_REMOTE_NAME: usize = 19;
    /// push remote URL。
    pub const PUSH_REMOTE_URL: usize = 20;
    /// push 领先提交数。
    pub const PUSH_COMMITS_AHEAD: usize = 21;
    /// push 落后提交数。
    pub const PUSH_COMMITS_BEHIND: usize = 22;
    /// skip-worktree 标记的文件数。
    pub const NUM_SKIP_WORKTREE: usize = 23;
    /// assume-unchanged 标记的文件数。
    pub const NUM_ASSUME_UNCHANGED: usize = 24;
    /// commit message 的编码（如 "utf-8"）。
    pub const COMMIT_ENCODING: usize = 25;
    /// commit message 摘要（受 -z 截断，经 SafePrint 转义）。
    pub const COMMIT_SUMMARY: usize = 26;
    /// 数据字段总数。非仓库响应（次字段=0）没有这些字段。
    pub const COUNT: usize = 27;
}

/// 一条完整响应（已按仓库/非仓库拆解）。
pub struct Response {
    /// 原样回显的请求 id。
    pub id: String,
    /// true = 是仓库（fields 有效）；false = 非仓库（无后续字段）。
    pub is_repo: bool,
    /// 仅 `is_repo` 为 true 时有值，长度恒为 [`field::COUNT`]。
    /// 数字字段存十进制字符串（zsh 侧 `typeset -gi` 自行转换）。
    pub fields: Option<[String; field::COUNT]>,
}

/// 把 [`Response`] 序列化为线上字节（含末尾 MSG_SEP）。
///
/// 实现要点：
/// - `is_repo = false` 时只写 `id` 与 `0` 两个字段。
/// - 所有字符串字段写入前必须过 [`safe_print`]。
/// - 字段间 FIELD_SEP，末尾 MSG_SEP，无其他空白。
pub fn serialize_response(r: &Response) -> Vec<u8> {
    let _ = r;
    todo!("实现：按 FIELD_SEP 连接字段、MSG_SEP 结尾；字符串先 SafePrint")
}

/// SafePrint 转义（response.cc:33-38），必须与原版**逐字节一致**：
///
/// - ASCII 控制字符与不可打印字符 → `'?'`
/// - 字节 > 127 原样保留（UTF-8 内容不受影响）
///
/// 若不一致，含特殊字符的分支名 / commit summary 会打乱字段边界，
/// 造成整个响应错位。注意按**字节**映射而非按 Unicode 字符处理。
pub fn safe_print(s: &str) -> String {
    let _ = s;
    todo!("实现：逐字节映射；可打印 ASCII 保留，<32 与 127 换成 '?'，>=128 保留")
}
