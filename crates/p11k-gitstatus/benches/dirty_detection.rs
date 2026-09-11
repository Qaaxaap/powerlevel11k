//! Dirty-detection performance benchmarks (the quantitative measure for the
//! performance contract, see CONTRIBUTING.md).
//!
//! # Benchmark scenarios (fixed fixtures)
//!
//! 1. **Clean large repo**: e.g. a linux.git clone (~80k entries), the hot path when fully clean.
//! 2. **Single modified file**: minimal dirty state (exercises the early-termination path).
//! 3. **Many untracked**: a node_modules-scale directory (exercises the untracked scan).
//!
//! # Comparison method
//!
//! - This crate: criterion benchmarks (`cargo bench -p p11k-gitstatus`).
//! - Original: on the same fixture, time 10 requests to gitstatusd from a shell loop
//!   (take the median); the script lives in `.cache/` (not committed).
//! - Pass bar: p11k takes no more than 2x the original (hit the bar first, then aim to match it).
//!
//! # Fixture setup
//!
//! The benchmark assumes the `P11K_BENCH_REPO` environment variable points at an
//! initialized repo; it is skipped when unset. The fixture-generation script also
//! lives in `.cache/`.

use std::path::PathBuf;

/// Return the benchmark repo path; None when the environment variable is unset.
fn bench_repo() -> Option<PathBuf> {
    todo!("read the P11K_BENCH_REPO env var")
}

/// Scenario 1: clean-repo hot path (parse index + full dirty scan, no dirty files).
fn bench_clean_repo(c: &mut criterion::Criterion) {
    let _ = (bench_repo, c);
    todo!("wrap the clean-repo get_dirty_candidates call in criterion.bench_function")
}

/// Scenario 2: single modified file (1 dirty candidate, verifies early termination does not regress).
fn bench_single_dirty_file(c: &mut criterion::Criterion) {
    let _ = (bench_repo, c);
    todo!("same as above, fixture is 1 modified file")
}

/// Scenario 3: many untracked (the untracked cache and scan path).
fn bench_many_untracked(c: &mut criterion::Criterion) {
    let _ = (bench_repo, c);
    todo!("same as above, fixture is thousands of untracked files")
}

criterion::criterion_group!(
    benches,
    bench_clean_repo,
    bench_single_dirty_file,
    bench_many_untracked
);
criterion::criterion_main!(benches);
