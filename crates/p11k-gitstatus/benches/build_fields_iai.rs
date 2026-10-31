//! Instruction-count benchmark of `RepoCache::build_fields`, measured with Callgrind.
//!
//! Callgrind counts user-space instructions under deterministic emulation, so the counts do not
//! move with CPU load, frequency or cache pressure. Use this harness to judge whether a change
//! made the call faster or slower, and the criterion harness for wall-clock milliseconds.
//!
//! The fixture is pinned (`git init -b main`, fixed author and committer dates), so repeated runs
//! measure the same commit. With that, the two 50-file cases repeat exactly and the 5000-file case
//! moves by about 0.03% between runs, because `readdir` order depends on the file system. Treat
//! differences below 0.1% as noise.
//!
//! The devShell provides both Valgrind and the runner (`just bench-iai`). Outside Nix:
//!
//! ```text
//! cargo install iai-callgrind-runner --version 0.16.1   # must match iai-callgrind in Cargo.toml
//! # plus your distribution's valgrind package
//! ```

mod common;

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};

use common::Prepared;

#[library_benchmark]
#[bench::clean_small(Prepared::new("clean-small", 50, false))]
#[bench::dirty_small(Prepared::new("dirty-small", 50, true))]
#[bench::clean_large(Prepared::new("clean-large", 5000, false))]
fn build_fields(mut prepared: Prepared) -> [Vec<u8>; p11k_gitstatus::protocol::field::COUNT] {
    let repo = prepared
        .cache
        .get_or_open(&prepared.path, false)
        .expect("repo opens");
    black_box(repo.build_fields(false))
}

library_benchmark_group!(
    name = build_fields_group;
    benchmarks = build_fields
);

main!(library_benchmark_groups = build_fields_group);
