//! 脏检测性能基准（性能契约的量化手段，见 CONTRIBUTING.md）。
//!
//! # 基准场景（固定 fixture）
//!
//! 1. **干净大仓库**：如 linux.git 克隆（~8 万条目），完全干净时的热路径。
//! 2. **单文件修改**：最小脏状态（验证提前终止路径）。
//! 3. **大量 untracked**：如 node_modules 级目录（验证 untracked 扫描）。
//!
//! # 对比方法
//!
//! - 本 crate：criterion 基准（`cargo bench -p p11k-gitstatus`）。
//! - 原版：同一 fixture 上，用 shell 循环对 gitstatusd 发 10 次请求计时
//!   （取中位数），脚本放 `.cache/`（不入库）。
//! - 达标线：p11k 耗时不超过原版的 2 倍（先达标，再追求追平）。
//!
//! # fixture 准备
//!
//! 基准假设环境变量 `P11K_BENCH_REPO` 指向已初始化的仓库；
//! 未设置时跳过。fixture 生成脚本同样放 `.cache/`。

use std::path::PathBuf;

/// 取基准仓库路径；未设置环境变量返回 None。
fn bench_repo() -> Option<PathBuf> {
    todo!("read the P11K_BENCH_REPO env var")
}

/// 场景 1：干净仓库热路径（parse index + 全量脏扫描，无脏文件）。
fn bench_clean_repo(c: &mut criterion::Criterion) {
    let _ = (bench_repo, c);
    todo!("wrap the clean-repo get_dirty_candidates call in criterion.bench_function")
}

/// 场景 2：单文件修改（脏候选 1 个，验证提前终止不会退化）。
fn bench_single_dirty_file(c: &mut criterion::Criterion) {
    let _ = (bench_repo, c);
    todo!("same as above, fixture is 1 modified file")
}

/// 场景 3：大量 untracked（untracked 缓存与扫描路径）。
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
