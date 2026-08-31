#!/bin/zsh
# p11k.zsh-theme —— p11k 的 zsh 主题（MVP）。
#
# 目标（与 README 一致）：
# 1. source 同一份 ~/.p10k.zsh（POWERLEVEL9K_* 配置完全兼容）
# 2. 视觉与 powerlevel10k 一致（双行布局、分隔符、图标、vcs 段）
# 3. 性能不差：git 状态由 p11k-d（Rust）后台提供
#
# 设计：
# - 本文件是纯 zsh；渲染逻辑在 zsh 侧（MVP），后续迁往 p11k-d 原生协议。
# - gitstatus 客户端（zsh/gitstatus.zsh）负责启动 p11k-d、握手、异步查询。
# - 配置：~/.p10k.zsh 存在则 source；否则用内置默认（lean 风格精简版）。
#
# 当前实现的分段：dir vcs status prompt_char context time virtualenv
# command_execution_time background_jobs os_icon。未知分段跳过（TODO）。
#
# 注意：本主题直接替代 ZSH_THEME 使用；不要与 powerlevel10k 主题同载。

# ────────────────────────── 环境与入口 ──────────────────────────

# 与 p10k 相同的入口保护（避免被 oh-my-zsh 的 promptinit 干扰）
if (( $+functions[p11k] )); then
  'builtin' 'unfunction' p11k 2>/dev/null || true
fi

# 主题根目录（与 p10k 相同的 ${(%):-%x} 技巧）
typeset -gr __p11k_root_dir=${${(%):-%x}:A:h}
typeset -gr __p11k_zdir=$__p11k_root_dir/zsh

# ────────────────────────── 配置加载 ──────────────────────────

# 先清空旧配置（与 p10k 的 unset -m 行为对齐，避免多次 source 残留）
unset -m '(POWERLEVEL9K_*|DEFAULT_USER)' 2>/dev/null

# 加载顺序：内置默认（完整兜底）→ 用户配置（覆盖）。
# 用户的 .p10k.zsh 可能是精简版（如 nix home-manager 生成的），
# 缺失的变量由默认配置补齐（对齐 p10k 的默认值行为）。
source "$__p11k_zdir/default-config.zsh"
if [[ -f "${POWERLEVEL9K_CONFIG_FILE:-$HOME/.p10k.zsh}" ]]; then
  source "${POWERLEVEL9K_CONFIG_FILE:-$HOME/.p10k.zsh}"
fi

# 解析后的布局：LEFT/RIGHT_PROMPT_ELEMENTS 展开为行（newline 分段）
# p10k 用 _p9k_init_lines 拆行；MVP 简化为两行：
#   LEFT 元素按序渲染到 PROMPT；第一个 newline 之后的部分是第二行。
typeset -ga __p11k_left_lines __p11k_right_lines

# ────────────────────────── 渲染核心 ──────────────────────────

# 终端颜色：POWERLEVEL9K_COLOR_SCHEME=light/dark 决定默认色
# MVP 用 256 色 + 配置覆盖；terminfo 能力检查（对齐 p10k 的做法）
zmodload zsh/terminfo 2>/dev/null || true

# _p11k_prompt_segment：渲染一个分段。
# 参数：<背景色> <前景色> <图标> <文本> <段名>
# 输出追加到全局 __p11k_out。
#
# p10k 的分隔符行为（对齐）：
# - 第一个分段前不加分隔符
# - 后续分段用 POWERLEVEL9K_LEFT_SEGMENT_SEPARATOR（默认 \uE0B0）
# - 图标与文本之间用 POWERLEVEL9K_LEFT_PROMPT_SEGMENT_END_SYMBOL 分隔
# - 颜色经 %K/%F 转义；图标经 %b%k%F 组合
typeset -g __p11k_out=''
typeset -gi __p11k_seg_count=0

# 读配置变量，空/未定义时返回默认值（对齐 p10k 的默认值机制：
# 用户 .p10k.zsh 顶部会 unset 全部 POWERLEVEL9K_* 并只重设部分）。
function _p11k_p9k() {
  local v=${(P)1}
  [[ -n $v ]] && print -r -- "$v" || print -r -- "$2"
}

