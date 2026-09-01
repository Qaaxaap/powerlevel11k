//! p11k-gitstatus — a byte-compatible Rust reimplementation of the
//! gitstatusd wire protocol (v1.5.5).
//!
//! The daemon can replace the original gitstatusd behind powerlevel10k /
//! gitstatus.plugin.zsh without any changes: set `GITSTATUS_DAEMON` to it.
//!
//! # Modules
//!
//! | Module | Purpose |
//! |---|---|---|
//! | [`protocol`] | Wire format: separators, requests, responses, handshake, SafePrint |
//! | [`options`] | Command-line arguments and exit codes |
//! | [`daemon`] | Process lifecycle: FIFO, pgid handshake, liveness, main loop |
//! | [`repo`] | Repo handle and LRU cache |
//! | [`index`] | git index parsing and dirty-candidate scan (performance core) |
//! | [`scan`] | Worktree traversal (openat/fstatat, directory stack) |
//! | [`untracked_cache`] | CheckDirMtime probe and untracked cache |
//!
//! # Performance
//!
//! Hot path for clean repos should stay in the tens-of-milliseconds range;
//! benchmark against the C++ original (see CONTRIBUTING.md "Performance
//! contract"). Key points: parallel index shards, early exit, directory-stack
//! reuse, untracked-cache pruning.

pub mod daemon;
pub mod index;
pub mod options;
pub mod protocol;
pub mod repo;
pub mod scan;
pub mod untracked_cache;
