#!/bin/zsh
# p11k.zsh-theme —— 薄加载器：powerlevel10k 渲染（vendor）+ p11k-d Rust 内核。
#
# 用法：oh-my-zsh 设 `ZSH_THEME="p11k"`，或直接 source 本文件。
#
# 设计（见 README「p11k 的正确用法」）：
# - 渲染/配置/视觉：**100% 复用 powerlevel10k**（vendor/powerlevel10k，MIT，
#   版权归 romkatv 及贡献者，见 vendor/powerlevel10k/LICENSE）。p10k 的 zsh
#   渲染是十年打磨，不该重写。
# - git 状态：**p11k-d**（Rust 内核），通过 GITSTATUS_DAEMON 注入给 p10k 的
#   gitstatus.plugin.zsh。这是 p11k 的唯一重写（技术原因：gitstatusd 是
#   C++/libgit2，p10k 停维护后无人修）。
#
# 历史：本文件曾是"在 zsh 里重写 p10k 渲染"的实验（已归档，见 git 历史），
# 结论是不可行（配置覆盖与打磨追不上 p10k），已重定位为薄加载器。

# 主题根目录（${(%):-%x} 展开为当前脚本路径）
typeset -gr __p11k_root_dir=${${(%):-%x}:A:h}

# git 状态内核：默认指向仓库内构建的 p11k-d；用户可用 GITSTATUS_DAEMON 覆盖
export GITSTATUS_DAEMON="${GITSTATUS_DAEMON:-$__p11k_root_dir/target/release/p11k-d}"
if [[ ! -x $GITSTATUS_DAEMON ]]; then
  print -u2 "p11k: daemon not found at $GITSTATUS_DAEMON (run 'cargo build --release' in $__p11k_root_dir)"
fi

# 复用 powerlevel10k 主题（懒加载，函数名保持 p10k/prompt_powerlevel9k_setup）
'builtin' 'source' "$__p11k_root_dir/vendor/powerlevel10k/powerlevel10k.zsh-theme"

# 确保启用（p10k 懒加载；重复调用幂等，见 _p9k_setup 的 __p9k_enabled 保护）
if (( $+functions[prompt_powerlevel9k_setup] )); then
  prompt_powerlevel9k_setup
fi

# 品牌命令：p11k 转发 p10k（configure/reload/display/segment/help）
if (( $+functions[p10k] )); then
  function p11k() { p10k "$@" }
fi
