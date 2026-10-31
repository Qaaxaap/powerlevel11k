//! Wall-clock benchmark of `RepoCache::build_fields`, the call a prompt makes on every precmd.
//!
//! Real execution means the machine's load, frequency and cache pressure show up in the numbers.
//! Use this harness for the figure you quote, on a quiet machine, and quote the machine too. Use
//! the `iai` harness for regression judgments: its instruction counts do not move with the
//! environment.
//!
//! ```text
//! cargo bench -p p11k-gitstatus --bench build_fields_criterion -- --save-baseline before
//! # make the change
//! cargo bench -p p11k-gitstatus --bench build_fields_criterion -- --baseline before
//! ```

mod common;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use common::Prepared;

fn bench_state(c: &mut Criterion, name: &str, files: usize, dirty: bool) {
    let mut prepared = Prepared::new(name, files, dirty);
    c.bench_function(&format!("build_fields/{name}"), |b| {
        b.iter(|| {
            let repo = prepared
                .cache
                .get_or_open(&prepared.path, false)
                .expect("repo opens");
            black_box(repo.build_fields(false))
        })
    });
}

fn build_fields(c: &mut Criterion) {
    bench_state(c, "clean-small", 50, false);
    bench_state(c, "dirty-small", 50, true);
    bench_state(c, "clean-large", 5000, false);
}

criterion_group!(benches, build_fields);
criterion_main!(benches);