function _p11k_prompt_segment() {
  local bg=$1 fg=$2 icon=$3 text=$4
  # 默认色：POWERLEVEL9K_${name}_FOREGROUND/BACKGROUND 已由配置展开，
  # 这里只处理"无颜色"（空=默认）
  if (( __p11k_seg_count > 0 )); then
    # 分隔符：用上一段背景色
    local sep_bg=$__p11k_last_bg
    local sep_fg=$bg
    [[ -n $sep_fg ]] || sep_fg='default'
    __p11k_out+="%K{$sep_bg}%F{$sep_fg}$POWERLEVEL9K_LEFT_SEGMENT_SEPARATOR%f%k"
  fi
  if [[ -n $bg ]]; then
    __p11k_out+="%K{$bg}"
  fi
  if [[ -n $fg ]]; then
    __p11k_out+="%F{$fg}"
  fi
  __p11k_out+="$icon"
  if [[ -n $icon && -n $text && -n $POWERLEVEL9K_LEFT_PROMPT_SEGMENT_END_SYMBOL ]]; then
    __p11k_out+="$POWERLEVEL9K_LEFT_PROMPT_SEGMENT_END_SYMBOL"
  fi
  __p11k_out+="$text"
  __p11k_out+="%f%k"
  __p11k_last_bg=$bg
  (( __p11k_seg_count++ ))
}

# ────────────────────────── 分段实现 ──────────────────────────

# os_icon：OS 图标（POWERLEVEL9K_OS_ICON，默认 Linux 图标）
function _p11k_seg_os_icon() {
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_OS_ICON_BACKGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_OS_ICON_FOREGROUND 255)" \
    "$(_p11k_p9k POWERLEVEL9K_OS_ICON_CONTENT_EXPANSION '')" ''
  :
}

# dir：当前目录（缩短策略 MVP 只支持 truncate_middle 的简单变体）
# p10k 的 dir 逻辑：icon + 路径；HOME 用 ~；路径段截断。
function _p11k_seg_dir() {
  local dir=$PWD
  [[ $dir == $HOME* ]] && dir="~${dir#$HOME}"
  local icon=$(_p11k_p9k POWERLEVEL9K_DIR_ICON '')
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_DIR_BACKGROUND blue)" \
    "$(_p11k_p9k POWERLEVEL9K_DIR_FOREGROUND 236)" "$icon " "$dir"
  :
}

# status：上一条命令退出码（0 时不显示，对齐 p10k）
function _p11k_seg_status() {
  (( __p11k_last_status == 0 )) && return
  local text=$__p11k_last_status
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_STATUS_ERROR_BACKGROUND red)" \
    "$(_p11k_p9k POWERLEVEL9K_STATUS_ERROR_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_STATUS_ERROR_VISUAL_IDENTIFIER_EXPANSION '✘') " "$text"
  :
}

# prompt_char：❯/❮ 提示符（root 显示 POWERLEVEL9K_PROMPT_CHAR_{OK,ERROR}_{VIINS,VICMD,VIOWR}_FOREGROUND 的第一组）
function _p11k_seg_prompt_char() {
  local fg bg char
  if (( __p11k_last_status == 0 )); then
    fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_FOREGROUND green)
    char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_CONTENT_EXPANSION '❯')
  else
    fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VIINS_FOREGROUND red)
    char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VIINS_CONTENT_EXPANSION '❯')
  fi
  bg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_BACKGROUND default)
  _p11k_prompt_segment "$bg" "$fg" '' "$char"
  __p11k_prompt_char_fg=$fg
  :  # vi_mode 支持 TODO（KEYMAP 切换时重绘）
}

