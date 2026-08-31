//! 与原版 gitstatusd 的对拍（differential）测试。
//!
//! # 前置条件
//!
//! 环境变量 `GITSTATUSD_PATH` 指向原版 C++ gitstatusd 二进制。
//! 未设置时本文件的测试整体跳过（`--ignored` 标记）。
//!
//! # 方法
//!
//! 1. 用同一套 FIFO/管道启动原版 daemon 与 p11k-d（见 daemon.rs 的启动
//!    时序注释；测试侧模拟 zsh：mkfifo + 后台 spawn + 读 20 字节 pgid）。
//! 2. 向两者喂**完全相同的请求字节流**（覆盖：握手、普通 repo、GIT_DIR
//!    前缀、diff 标志、超大 index、非仓库目录）。
//! 3. 逐字节比较两者的响应。协议要求**完全一致**，无豁免字段。
//!
//! # 已知差异风险
//!
//! - **SafePrint 高字节**：原版对字节 >0x7F 的处理随构建环境漂移
//!   （`std::isprint(负数)` 是 UB）。官方预编译二进制实测保留 >0x7F
//!   （与 p11k 一致，无差异）；本地自编译 gitstatusd 可能变 `'?'`。
//!   对拍请使用官方预编译二进制（`GITSTATUSD_PATH`）。
//! - 响应中与时间/路径相关的字段（如 workdir 规范化、commit summary 的
//!   编码）需用相同输入保证确定性；对拍时仓库 fixture 必须状态固定。
//! - 原版并发行为（tag 查询后台线程）不产生可观测差异，响应顺序仍按
//!   请求顺序。

/// 对拍：单仓库常见状态序列（clean → modify → stage → untracked）。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_single_repo() {
    todo!("实现：spawn 两端 → 喂同一请求序列 → 逐字节比较响应")
}

/// 对拍：GIT_DIR 前缀与 linked worktree 请求。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_gitdir_and_worktree() {
    todo!("实现：':/path/.git' 请求的两端行为一致性")
}

/// 对拍：探活与退出（-l 锁释放、-p 父进程消失、EOF）。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_liveness() {
    todo!("实现：验证 EOF 退出、lock-fd/parent-pid 探活路径的退出码一致")
}
