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

**`main` = the Rust engine (milestone M0 done).**

The engine is a **pty host with a transparent proxy**: it spawns a theme-less
shell in a pty, passes bytes through untouched (the real terminal renders
vim/outputs directly), and takes over only for a moment at prompt time — a
shell hook announces the prompt, the engine draws the themed header
(multiline, right-aligned status) and hands back. The shell's own line editor
draws the input line, so completion/history/vi-mode geometry stays self-consistent.

Verified in M0: byte passthrough (output, vim fullscreen/editing/exit), prompt
window takeover (header + right-aligned exit status), Ctrl-C → `✘ 130`,
Tab completion (redraw column offset converges to the placeholder width — no
input-line concatenation), history, clean `exit`. Real terminal is put in raw
mode like any pty host (tmux), so ^C reaches the shell, not the engine.

Deployment model: the engine execs over the shell, so it inherits the launching
shell's environment and user config is re-sourced inside the pty shell. Recursion
is broken by `P11K_ENGINE` — the engine sets it for the inner shell, and a
misconfigured re-entry degrades to a clean shell with a fix-it message instead
of hanging. The inner shell is double-forked (orphaned) out of the engine's
process tree, and the engine emits OSC 133 A/B prompt markers so kitty's
close-window confirmation sees the shell sitting at a prompt (no more "it is
running" dialog).

### Try it (engine, main branch)

```zsh
# build
cargo build -p p11k-engine

# user config: theme must be disabled (the engine is the theme). A filtered
# copy is used during development; omit P11K_USER_ZSHRC to fall back to
# ~/.zshrc (make sure ZSH_THEME is commented out there).
P11K_USER_ZSHRC=/tmp/p11k-usertest.zshrc target/debug/p11k

# or, as a "theme": add one line to the shell's rc and open a shell.
# The engine cannot auto-detect the launching shell after exec (version vars
# are not exported, parent is the terminal), so pass it explicitly:
#   zsh:  [[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell zsh
#   bash: [[ -z "$P11K_ENGINE" ]] && exec /path/to/p11k --shell bash
#   fish: if not set -q P11K_ENGINE; exec /path/to/p11k --shell fish; end
exec /path/to/target/debug/p11k --shell zsh
```

Known limits (M0, tracked): full-screen zle redraws on resize can clobber the
header; a previous command that does not end with a newline leaves the header
painted over residual text; user shell configs are re-sourced but a theme set
via `ZSH_THEME` still fights the engine (documented, not auto-filtered).

**`compat/p10k` = the p10k-compatible line (maintained).** p10k's zsh
rendering is a decade of polish and must not be rewritten; that branch
vendors the p10k theme and replaces only the kernel: `p11k-d` (Rust)
answers the gitstatusd wire protocol, byte-identical across a 12-scenario
differential matrix. Use it via:

```zsh
# on the compat/p10k branch:
export GITSTATUS_DAEMON=/path/to/p11k-d
```

Config, look, status, vcs — everything stays identical to p10k, except
the git-status backend is a maintained Rust binary.

### Kernel roadmap (shared)

- [x] M1: byte-compatible gitstatusd replacement (v1.5.5 protocol)
- [x] Performance core: index parsing, `fstatat` scanning, parallel
      shards, untracked cache, RAII directory fds
- [ ] Long-term maintenance & hardening of the daemon (the point of p11k)

## License

GPLv3. See [LICENSE](LICENSE).


### Third-party notices

This project vendors the powerlevel10k theme under `vendor/powerlevel10k/`,
licensed under the MIT License, copyright (c) 2019 Roman Perepelitsa and
contributors (the original copyright notice is kept verbatim in
`vendor/powerlevel10k/LICENSE`).

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

**`main` 分支 = Rust 引擎（M0 完成）。**

引擎是一个 **pty 宿主 + 透明代理**：在 pty 里跑一个无主题 shell，字节原样
透传（vim/命令输出由真实终端直接渲染），只在 prompt 时刻短暂接管——shell
钩子宣告出提示符后，引擎画主题 header（多行、右侧对齐的状态栏），画完交还。
输入行由 shell 自己的行编辑器画，补全/历史/vi 模式的几何自洽。

M0 已验证：字节透传（输出、vim 全屏/编辑/退出）、prompt 窗口接管（header +
右侧退出码）、Ctrl-C → `✘ 130`、Tab 补全、历史、resize 重对齐、干净退出。
真实终端被设为 raw 模式（和 tmux 等 pty 宿主一致），所以 ^C 打到的是 shell
而不是引擎。

已知局限（M0 记录）：引擎死则 shell 一起死（pty EOF 触发 SIGHUP，计划用
keepalive 持有进程解决）；resize 时 zle 全屏重绘可能冲掉 header；尚未接入
用户真实 shell 配置（shell 跑在引擎生成的 ZDOTDIR 里）。

**`compat/p10k` 分支 = p10k 兼容线（维护中）。** p10k 的 zsh 渲染是十年
打磨的产物，不该重写；该分支 vendor p10k 主题，只替换内核：`p11k-d`
（Rust）以字节兼容方式应答 gitstatusd 线上协议（12 场景差分矩阵逐字节
一致）。用法：

```zsh
# 在 compat/p10k 分支下：
export GITSTATUS_DAEMON=/path/to/p11k-d
```

配置、外观、status、vcs —— 一切与 p10k 完全一致，唯一区别是 git
状态后端换成了有人维护的 Rust 二进制。

### 内核路线图（两条线共用）

- [x] M1：字节兼容的 gitstatusd 替代（v1.5.5 协议）
- [x] 性能核心：index 解析、`fstatat` 扫描、并行分片、untracked
      缓存、RAII 目录 fd
- [ ] daemon 的长期维护与加固（p11k 的真正价值）

## 许可证

GPLv3，见 [LICENSE](LICENSE)。

### 第三方声明

本项目在 `vendor/powerlevel10k/` 下包含 powerlevel10k 主题，采用 MIT
许可证，版权归 Roman Perepelitsa 及贡献者（2019）（原版权声明原样保留于
`vendor/powerlevel10k/LICENSE`）。