# context：user@host（与 DEFAULT_USER 相同时不显示，对齐 p10k）
function _p11k_seg_context() {
  local user=$USER host=${HOST%%.*}
  [[ $user == ${DEFAULT_USER:-} ]] && return
  local icon=${POWERLEVEL9K_CONTEXT_DEFAULT_VISUAL_IDENTIFIER_EXPANSION:-''}
  local text
  if [[ $user == root ]]; then
    text=$POWERLEVEL9K_CONTEXT_ROOT_TEMPLATE
  else
    text=$POWERLEVEL9K_CONTEXT_TEMPLATE
  fi
  text=${text//%n/$user}
  text=${text//%m/$host}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_CONTEXT_BACKGROUND 238)" \
    "$(_p11k_p9k POWERLEVEL9K_CONTEXT_FOREGROUND 255)" "$icon " "$text"
  :
}

# time：当前时间（默认 %D{%H:%M:%S} 的变体）
function _p11k_seg_time() {
  local fmt=$(_p11k_p9k POWERLEVEL9K_TIME_FORMAT '%D{%H:%M:%S}')
  local text=$POWERLEVEL9K_TIME_CONTENT_EXPANSION
  [[ -z $text ]] && text=${(%):-$fmt}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_TIME_BACKGROUND 234)" \
    "$(_p11k_p9k POWERLEVEL9K_TIME_FOREGROUND 244)" \
    "$(_p11k_p9k POWERLEVEL9K_TIME_VISUAL_IDENTIFIER_EXPANSION '') " "$text"
  :
}

# virtualenv：Python 虚拟环境名
function _p11k_seg_virtualenv() {
  [[ -n $VIRTUAL_ENV ]] || return
  local name=${VIRTUAL_ENV:t}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_VIRTUALENV_BACKGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_VIRTUALENV_FOREGROUND 255)" \
    "$(_p11k_p9k POWERLEVEL9K_VIRTUALENV_VISUAL_IDENTIFIER_EXPANSION '🐍') " "$name"
  :
}

# command_execution_time：超过 POWERLEVEL9K_COMMAND_EXECUTION_TIME_THRESHOLD 才显示
function _p11k_seg_command_execution_time() {
  local thresh=${POWERLEVEL9K_COMMAND_EXECUTION_TIME_THRESHOLD:-3}
  (( __p11k_last_exec_time >= thresh )) || return
  local text="${__p11k_last_exec_time}s"
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_COMMAND_EXECUTION_TIME_BACKGROUND yellow)" \
    "$(_p11k_p9k POWERLEVEL9K_COMMAND_EXECUTION_TIME_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_COMMAND_EXECUTION_TIME_VISUAL_IDENTIFIER_EXPANSION '') " "$text"
  :
}

# background_jobs：后台任务数
function _p11k_seg_background_jobs() {
  local -i n=${#jobstates}
  (( n > 0 )) || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_BACKGROUND_JOBS_BACKGROUND yellow)" \
    "$(_p11k_p9k POWERLEVEL9K_BACKGROUND_JOBS_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_BACKGROUND_JOBS_VISUAL_IDENTIFIER_EXPANSION '') " "$n"
  :
}

# vcs：git 状态（数据来自 p11k-d，见 zsh/gitstatus.zsh）
function _p11k_seg_vcs() {
  # __p11k_vcs_* 由 gitstatus 异步回调填充；尚未就绪时跳过
  (( ${+__p11k_vcs_ready} )) || return
  [[ -n $__p11k_vcs_branch ]] || return
  local text=$__p11k_vcs_branch
  if (( __p11k_vcs_dirty )); then
    text+=" ${POWERLEVEL9K_VCS_UNSTAGED_ICON:-!}$__p11k_vcs_unstaged"
    text+=" ${POWERLEVEL9K_VCS_UNTRACKED_ICON:-?}$__p11k_vcs_untracked"
  fi
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_VCS_CLEAN_BACKGROUND green)" \
    "$(_p11k_p9k POWERLEVEL9K_VCS_CLEAN_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_VCS_GIT_ICON '') " "$text"
  :
}

# ────────────────────────── 布局与渲染 ──────────────────────────

