# powerlevel11k

A zsh prompt in the spirit of powerlevel10k, rebuilt with a Rust core.

> **Status: early development.** Nothing here works yet. Do not source this in
> your `.zshrc`. This repository exists so the work happens in the open.
> Comments and scaffolding are written with the help of AI agents; production
> code is written by humans.

## Why this exists

powerlevel10k is not dead, but it is done. In 2024 romkatv moved it to
maintenance-only mode: no new features, most bugs won't be fixed, help
requests are ignored, and the repo will not be handed over. It still works
today and it may keep working for years, but anyone who wants more than what
ships in v1.20.0 has nowhere to go.

p11k picks up from there. The plan:

- Keep the powerlevel10k look: two-line layout, segments, separators,
  `p10k configure`-style setup.
- Keep the two things that make p10k feel fast: instant prompt and a
  background git status daemon.
- Keep existing configs working: source your current `.p10k.zsh` and get
  the same prompt, no migration.
- Replace the internals: git status, async segments, and rendering move out
  of zsh/C++ into a Rust daemon (`p11k-d`). zsh keeps a thin shim for the
  things only zsh knows (`$KEYMAP`, exit codes, job states).

## Why Rust

Three practical reasons:

1. p10k's only compiled component is `gitstatusd`, a C++ daemon linked
   against libgit2. That is the piece worth rewriting, and Rust is a better
   fit for a long-lived background process that parses untrusted git index
   data.
2. A single static Rust binary ships per OS/arch. p10k needs to fetch the
   right gitstatusd per zsh version; p11k won't.
3. The niche is empty: there is no maintained Rust successor to p10k, and
   the closest prior attempt (gitstatus-rs) was abandoned in 2023.

## Roadmap

1. Reimplement the gitstatusd protocol in Rust, byte-for-byte compatible, so
   p11k's daemon can back the original p10k. This is the foundation.
2. Rebuild the performance core: index parsing, `fstatat`-based scanning,
   parallel shards, untracked cache. Benchmarked against the C++ original.
3. Rendering engine: ANSI width math, templates, and the core segments
   (dir, vcs, status, execution time, jobs, virtualenv, ...).
4. Instant prompt and transient prompt, driven by the zsh shim.
5. The long tail: remaining segments, configuration wizard, framework
   integrations (oh-my-zsh, prezto, zinit).

## Status

M1 (gitstatusd-compatible daemon) is done: `p11k-d` answers the same
wire protocol as the original and is byte-identical across a 12-scenario
differential matrix (modified/deleted/untracked, synthetic 5k-file repo
and nixpkgs). It can replace gitstatusd via `GITSTATUS_DAEMON`.

A first-party zsh theme (`p11k.zsh-theme`) is under development. It
sources the same `~/.p10k.zsh` (with default fallbacks for sparse
configs), renders the core segments, supports
`POWERLEVEL9K_SHORTEN_STRATEGY`, transient prompt and vi mode, and
queries git status through `p11k-d` asynchronously.

```zsh
# oh-my-zsh: ZSH_THEME 里不能用自定义路径，直接 source 替代
source ~/Projects/powerlevel11k/p11k.zsh-theme
# 或先 build daemon（主题依赖它）
cd ~/Projects/powerlevel11k && cargo build --release
```

## License

GPLv3. See [LICENSE](LICENSE).

---

# powerlevel11k（中文）

一个延续 powerlevel10k 路线的 zsh 提示符，内核用 Rust 重写。

> **状态：早期开发。** 目前没有任何可用的功能，不要把它写进你的
> `.zshrc`。建这个仓库是为了把开发过程公开进行。注释与框架由 AI
> 辅助编写，生产代码由人编写。

## 为什么要做

powerlevel10k 没有死，但它已经完结了。2024 年 romkatv 宣布进入纯维护
模式：不新增功能、大多数 bug 不修、求助被忽略，且仓库不会移交。它今天
依然能用，也许还能用好几年，但想要 v1.20.0 之外东西的人没有去处。

p11k 从这里接手。计划是：

- 保持 powerlevel10k 的外观：双行布局、分段、分隔符、类似
  `p10k configure` 的配置方式。
- 保留让它"快"的两样东西：instant prompt 和后台 git 状态守护进程。
- 兼容存量配置：直接 source 你现有的 `.p10k.zsh`，得到同样的提示符，
  无需迁移。
- 重写内部实现：git 状态、异步分段、渲染从 zsh/C++ 搬进 Rust 守护进程
  （`p11k-d`），zsh 侧只留一个薄适配层，处理只有 zsh 才知道的东西
  （`$KEYMAP`、退出码、任务状态）。

## 为什么用 Rust

三个实际理由：

1. p10k 唯一的编译组件是 `gitstatusd`——一个链接 libgit2 的 C++ 守护
   进程。这是最值得重写的部分，而一个需要长期运行、解析不可信 git
   index 数据的后台进程，Rust 比 C++ 更合适。
2. 单个静态 Rust 二进制按 OS/arch 分发即可。p10k 需要按 zsh 版本拉取
   对应的 gitstatusd，p11k 不用。
3. 这个生态位是空的：不存在有人维护的 Rust 版 p10k 继任者，最接近的
   尝试（gitstatus-rs）2023 年就停了。

## 路线图

1. 用 Rust 逐字节复刻 gitstatusd 协议，让 p11k 的守护进程可以直接给原版
   p10k 当后端。这是所有工作的地基。
2. 重建性能内核：index 解析、基于 `fstatat` 的扫描、并行分片、untracked
   缓存，并与 C++ 原版做基准对比。
3. 渲染引擎：ANSI 宽度计算、模板与核心分段（dir、vcs、status、执行时间、
   后台任务、virtualenv……）。
4. instant prompt 与 transient prompt，由 zsh 适配层驱动时序。
5. 长尾：其余分段、配置向导、框架集成（oh-my-zsh、prezto、zinit）。

## 当前状态

M1（gitstatusd 兼容 daemon）已完成：`p11k-d` 与原版走同一线上协议，
在 12 场景差分矩阵（修改/删除/untracked，5k 合成仓库与 nixpkgs）中
逐字节一致，可通过 `GITSTATUS_DAEMON` 直接替换 gitstatusd。

第一方 zsh 主题（`p11k.zsh-theme`）开发中：source 同一份
`~/.p10k.zsh`（精简配置有默认值兜底），渲染核心分段，支持
`POWERLEVEL9K_SHORTEN_STRATEGY`、transient prompt 与 vi 模式，
通过 `p11k-d` 异步获取 git 状态。

```zsh
# 先构建 daemon（主题依赖它）
cd ~/Projects/powerlevel11k && cargo build --release
# 在 .zshrc 里加载主题
source ~/Projects/powerlevel11k/p11k.zsh-theme
```

## 许可证

GPLv3，见 [LICENSE](LICENSE)。
