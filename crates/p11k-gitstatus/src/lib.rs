//! p11k-gitstatus —— gitstatusd v1.5.5 协议的 Rust 复刻库。
//!
//! # 目标（M1）
//!
//! 实现一个**逐字节兼容**的 gitstatus 后端：现有 powerlevel10k /
//! `gitstatus.plugin.zsh` 无需任何修改即可把本库实现的 daemon 当作
//! gitstatusd 使用。兼容契约的全部细节见调研报告
//! `.cache/p11k-research/gitstatus-report.md`（刻意不入库）。
//!
//! # 模块地图
//!
//! | 模块 | 职责 | 对齐原版 |
//! |---|---|---|
//! | [`protocol`] | 线上格式：分隔符、请求/响应、握手、SafePrint | serialization.h / request.cc / response.cc |
//! | [`options`] | 命令行参数与退出码 | options.cc / options.h |
//! | [`daemon`] | 进程生命周期：FIFO、pgid 握手、探活、主循环 | gitstatus.plugin.zsh / gitstatus.cc |
//! | [`repo`] | 仓库句柄与 LRU 缓存 | repo_cache.cc |
//! | [`index`] | git index 解析与脏候选扫描（性能核心） | index.cc |
//! | [`scan`] | 工作区遍历（openat/fstatat、目录栈） | index.cc 遍历部分 |
//! | [`untracked_cache`] | CheckDirMtime 探针与 untracked 缓存 | check_dir_mtime.cc |
//!
//! # 性能契约
//!
//! 干净仓库热路径的目标量级为数十毫秒，必须与 C++ 原版做基准对比
//! （见 CONTRIBUTING.md「性能契约」）。性能关键点：index 并行分片、
//! 提前终止、目录栈复用、Arena 分配、untracked cache 剪枝。
//!
//! # 实现约定
//!
//! 本 crate 的生产逻辑由人实现；AI 只提供模块骨架、类型签名与本注释。
//! 每个 `todo!()` 对应的实现要点都写在所在模块的文档注释里。

pub mod daemon;
pub mod index;
pub mod options;
pub mod protocol;
pub mod repo;
pub mod scan;
pub mod untracked_cache;