# 按元素名渲染一行。
# 参数：<元素名数组>；输出到 __p11k_out。
function _p11k_render_line() {
  local name
  for name in "$@"; do
    case $name in
      newline) continue;;
      os_icon) _p11k_seg_os_icon;;
      dir) _p11k_seg_dir;;
      status) _p11k_seg_status;;
      prompt_char) _p11k_seg_prompt_char;;
      context) _p11k_seg_context;;
      time) _p11k_seg_time;;
      virtualenv) _p11k_seg_virtualenv;;
      command_execution_time) _p11k_seg_command_execution_time;;
      background_jobs) _p11k_seg_background_jobs;;
      vcs) _p11k_seg_vcs;;
      *) ;; # TODO：其余段（kubecontext/aws/电池等）
    esac
  done
}

# 主渲染：precmd 调用。对齐 p10k 的布局：
#   第一行 = LEFT 首个 newline 之前 + RPROMPT（右对齐）
#   第二行 = LEFT 首个 newline 之后
# PROMPT 末尾附加 prompt_char 行的新行符（p10k 用 %\n 之类）。
function _p11k_prompt() {
  emulate -L zsh
  local -a left right
  left=("${POWERLEVEL9K_LEFT_PROMPT_ELEMENTS[@]}")
  right=("${POWERLEVEL9K_RIGHT_PROMPT_ELEMENTS[@]}")
  # 拆行：LEFT 里第一个 newline 前是第一行
  local -a line1 line2
  local name split=0
  for name in "${left[@]}"; do
    if [[ $name == newline ]]; then
      split=1
      continue
    fi
    if (( split )); then
      line2+=("$name")
    else
      line1+=("$name")
    fi
  done
  __p11k_out=''
  __p11k_seg_count=0
  __p11k_last_bg='default'
  _p11k_render_line "${line1[@]}"
  local left_prompt=$__p11k_out
  __p11k_out=''
  __p11k_seg_count=0
  __p11k_last_bg='default'
  _p11k_render_line "${right[@]}"
  local right_prompt=$__p11k_out
  # 两行布局：第一行 left+right（RPROMPT 右对齐），第二行 left2
  PROMPT="$left_prompt$POWERLEVEL9K_PROMPT_ADD_NEWLINE_PREFIX"
  RPROMPT="$right_prompt"
  if (( ${#line2} > 0 )); then
    __p11k_out=''
    __p11k_seg_count=0
    __p11k_last_bg='default'
    _p11k_render_line "${line2[@]}"
    PROMPT+="
$__p11k_out"
  fi
  PROMPT+=$POWERLEVEL9K_PROMPT_ADD_NEWLINE_SUFFIX
  PROMPT+=" "
}

# ────────────────────────── 钩子 ──────────────────────────

# precmd：记录退出码与执行时间，触发渲染与 gitstatus 查询
function _p11k_precmd() {
  emulate -L zsh
  __p11k_last_status=$?
  local now=$EPOCHREALTIME
  if (( ${+__p11k_last_cmd_time} )); then
    __p11k_last_exec_time=$(( now - __p11k_last_cmd_time ))
  fi
  __p11k_last_cmd_time=$now
  # gitstatus：异步查询当前目录（失败不传播，避免钩子返回非零）
  (( ${+functions[_p11k_gitstatus_query]} )) && _p11k_gitstatus_query || true
  _p11k_prompt
}

# preexec：记录命令开始时间
function _p11k_preexec() {
  emulate -L zsh
  __p11k_last_cmd_time=$EPOCHREALTIME
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd _p11k_precmd
add-zsh-hook preexec _p11k_preexec

# ────────────────────────── gitstatus 初始化 ──────────────────────────

source "$__p11k_zdir/gitstatus.zsh"
_p11k_gitstatus_start

# 主题入口（与 p10k 的 p10k 函数对齐，供 p10k reload 兼容）
function p11k() {
  case $1 in
    reload) _p11k_gitstatus_start; _p11k_prompt;;
    *) print -u2 "Usage: p11k reload";;
  esac
}
