//! gitstatus 线上协议（IPC wire format）。
//!
//! 依据：romkatv/gitstatus v1.5.5 的 `serialization.h` / `request.cc` / `response.cc`。
//!
//! # 传输层
//!
//! 请求走 daemon 的 **stdin**，响应走 daemon 的 **stdout**，均为字节流。
//! 每条消息以消息分隔符结尾，消息内字段以字段分隔符分隔。
//! 没有长度前缀，没有转义层（字符串内容里不出现分隔符，见 [`safe_print`]）。
//!
//! # 与原版的已知差异
//!
//! 无。SafePrint 行为以官方预编译二进制的**实测**为准（v1.5.4
//! linux-x86_64，Alpine musl + GCC 9.3.0 static-pie）：控制字符 → `'?'`，
//! 字节 >0x7F 原样保留（实测 commit_summary 中 UTF-8 不变、`\x01` 变
//! `'?'`）。p11k 固定采用作者意图（保留 >0x7F），与官方二进制一致。
//! 原版源码 `c > 127 || std::isprint(c)` 中 `std::isprint(负数)` 是 UB，
//! 本地 glibc 自编译行为可能不同；对拍测试须使用官方预编译二进制。

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
/// 字段语义（对齐 request.cc 的 ParseRequest）：
///
/// - **id**：任意字节串（zsh 侧形如 `"1699999999.123 _p9k_vcs"`），响应首字段
///   必须**逐字节**回显。用 `Vec<u8>` 而非 `String`：原版 std::string 不要求
///   UTF-8，回显必须字节忠实。
/// - **dir**：目录绝对路径。首字节 `:` 表示其后是 **GIT_DIR 的直接路径**
///   （from_dotgit：直接打开该 .git，不向上搜索）。`dir` 可为空（握手请求）——
///   原版此处是 `*begin` 解引用的 UB，实际表现为空 dir 不触发 from_dotgit。
///   同样用 `Vec<u8>`：Linux 路径无编码约定。
/// - **diff**：可选第三字段，必须为单字节 `'0'` 或 `'1'`。`'1'` = 跳过 index
///   比较（skip_index=true），`'0'` 或缺省 = 全算。注意原版内部 bool `diff`
///   的语义与本字段**相反**（线上 '0' → 内部 true）。
pub struct Request {
    /// 请求 id，响应首字段逐字节回显。
    pub id: Vec<u8>,
    /// 目录绝对路径；`dir_is_gitdir` 为 true 时不含前导 `:`。
    pub dir: Vec<u8>,
    /// `dir` 是否以 `:` 前缀给出（= 直接把 `dir` 当 GIT_DIR 用）。
    pub dir_is_gitdir: bool,
    /// 线上第三字段 `'1'` = 跳过 index 比较（不统计 staged/unstaged/untracked）。
    pub skip_index: bool,
}

/// 从字节流解析一条请求（不含末尾 MSG_SEP）。
///
/// 输入必须非空：EOF 由调用方检测（daemon 层 read 返回 0 字节），
/// 空消息（两个连续 MSG_SEP）与原版一样属于畸形输入。
/// 畸形请求（缺字段分隔符、diff 字段非单字节 `'0'`/`'1'`、超过 3 个字段）
/// 直接 panic：对齐原版 `VERIFY` 失败即 abort（request.cc），daemon 进程终止。
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
    /// 原样回显的请求 id（字节忠实；写入前经 [`safe_print`]）。
    pub id: Vec<u8>,
    /// true = 是仓库（fields 有效）；false = 非仓库（无后续字段）。
    pub is_repo: bool,
    /// 仅 `is_repo` 为 true 时有值，长度恒为 [`field::COUNT`]。
    /// 数字字段存十进制字节串（对齐原版 Print(ssize_t) 的直接十进制）。
    /// 用 `Vec<u8>`：分支名/路径不保证 UTF-8（对齐原版 StringView）。
    pub fields: Option<[Vec<u8>; field::COUNT]>,
}

/// 把 [`Response`] 序列化为线上字节（含末尾 MSG_SEP）。
///
/// 布局（对齐 response.cc 的 ResponseWriter）：
///
/// ```text
/// safe_print(id) \x1f 1 [\x1f safe_print(f1) ... \x1f safe_print(f27)] \x1e
/// ```
///
/// 非仓库响应只写 `safe_print(id) \x1f 0 \x1e`（对齐析构回退路径）。
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

/// SafePrint 转义（对齐 response.cc:33-38 的**作者意图**）：
///
/// - 可打印 ASCII（0x20..=0x7E）原样保留
/// - 字节 >0x7F 原样保留（UTF-8 内容不受影响）
/// - 其余（控制字符 <0x20、DEL 0x7F）→ `'?'`
///
/// 与原版 x86 字面行为的差异见模块文档「与原版的已知差异」：p11k 不做
/// 平台分叉，固定保留 >0x7F。注意按**字节**映射而非按 Unicode 字符处理。
pub fn safe_print(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&c| if c >= 0x20 && c != 0x7f { c } else { b'?' })
        .collect()
}
