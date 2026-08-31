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

# 主题根目录（与 p10k 相同的 ${(%):-%x} 技巧）。
# 重复 source 保护：typeset -gr 二次定义会报 read-only 错误。
(( $+__p11k_root_dir )) || typeset -gr __p11k_root_dir=${${(%):-%x}:A:h}
(( $+__p11k_zdir )) || typeset -gr __p11k_zdir=$__p11k_root_dir/zsh
# instant prompt 缓存目录（对齐 p10k：~/.cache 下）
(( $+__p11k_cache_dir )) || typeset -gr __p11k_cache_dir=${XDG_CACHE_HOME:-$HOME/.cache}/p11k
zmodload zsh/files 2>/dev/null || true

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

# instant prompt 渲染时跳过的动态段（值会变或需子进程，对齐 p10k 的
# instant_prompt_* 缺失即跳过行为）。__p11k_instant=1 时 _p11k_render_line
# 跳过这些段。
typeset -ga __p11k_dynamic_segs=(vcs status command_execution_time background_jobs \
  time load ram swap disk_usage battery todo timewarrior taskwarrior nordvpn kubecontext)

# instant prompt 缓存 sig 去重表（会话内每目录只 dump 一次）
typeset -gA _p11k_dumped_instant_prompt_sigs

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
  if [[ -n $v ]]; then
    # g:: 递归转义：配置里的 \uXXXX（如 \uE0B0 分隔符）转成实际字符
    print -r -- "${(g::)v}"
  else
    print -r -- "${(g::)2}"
  fi
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
    __p11k_out+="%K{$sep_bg}%F{$sep_fg}$(_p11k_p9k POWERLEVEL9K_LEFT_SEGMENT_SEPARATOR '\uE0B0')%f%k"
  fi
  if [[ -n $bg ]]; then
    __p11k_out+="%K{$bg}"
  fi
  if [[ -n $fg ]]; then
    __p11k_out+="%F{$fg}"
  fi
  __p11k_out+="$icon"
  if [[ -n $icon && -n $text ]]; then
    __p11k_out+="$(_p11k_p9k POWERLEVEL9K_LEFT_PROMPT_SEGMENT_END_SYMBOL ' ')"
  fi
  # prompt 解析里 % 是转义符：文本中的 % 必须转义（对齐 p10k 的转义处理）
  __p11k_out+="${text//\%/%%}"
  __p11k_out+="%f%k"
  __p11k_last_bg=$bg
  (( __p11k_seg_count++ ))
}

# ────────────────────────── 分段实现 ──────────────────────────

# 通用"读环境变量即显示"段。
# 参数：<元素名> <环境变量名> <文本> <背景默认> <前景默认> <图标默认>
function _p11k_env_seg() {
  local name=$1 env_var=$2 text=$3 bg_def=$4 fg_def=$5 icon_def=$6
  [[ -n ${(P)env_var} ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_${name:u}_BACKGROUND $bg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_FOREGROUND $fg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_VISUAL_IDENTIFIER_EXPANSION $icon_def) " "$text"
}

# 通用"命令输出即显示"段（每次 precmd 执行一次，失败静默）。
# 参数：<元素名> <命令> <背景默认> <前景默认> <图标默认>
function _p11k_cmd_seg() {
  local name=$1 cmd=$2 bg_def=$3 fg_def=$4 icon_def=$5 text
  text=$($cmd 2>/dev/null) || return
  [[ -n $text ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_${name:u}_BACKGROUND $bg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_FOREGROUND $fg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_VISUAL_IDENTIFIER_EXPANSION $icon_def) " "$text"
}

# anaconda：$CONDA_DEFAULT_ENV
function _p11k_seg_anaconda() {
  _p11k_env_seg anaconda CONDA_DEFAULT_ENV "$CONDA_DEFAULT_ENV" blue 236 '🅔'
}

# nix_shell：$IN_NIX_SHELL
function _p11k_seg_nix_shell() {
  _p11k_env_seg nix_shell IN_NIX_SHELL nix-shell blue 236 '❄️'
}

# ssh：$SSH_CONNECTION 存在时显示（对齐 p10k：显示 user@host 简写）
function _p11k_seg_ssh() {
  [[ -n $SSH_CONNECTION ]] || return
  local text=$(_p11k_p9k POWERLEVEL9K_SSH_TEMPLATE '%n@%m')
  # pattern 里裸 % 是"末尾锚点"，需转义 \%n 才匹配字面 %n（同 context 段）
  text=${text//\%n/$USER}
  text=${text//\%m/${HOST%%.*}}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_SSH_BACKGROUND 238)" \
    "$(_p11k_p9k POWERLEVEL9K_SSH_FOREGROUND 255)" \
    "$(_p11k_p9k POWERLEVEL9K_SSH_VISUAL_IDENTIFIER_EXPANSION '') " "$text"
}

# root_indicator：root 用户
function _p11k_seg_root_indicator() {
  (( EUID == 0 )) || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_ROOT_INDICATOR_BACKGROUND 1)" \
    "$(_p11k_p9k POWERLEVEL9K_ROOT_INDICATOR_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_ROOT_INDICATOR_VISUAL_IDENTIFIER_EXPANSION '❖') " ''
}

# dir_writable：目录不可写时显示
function _p11k_seg_dir_writable() {
  [[ -w $PWD ]] && return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_DIR_WRITABLE_BACKGROUND 1)" \
    "$(_p11k_p9k POWERLEVEL9K_DIR_WRITABLE_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_DIR_WRITABLE_VISUAL_IDENTIFIER_EXPANSION '✗') " ''
}

# ranger/nnn/yazi/lf/xplr：文件管理器嵌套级别
function _p11k_seg_ranger() { _p11k_env_seg ranger RANGER_LEVEL "$RANGER_LEVEL" 238 255 '🗘'; }
function _p11k_seg_nnn() { _p11k_env_seg nnn NNNLVL "$NNNLVL" 238 255 '🗘'; }
function _p11k_seg_yazi() { _p11k_env_seg yazi YAZI_LEVEL "$YAZI_LEVEL" 238 255 '🗘'; }
function _p11k_seg_lf() { _p11k_env_seg lf LF_LEVEL "$LF_LEVEL" 238 255 '🗘'; }
function _p11k_seg_xplr() { _p11k_env_seg xplr XPLR_LEVEL "$XPLR_LEVEL" 238 255 '🗘'; }

# vim_shell：$VIMRUNTIME
function _p11k_seg_vim_shell() {
  _p11k_env_seg vim_shell VIMRUNTIME vim-shell 238 255 ''
}

# midnight_commander：$MC_SID
function _p11k_seg_midnight_commander() {
  _p11k_env_seg midnight_commander MC_SID midnight-commander 238 255 '▶'
}

# proxy：HTTP(S)/ALL_PROXY
function _p11k_seg_proxy() {
  local p=${HTTPS_PROXY:-${https_proxy:-${HTTP_PROXY:-${http_proxy:-${ALL_PROXY:-${all_proxy:-}}}}}}
  [[ -n $p ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_PROXY_BACKGROUND 238)" \
    "$(_p11k_p9k POWERLEVEL9K_PROXY_FOREGROUND 255)" \
    "$(_p11k_p9k POWERLEVEL9K_PROXY_VISUAL_IDENTIFIER_EXPANSION '🌐') " ''
}

# aws：$AWS_PROFILE
function _p11k_seg_aws() {
  _p11k_env_seg aws AWS_PROFILE "$AWS_PROFILE" 208 236 ''
}

# aws_eb_env：$AWS_EB_ENV
function _p11k_seg_aws_eb_env() {
  _p11k_env_seg aws_eb_env AWS_EB_ENV "$AWS_EB_ENV" 2 236 'EB'
}

# google_app_cred：$GOOGLE_APPLICATION_CREDENTIALS
function _p11k_seg_google_app_cred() {
  local cred=$GOOGLE_APPLICATION_CREDENTIALS
  [[ -n $cred ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_GOOGLE_APP_CRED_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_GOOGLE_APP_CRED_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_GOOGLE_APP_CRED_VISUAL_IDENTIFIER_EXPANSION '') " "GOOGLE"
}

# toolbox：$CONTAINER_ID（podman/docker toolbox）
function _p11k_seg_toolbox() {
  _p11k_env_seg toolbox CONTAINER_ID "$CONTAINER_ID" 4 236 '⛵'
}

# nodeenv：$NODE_VIRTUAL_ENV
function _p11k_seg_nodeenv() {
  _p11k_env_seg nodeenv NODE_VIRTUAL_ENV "$NODE_VIRTUAL_ENV:t" 2 236 '⬢'
}

# nvm：$NVM_BIN 存在时显示版本（简化：读 $NVM_DIR 的 alias 不查，直接 nvm 目录名）
function _p11k_seg_nvm() {
  [[ -n $NVM_BIN ]] || return
  local v=''
  [[ -f .nvmrc ]] && v=$(<.nvmrc)
  [[ -z $v ]] && v=node
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_NVM_BACKGROUND 238)" \
    "$(_p11k_p9k POWERLEVEL9K_NVM_FOREGROUND 255)" \
    "$(_p11k_p9k POWERLEVEL9K_NVM_VISUAL_IDENTIFIER_EXPANSION '⬢') " "$v"
}

# rbenv：$RBENV_VERSION
function _p11k_seg_rbenv() {
  _p11k_env_seg rbenv RBENV_VERSION "$RBENV_VERSION" 1 236 '�delay�'
}

# cpu_arch：uname -m
function _p11k_seg_cpu_arch() {
  _p11k_cmd_seg cpu_arch 'uname -m' 4 236 ''
}

# detect_virt：systemd-detect-virt
function _p11k_seg_detect_virt() {
  _p11k_cmd_seg detect_virt 'systemd-detect-virt' 238 255 '🖥️'
}

# docker_machine：$DOCKER_MACHINE_NAME
function _p11k_seg_docker_machine() {
  _p11k_env_seg docker_machine DOCKER_MACHINE_NAME "$DOCKER_MACHINE_NAME" 4 236 '🐳'
}

# gcloud：$CLOUDSDK_ACTIVE_CONFIG_NAME（快路径；完整 gcloud 查询后续）
function _p11k_seg_gcloud() {
  _p11k_env_seg gcloud CLOUDSDK_ACTIVE_CONFIG_NAME "$CLOUDSDK_ACTIVE_CONFIG_NAME" 4 236 '☁️'
}

# terraform：.terraform/environment 文件内容
function _p11k_seg_terraform() {
  local env_file=.terraform/environment
  [[ -f $env_file ]] || return
  local ws=$(<$env_file)
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_TERRAFORM_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_TERRAFORM_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_TERRAFORM_VISUAL_IDENTIFIER_EXPANSION '🛠') " "$ws"
}

# pyenv/goenv/nodenv/jenv/plenv/phpenv/scalaenv/luaenv：version-file
function _p11k_seg_pyenv() { _p11k_cmd_seg pyenv 'pyenv version-name' 4 236 '🐍'; }
function _p11k_seg_goenv() { _p11k_cmd_seg goenv 'goenv version-name' 4 236 '🐹'; }
function _p11k_seg_nodenv() { _p11k_cmd_seg nodenv 'nodenv version-name' 4 236 '⬢'; }
function _p11k_seg_jenv() { _p11k_cmd_seg jenv 'jenv version-name' 4 236 '☕'; }
function _p11k_seg_plenv() { _p11k_cmd_seg plenv 'plenv version-name' 4 236 '🐧'; }
function _p11k_seg_phpenv() { _p11k_cmd_seg phpenv 'phpenv version-name' 4 236 '🐘'; }
function _p11k_seg_scalaenv() { _p11k_cmd_seg scalaenv 'scalaenv version-name' 4 236 '🗬'; }
function _p11k_seg_luaenv() { _p11k_cmd_seg luaenv 'luaenv version-name' 4 236 '🌙'; }

# 通用"慢命令 + 60s 缓存"段：版本类段用（node --version 等每次
# precmd 调用太慢，p10k 同样有段级缓存）。
function _p11k_cmd_seg_cached() {
  local name=$1 cmd=$2 bg_def=$3 fg_def=$4 icon_def=$5
  local cache_var=__p11k_cache_${name:u}
  local -a cache
  # (=P)：间接展开并强制分词（(@P) 在引号内不分词，cache[1] 会变成整串）
  cache=("${(P)=cache_var}")
  if (( ${+cache[1]} == 0 || EPOCHSECONDS - cache[1] > 60 )); then
    local out
    out=$($cmd 2>/dev/null)
    typeset -g $cache_var="$EPOCHSECONDS $out"
    cache=("${(P)=cache_var}")
  fi
  [[ -n ${cache[2]} ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_${name:u}_BACKGROUND $bg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_FOREGROUND $fg_def)" \
    "$(_p11k_p9k POWERLEVEL9K_${name:u}_VISUAL_IDENTIFIER_EXPANSION $icon_def) " "${cache[2]}"
}

# 版本段（命令输出修剪后显示）
function _p11k_seg_node_version() { _p11k_cmd_seg_cached node_version 'node --version' 2 236 '⬢'; }
function _p11k_seg_go_version() { _p11k_cmd_seg_cached go_version 'go version | sed "s/go version //"' 4 236 '🐹'; }
function _p11k_seg_rust_version() { _p11k_cmd_seg_cached rust_version 'rustc --version | cut -d" " -f2' 208 236 '🦀'; }
function _p11k_seg_java_version() { _p11k_cmd_seg_cached java_version 'java -version 2>&1 | head -1 | cut -d"\"" -f2' 208 236 '☕'; }
function _p11k_seg_php_version() { _p11k_cmd_seg_cached php_version 'php --version | head -1 | cut -d" " -f2' 5 236 '🐘'; }
function _p11k_seg_dotnet_version() { _p11k_cmd_seg_cached dotnet_version 'dotnet --version' 5 236 '🥅'; }
function _p11k_seg_terraform_version() { _p11k_cmd_seg_cached terraform_version 'terraform version | head -1 | cut -d" " -f2' 4 236 '🛠'; }

# kubecontext：kubectl 当前上下文（60s 缓存；支持 p10k 的
# POWERLEVEL9K_KUBECONTEXT_DEFAULT_CONTENT_EXPANSION，模板内可用
# P9K_CONTENT / P9K_KUBECONTEXT_NAME / P9K_KUBECONTEXT_CLOUD_CLUSTER /
# P9K_KUBECONTEXT_NAMESPACE）。
function _p11k_seg_kubecontext() {
  local cache_var=__p11k_cache_kubecontext
  local -a cache
  cache=("${(P)=cache_var}")
  if (( ${+cache[1]} == 0 || EPOCHSECONDS - cache[1] > 60 )); then
    local ctx='' cluster='' ns=''
    ctx=$(kubectl config current-context 2>/dev/null)
    if [[ -n $ctx ]]; then
      # 从 kubeconfig 的 contexts 段解析 cluster/namespace（简化版；
      # 只读 ${KUBECONFIG:-~/.kube/config} 第一个文件）
      local kc=${KUBECONFIG%%:*}
      [[ -n $kc ]] || kc=$HOME/.kube/config
      if [[ -f $kc ]]; then
        local pair
        pair=$(awk -v want="$ctx" '
          function flush() { if (name == want) { print cluster; print ns; name = "" } }
          /^- context:/ { flush(); in_block=1; cluster=""; ns=""; name=""; next }
          in_block && $1 == "name:" { name=$2; next }
          in_block && $1 == "cluster:" { cluster=$2; next }
          in_block && $1 == "namespace:" { ns=$2; next }
          in_block && $0 !~ /^[[:space:]-]/ { in_block=0 }
          END { flush() }' "$kc" 2>/dev/null)
        cluster=${pair%%$'\n'*}
        ns=${pair#*$'\n'}
        [[ $ns == $pair ]] && ns=''
      fi
    fi
    typeset -g $cache_var="$EPOCHSECONDS $ctx $cluster $ns"
    cache=("${(P)=cache_var}")
  fi
  [[ -n ${cache[2]} ]] || return
  local P9K_CONTENT=${cache[2]}
  local P9K_KUBECONTEXT_NAME=${cache[2]}
  local P9K_KUBECONTEXT_CLOUD_CLUSTER=${cache[3]}
  local P9K_KUBECONTEXT_NAMESPACE=${cache[4]}
  local text=$(_p11k_p9k POWERLEVEL9K_KUBECONTEXT_DEFAULT_CONTENT_EXPANSION '${P9K_CONTENT}')
  text=${(e)text}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_KUBECONTEXT_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_KUBECONTEXT_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_KUBECONTEXT_VISUAL_IDENTIFIER_EXPANSION '⎈') " "$text"
}

# 系统段（读 /proc，快，无缓存）：
# load：/proc/loadavg 第一字段
function _p11k_seg_load() {
  local -a la
  la=("${(@f)$(</proc/loadavg 2>/dev/null)}")
  [[ -n $la[1] ]] || return
  local text=${la[1]%% *}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_LOAD_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_LOAD_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_LOAD_VISUAL_IDENTIFIER_EXPANSION '') " "$text"
}

# ram：MemAvailable/MemTotal 百分比
function _p11k_seg_ram() {
  local total avail
  total=$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null)
  avail=$(awk '/MemAvailable/ {print $2}' /proc/meminfo 2>/dev/null)
  [[ -n $total && -n $avail ]] || return
  local pct=$(( (total - avail) * 100 / total ))
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_RAM_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_RAM_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_RAM_VISUAL_IDENTIFIER_EXPANSION '') " "${pct}%"
}

# swap：SwapUsed/SwapTotal 百分比
function _p11k_seg_swap() {
  local used total
  used=$(awk '/SwapUsed/ {print $2}' /proc/meminfo 2>/dev/null)
  total=$(awk '/SwapTotal/ {print $2}' /proc/meminfo 2>/dev/null)
  [[ -n $used && -n $total && $total -gt 0 ]] || return
  local pct=$(( used * 100 / total ))
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_SWAP_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_SWAP_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_SWAP_VISUAL_IDENTIFIER_EXPANSION '易') " "${pct}%"
}

# disk_usage：当前分区使用率
function _p11k_seg_disk_usage() {
  local pct
  pct=$(df -h . 2>/dev/null | awk 'NR==2 {gsub("%","",$5); print $5}')
  [[ -n $pct ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_DISK_USAGE_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_DISK_USAGE_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_DISK_USAGE_VISUAL_IDENTIFIER_EXPANSION '') " "${pct}%"
}

# battery：/sys/class/power_supply（笔记本；无电池时跳过）
function _p11k_seg_battery() {
  local cap
  cap=$(cat /sys/class/power_supply/BAT*/capacity 2>/dev/null | head -1)
  [[ -n $cap ]] || return
  local icon=''
  (( cap < 20 )) && icon=''
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_BATTERY_LOW_BACKGROUND 208)" \
    "$(_p11k_p9k POWERLEVEL9K_BATTERY_LOW_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_BATTERY_LOW_VISUAL_IDENTIFIER_EXPANSION $icon) " "${cap}%"
}

# package：package.json 的 version
function _p11k_seg_package() {
  [[ -f package.json ]] || return
  local v
  v=$(awk -F'"' '/"version"/ {print $4; exit}' package.json 2>/dev/null)
  [[ -n $v ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_PACKAGE_BACKGROUND 208)" \
    "$(_p11k_p9k POWERLEVEL9K_PACKAGE_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_PACKAGE_VISUAL_IDENTIFIER_EXPANSION '📦') " "$v"
}

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
  # 目录截断（对齐 p10k 的 SHORTEN_STRATEGY）
  local strategy=$(_p11k_p9k POWERLEVEL9K_SHORTEN_STRATEGY truncate_to_last)
  local delim=$(_p11k_p9k POWERLEVEL9K_SHORTEN_DELIMITER '..')
  local -a parts
  case $strategy in
    truncate_to_last)
      dir=${dir:t};;  # 只显示最后一段
    truncate_from_right)
      parts=("${(@s:/:)dir}")
      if (( ${#parts} > ${POWERLEVEL9K_SHORTEN_DIR_LENGTH:-3} + 1 )); then
        dir="$delim/${parts[-1]}"
      fi;;
    truncate_middle)
      parts=("${(@s:/:)dir}")
      if (( ${#parts} > ${POWERLEVEL9K_SHORTEN_DIR_LENGTH:-1} + 1 )); then
        dir="${(j:/:)parts[1,${POWERLEVEL9K_SHORTEN_DIR_LENGTH:-1}]}/$delim/${parts[-1]}"
      fi;;
  esac
  local icon=$(_p11k_p9k POWERLEVEL9K_DIR_ICON '')
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_DIR_BACKGROUND blue)" \
    "$(_p11k_p9k POWERLEVEL9K_DIR_FOREGROUND 236)" "$icon" "$dir"
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

# direnv：$DIRENV_DIR（direnv 激活时设置，值含前导 /）
function _p11k_seg_direnv() {
  [[ -n $DIRENV_DIR ]] || return
  local dir=${DIRENV_DIR#/}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_DIRENV_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_DIRENV_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_DIRENV_VISUAL_IDENTIFIER_EXPANSION '\uf0c9') " "${dir:t}"
}

# asdf：$ASDF_DIR 或 .tool-versions 存在；显示首个工具名@版本
function _p11k_seg_asdf() {
  [[ -n $ASDF_DIR || -f .tool-versions ]] || return
  local v=''
  [[ -f .tool-versions ]] && v=$(awk 'NF && $1 !~ /^#/ {print $1"@"$2; exit}' .tool-versions)
  [[ -z $v ]] && v=asdf
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_ASDF_BACKGROUND 7)" \
    "$(_p11k_p9k POWERLEVEL9K_ASDF_FOREGROUND 237)" \
    "$(_p11k_p9k POWERLEVEL9K_ASDF_VISUAL_IDENTIFIER_EXPANSION '\uf4c2') " "$v"
}

# rvm：$GEM_HOME（gemset 路径末段形如 ruby-3.2.2）
function _p11k_seg_rvm() {
  [[ -n $GEM_HOME ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_RVM_BACKGROUND 1)" \
    "$(_p11k_p9k POWERLEVEL9K_RVM_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_RVM_VISUAL_IDENTIFIER_EXPANSION '\ue21e') " "${GEM_HOME:t}"
}

# fvm：.fvmrc 存在（flutter 版本，形如 flutter: 3.24.0）
function _p11k_seg_fvm() {
  [[ -f .fvmrc ]] || return
  local v=$(awk '/flutter/ {print $2}' .fvmrc)
  [[ -z $v ]] && v=fvm
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_FVM_BACKGROUND 7)" \
    "$(_p11k_p9k POWERLEVEL9K_FVM_FOREGROUND 237)" \
    "$(_p11k_p9k POWERLEVEL9K_FVM_VISUAL_IDENTIFIER_EXPANSION '\uf8b0') " "$v"
}

# perlbrew：$PERLBREW_PERL（形如 perl-5.36.0）
function _p11k_seg_perlbrew() {
  [[ -n $PERLBREW_PERL ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_PERLBREW_BACKGROUND 2)" \
    "$(_p11k_p9k POWERLEVEL9K_PERLBREW_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_PERLBREW_VISUAL_IDENTIFIER_EXPANSION '\ue739') " "${PERLBREW_PERL#perl-}"
}

# haskell_stack：.stack.yaml 存在
function _p11k_seg_haskell_stack() {
  [[ -f .stack.yaml ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_HASKELL_STACK_BACKGROUND 3)" \
    "$(_p11k_p9k POWERLEVEL9K_HASKELL_STACK_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_HASKELL_STACK_VISUAL_IDENTIFIER_EXPANSION '\u03bb') " 'stack'
}

# azure：az 登录态（~/.azure/azureProfile.json 的订阅名）
function _p11k_seg_azure() {
  local profile=${AZURE_CONFIG_DIR:-$HOME/.azure}/azureProfile.json
  [[ -f $profile ]] || return
  local name=$(grep -o '"name": *"[^"]*"' "$profile" 2>/dev/null | head -1 | sed 's/.*"name": *"//; s/"$//')
  [[ -z $name ]] && name=azure
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_AZURE_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_AZURE_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_AZURE_VISUAL_IDENTIFIER_EXPANSION '\uf0c2') " "$name"
}

# nordvpn：nordvpnd 守护进程运行中（状态简化：Connected/Disconnected）
function _p11k_seg_nordvpn() {
  (( $+commands[nordvpn] )) || return
  local state=Disconnected
  pgrep -x nordvpnd >/dev/null && state=Connected
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_NORDVPN_BACKGROUND 6)" \
    "$(_p11k_p9k POWERLEVEL9K_NORDVPN_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_NORDVPN_VISUAL_IDENTIFIER_EXPANSION '\uf023') " "$state"
}

# chezmoi_shell：$CHEZMOI（chezmoi activate 时设置）
function _p11k_seg_chezmoi_shell() {
  _p11k_env_seg chezmoi_shell CHEZMOI "$CHEZMOI" 4 236 '\uf015'
}

# todo：todo.txt 未完成条目数（~/.todo/todo.txt，'x ' 前缀视为完成）
function _p11k_seg_todo() {
  (( $+commands[todo.sh] )) || return
  local n=0
  [[ -f $HOME/.todo/todo.txt ]] && n=$(grep -cv '^x ' "$HOME/.todo/todo.txt" 2>/dev/null)
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_TODO_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_TODO_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_TODO_VISUAL_IDENTIFIER_EXPANSION '\u2713') " "$n"
}

# timewarrior：timew 已安装（不查状态，避免每次 prompt 起子进程）
function _p11k_seg_timewarrior() {
  (( $+commands[timew] )) || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_TIMEWARRIOR_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_TIMEWARRIOR_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_TIMEWARRIOR_VISUAL_IDENTIFIER_EXPANSION '\uf017') " 'timew'
}

# taskwarrior：task 已安装（不查计数，避免每次 prompt 起子进程）
function _p11k_seg_taskwarrior() {
  (( $+commands[task] )) || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_TASKWARRIOR_BACKGROUND 6)" \
    "$(_p11k_p9k POWERLEVEL9K_TASKWARRIOR_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_TASKWARRIOR_VISUAL_IDENTIFIER_EXPANSION '\u2713') " 'task'
}

# per_directory_history：$PER_DIRECTORY_HISTORY_TOGGLED（local/global）
function _p11k_seg_per_directory_history() {
  [[ -n $PER_DIRECTORY_HISTORY_TOGGLED ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_PER_DIRECTORY_HISTORY_BACKGROUND 1)" \
    "$(_p11k_p9k POWERLEVEL9K_PER_DIRECTORY_HISTORY_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_PER_DIRECTORY_HISTORY_VISUAL_IDENTIFIER_EXPANSION '\uf5d7') " "$PER_DIRECTORY_HISTORY_TOGGLED"
}

function _p11k_seg_prompt_char() {
  local fg bg char mode
  # vi 模式（zle-keymap-select 钩子维护；非 vi 用户恒为 0）
  mode=${__p11k_vi_mode:-0}
  if (( __p11k_last_status == 0 )); then
    if (( mode )); then
      fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VICMD_FOREGROUND green)
      char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VICMD_CONTENT_EXPANSION '❮')
    else
      fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_FOREGROUND green)
      char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_CONTENT_EXPANSION '❯')
    fi
  else
    if (( mode )); then
      fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VICMD_FOREGROUND red)
      char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VICMD_CONTENT_EXPANSION '❮')
    else
      fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VIINS_FOREGROUND red)
      char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_ERROR_VIINS_CONTENT_EXPANSION '❯')
    fi
  fi
  bg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_BACKGROUND default)
  _p11k_prompt_segment "$bg" "$fg" '' "$char"
  __p11k_prompt_char_fg=$fg
}

# 独立的 vi_mode 段（用户配置含 vi_mode 元素时显示）
function _p11k_seg_vi_mode() {
  local mode=${__p11k_vi_mode:-0}
  if (( mode )); then
    _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_VI_MODE_NORMAL_BACKGROUND 236)" \
      "$(_p11k_p9k POWERLEVEL9K_VI_MODE_NORMAL_FOREGROUND 255)" \
      "$(_p11k_p9k POWERLEVEL9K_VI_MODE_NORMAL_VISUAL_IDENTIFIER_EXPANSION 'N')" 'NORMAL'
  else
    _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_VI_MODE_INSERT_BACKGROUND 238)" \
      "$(_p11k_p9k POWERLEVEL9K_VI_MODE_INSERT_FOREGROUND 255)" \
      "$(_p11k_p9k POWERLEVEL9K_VI_MODE_INSERT_VISUAL_IDENTIFIER_EXPANSION 'I')" 'INSERT'
  fi
}

# context：user@host（与 DEFAULT_USER 相同时不显示，对齐 p10k）
function _p11k_seg_context() {
  # user/host 在段内展开为实际值（不依赖 prompt 的 %n/%m 转义，
  # 否则会被 _p11k_prompt_segment 的 % 转义破坏成 %%n）
  local user=${USER:-$(id -un)}
  local host=${HOST%%.*}
  [[ $user == ${DEFAULT_USER:-} ]] && return
  local icon=$(_p11k_p9k POWERLEVEL9K_CONTEXT_DEFAULT_VISUAL_IDENTIFIER_EXPANSION '')
  local text
  if [[ $user == root ]]; then
    text=$(_p11k_p9k POWERLEVEL9K_CONTEXT_ROOT_TEMPLATE '%n@%m')
  else
    text=$(_p11k_p9k POWERLEVEL9K_CONTEXT_TEMPLATE '%n@%m')
  fi
  # zsh 的 // 替换中 % 需转义（\%n 才匹配字面 %n）
  text=${text//\%n/$user}
  text=${text//\%m/$host}
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_CONTEXT_BACKGROUND 238)" \
    "$(_p11k_p9k POWERLEVEL9K_CONTEXT_FOREGROUND 255)" "$icon" "$text"
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
  local thresh=$(_p11k_p9k POWERLEVEL9K_COMMAND_EXECUTION_TIME_THRESHOLD 3)
  (( __p11k_last_exec_time >= thresh )) || return
  # 格式化（对齐 p10k：>=1h 显示 h/m/s，>=1m 显示 m/s，否则秒）
  local s=${__p11k_last_exec_time%.*}
  local text
  if (( s >= 3600 )); then
    text="$(( s / 3600 ))h $(( (s % 3600) / 60 ))m $(( s % 60 ))s"
  elif (( s >= 60 )); then
    text="$(( s / 60 ))m $(( s % 60 ))s"
  else
    text="${__p11k_last_exec_time}s"
  fi
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

# custom 段（对齐 p10k：元素 custom_<name>，配置 POWERLEVEL9K_CUSTOM_<name>
# 是命令，输出即段文本。颜色走 POWERLEVEL9K_CUSTOM_<name>_{BACKGROUND,FOREGROUND}）
function _p11k_seg_custom() {
  local name=${1#custom_}
  local upper=${name:u}
  local cmd=$(_p11k_p9k POWERLEVEL9K_CUSTOM_${upper} '')
  [[ -n $cmd ]] || return
  local text
  text=$(eval "$cmd" 2>/dev/null) || return
  [[ -n $text ]] || return
  _p11k_prompt_segment "$(_p11k_p9k POWERLEVEL9K_CUSTOM_${upper}_BACKGROUND 4)" \
    "$(_p11k_p9k POWERLEVEL9K_CUSTOM_${upper}_FOREGROUND 236)" \
    "$(_p11k_p9k POWERLEVEL9K_CUSTOM_${upper}_VISUAL_IDENTIFIER_EXPANSION '') " "$text"
}

# vcs：git 状态（数据来自 p11k-d，见 zsh/gitstatus.zsh）
function _p11k_seg_vcs() {
  # __p11k_vcs_* 由 gitstatus 异步回调填充；尚未就绪时跳过
  (( ${+__p11k_vcs_ready} )) || return
  local text=$__p11k_vcs_branch
  # detached/tag：无分支时用 tag 名
  [[ -n $text ]] || text=$__p11k_vcs_tag
  [[ -n $text ]] || return
  # ahead/behind（对齐 p10k 的 ↑n↓n）
  if (( __p11k_vcs_ahead > 0 || __p11k_vcs_behind > 0 )); then
    (( __p11k_vcs_ahead > 0 )) && text+=" ↑$__p11k_vcs_ahead"
    (( __p11k_vcs_behind > 0 )) && text+=" ↓$__p11k_vcs_behind"
  fi
  # stash
  (( __p11k_vcs_stashes > 0 )) && text+=" $(_p11k_p9k POWERLEVEL9K_VCS_STASH_ICON '⬤')$__p11k_vcs_stashes"
  # dirty 细分：unstaged/untracked（对齐 p10k 的 !n ?n）
  local bg fg
  if (( __p11k_vcs_dirty )); then
    bg=$(_p11k_p9k POWERLEVEL9K_VCS_MODIFIED_BACKGROUND yellow)
    fg=$(_p11k_p9k POWERLEVEL9K_VCS_MODIFIED_FOREGROUND 236)
    (( __p11k_vcs_unstaged > 0 )) &&
      text+=" $(_p11k_p9k POWERLEVEL9K_VCS_UNSTAGED_ICON '!')$__p11k_vcs_unstaged"
    (( __p11k_vcs_untracked > 0 )) &&
      text+=" $(_p11k_p9k POWERLEVEL9K_VCS_UNTRACKED_ICON '?')$__p11k_vcs_untracked"
  else
    bg=$(_p11k_p9k POWERLEVEL9K_VCS_CLEAN_BACKGROUND green)
    fg=$(_p11k_p9k POWERLEVEL9K_VCS_CLEAN_FOREGROUND 236)
  fi
  _p11k_prompt_segment "$bg" "$fg" \
    "$(_p11k_p9k POWERLEVEL9K_VCS_GIT_ICON '') " "$text"
  :
}

# ────────────────────────── 布局与渲染 ──────────────────────────

# 按元素名渲染一行。
# 参数：<元素名数组>；输出到 __p11k_out。
function _p11k_render_line() {
  local name
  for name in "$@"; do
    # instant 渲染：跳过动态段（vcs/time/状态等）
    (( ${+__p11k_instant} )) && [[ ${__p11k_dynamic_segs[(I)$name]} != 0 ]] && continue
    case $name in
      newline) continue;;
      os_icon) _p11k_seg_os_icon;;
      dir) _p11k_seg_dir;;
      anaconda) _p11k_seg_anaconda;;
      nix_shell) _p11k_seg_nix_shell;;
      ssh) _p11k_seg_ssh;;
      root_indicator) _p11k_seg_root_indicator;;
      dir_writable) _p11k_seg_dir_writable;;
      ranger) _p11k_seg_ranger;;
      nnn) _p11k_seg_nnn;;
      yazi) _p11k_seg_yazi;;
      lf) _p11k_seg_lf;;
      xplr) _p11k_seg_xplr;;
      vim_shell) _p11k_seg_vim_shell;;
      midnight_commander) _p11k_seg_midnight_commander;;
      proxy) _p11k_seg_proxy;;
      aws) _p11k_seg_aws;;
      aws_eb_env) _p11k_seg_aws_eb_env;;
      google_app_cred) _p11k_seg_google_app_cred;;
      toolbox) _p11k_seg_toolbox;;
      nodeenv) _p11k_seg_nodeenv;;
      nvm) _p11k_seg_nvm;;
      rbenv) _p11k_seg_rbenv;;
      cpu_arch) _p11k_seg_cpu_arch;;
      detect_virt) _p11k_seg_detect_virt;;
      docker_machine) _p11k_seg_docker_machine;;
      gcloud) _p11k_seg_gcloud;;
      terraform) _p11k_seg_terraform;;
      pyenv) _p11k_seg_pyenv;;
      goenv) _p11k_seg_goenv;;
      nodenv) _p11k_seg_nodenv;;
      jenv) _p11k_seg_jenv;;
      plenv) _p11k_seg_plenv;;
      phpenv) _p11k_seg_phpenv;;
      scalaenv) _p11k_seg_scalaenv;;
      luaenv) _p11k_seg_luaenv;;
      node_version) _p11k_seg_node_version;;
      go_version) _p11k_seg_go_version;;
      rust_version) _p11k_seg_rust_version;;
      java_version) _p11k_seg_java_version;;
      php_version) _p11k_seg_php_version;;
      dotnet_version) _p11k_seg_dotnet_version;;
      terraform_version) _p11k_seg_terraform_version;;
      direnv) _p11k_seg_direnv;;
      asdf) _p11k_seg_asdf;;
      rvm) _p11k_seg_rvm;;
      fvm) _p11k_seg_fvm;;
      perlbrew) _p11k_seg_perlbrew;;
      haskell_stack) _p11k_seg_haskell_stack;;
      azure) _p11k_seg_azure;;
      nordvpn) _p11k_seg_nordvpn;;
      chezmoi_shell) _p11k_seg_chezmoi_shell;;
      todo) _p11k_seg_todo;;
      timewarrior) _p11k_seg_timewarrior;;
      taskwarrior) _p11k_seg_taskwarrior;;
      per_directory_history) _p11k_seg_per_directory_history;;
      kubecontext) _p11k_seg_kubecontext;;
      load) _p11k_seg_load;;
      ram) _p11k_seg_ram;;
      swap) _p11k_seg_swap;;
      disk_usage) _p11k_seg_disk_usage;;
      battery) _p11k_seg_battery;;
      package) _p11k_seg_package;;
      custom_*) _p11k_seg_custom "$name";;
      status) _p11k_seg_status;;
      prompt_char) _p11k_seg_prompt_char;;
      vi_mode) _p11k_seg_vi_mode;;
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

# _p11k_pad <left> <right>：右对齐拼接（gap 空格填充；宽度用 sed 剥
# %{...%} 与 %K{}/%F{}/%B{} 转义后估算，CJK 按 1 列计，近似宽度）。
function _p11k_pad() {
  emulate -L zsh
  local lw=${#$(print -rn -- "$1" | sed -E 's/%\{[^}]*\}//g; s/%[KFB]\{[^}]*\}//g')}
  local rw=${#$(print -rn -- "$2" | sed -E 's/%\{[^}]*\}//g; s/%[KFB]\{[^}]*\}//g')}
  local gap=$(( COLUMNS - lw - rw ))
  local out=$1
  (( gap > 0 )) && out+="${(l:$gap:: :)}"
  out+=$2
  print -rn -- "$out"
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
  # 两行布局：第一行 left+right（RPROMPT 原生右对齐），
  # 第二行 left2 + right2（right2 用 gap 空格填充右对齐）。
  PROMPT="$left_prompt$POWERLEVEL9K_PROMPT_ADD_NEWLINE_PREFIX"
  RPROMPT="$right_prompt"
  if (( ${#line2} > 0 )); then
    # 第二行 left
    __p11k_out=''
    __p11k_seg_count=0
    __p11k_last_bg='default'
    _p11k_render_line "${line2[@]}"
    local left2=$__p11k_out
    # 第二行 right（right 元素里 newline 之后的部分）
    local right2=''
    local -a right2_elems=()
    local r_split=0 r_name
    for r_name in "${right[@]}"; do
      if [[ $r_name == newline ]]; then
        r_split=1
        continue
      fi
      (( r_split )) && right2_elems+=("$r_name")
    done
    if (( ${#right2_elems} > 0 )); then
      __p11k_out=''
      __p11k_seg_count=0
      __p11k_last_bg='default'
      _p11k_render_line "${right2_elems[@]}"
      right2=$__p11k_out
    fi
    if [[ -n $right2 ]]; then
      left2=$(_p11k_pad "$left2" "$right2")
    fi
    PROMPT+="
$left2"
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
  __p11k_last_pwd=$PWD
  # gitstatus：异步查询当前目录（失败不传播，避免钩子返回非零）
  (( ${+functions[_p11k_gitstatus_query]} )) && _p11k_gitstatus_query || true
  _p11k_prompt
  # instant prompt 缓存：首次渲染后写入（下次启动秒显）
  _p11k_maybe_dump_instant_prompt
}

# preexec：记录命令开始时间
function _p11k_preexec() {
  emulate -L zsh
  __p11k_last_cmd_time=$EPOCHREALTIME
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd _p11k_precmd
add-zsh-hook preexec _p11k_preexec

# ────────────────────────── instant prompt（对齐 p10k） ──────────────────────────
# 机制（复刻 p10k）：
#   生成：precmd 首次渲染后同步 dump（无 p10k 的 zle -F 调度，简化）；
#        内容文件按 PWD 长度分文件，文件内多条记录用  分隔、
#        key=PWD:ssh:root 匹配（对齐 p10k 的 prompt-${#pwd} 布局）。
#   显示：用户 .zshrc 顶部 source 缓存模板（或本主题末尾兜底 source）。
#        模板校验 tty/交互/zle 后打印 instant prompt 文本（terminfo[sc]
#        保存光标），然后把 stdout 重定向进临时文件（吞掉 .zshrc 后续
#        输出），unsetopt prompt_cr prompt_sp + DISABLE_UPDATE_PROMPT=true
#        防止 zsh/oh-my-zsh 重绘 prompt 覆盖。
#   清理：precmd 首钩子 sched +0 -> cleanup：恢复 fd、terminfo[rc] 恢复
#        光标 + terminfo[ed] 清屏到末尾、cat 重放被吞的输出、
#        setopt prompt_cr prompt_sp。

# instant 版渲染：三段式（对齐 p10k 的 _p9k_set_instant_prompt）：
#   _p11k__instant_prompt = 第一行(含换行)  第二行(含 gap 右对齐) 
# 与 _p11k_prompt 的拆行/渲染逻辑保持同步；动态段被 __p11k_dynamic_segs 过滤。
function _p11k_set_instant_prompt() {
  emulate -L zsh
  local -a left right line1 line2
  left=("${POWERLEVEL9K_LEFT_PROMPT_ELEMENTS[@]}")
  right=("${POWERLEVEL9K_RIGHT_PROMPT_ELEMENTS[@]}")
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
  __p11k_instant=1
  __p11k_out=''
  __p11k_seg_count=0
  __p11k_last_bg=default
  _p11k_render_line "${line1[@]}"
  local l1=$__p11k_out
  local left2=''
  if (( ${#line2} > 0 )); then
    __p11k_out=''
    __p11k_seg_count=0
    __p11k_last_bg=default
    _p11k_render_line "${line2[@]}"
    left2=$__p11k_out
  fi
  # 第二行右侧（RIGHT 里 newline 之后的元素）
  local right2=''
  local -a right2_elems=()
  local r_split=0 r_name
  for r_name in "${right[@]}"; do
    if [[ $r_name == newline ]]; then
      r_split=1
      continue
    fi
    (( r_split )) && right2_elems+=("$r_name")
  done
  if (( ${#right2_elems} > 0 )); then
    __p11k_out=''
    __p11k_seg_count=0
    __p11k_last_bg=default
    _p11k_render_line "${right2_elems[@]}"
    right2=$__p11k_out
  fi
  __p11k_instant=0
  if [[ -n $right2 ]]; then
    left2=$(_p11k_pad "$left2" "$right2")
  fi
  # 注意：$'...' 在双引号字符串内是字面文本，必须先赋值给变量
  local lf=$'\n' us=$'\x1f'
  _p11k__instant_prompt="$l1$POWERLEVEL9K_PROMPT_ADD_NEWLINE_PREFIX$lf$left2$POWERLEVEL9K_PROMPT_ADD_NEWLINE_SUFFIX $us"
}

# 写缓存：root_file（一次性模板）+ prompt_file（按 key 追加记录）
function _p11k_dump_instant_prompt() {
  emulate -L zsh
  local user=${(%):-%n}
  local root_dir=$__p11k_cache_dir
  local prompt_dir=$root_dir/p11k-$user
  local root_file=$root_dir/p11k-instant-prompt-$user.zsh
  local prompt_file=$prompt_dir/prompt-${#PWD}
  [[ -d $prompt_dir ]] || mkdir -p $prompt_dir || return 1
  [[ -w $root_dir && -w $prompt_dir ]] || return 1
  if [[ ! -e $root_file ]]; then
    local tmp=$root_file.tmp.$$
    {
      cat >$tmp <<'P11K_EOF'
[[ -t 0 && -t 1 && -t 2 && -o interactive && -o zle && -o no_xtrace ]] || return 0
() {
  # 防重复 source（主题兜底 source 时）
  (( ${+__p11k_instant_prompt_sourced} || ${+__p11k_instant_prompt_active} )) && return
  [[ $POWERLEVEL9K_INSTANT_PROMPT != off && $POWERLEVEL9K_DISABLE_INSTANT_PROMPT != true ]] || return
  zmodload zsh/langinfo zsh/terminfo zsh/system 2>/dev/null || return
  (( terminfo[colors] >= 8 )) || return
  (( $+terminfo[sc] && $+terminfo[rc] && $+terminfo[ed] )) || return
  local user=${(%):-%n}
  local pwd=${(%):-%/}
  [[ $pwd == /* ]] || return
  local prompt_dir=${XDG_CACHE_HOME:-$HOME/.cache}/p11k/p11k-$user
  local prompt_file=$prompt_dir/prompt-${#pwd}
  local rs=$'' us=$''
  local key=$pwd:${${SSH_CONNECTION:+1}:-0}:${(%):-%#}
  local content
  { content="$(<$prompt_file)" } 2>/dev/null || return
  local tail=${content##*$rs$key$us}
  (( ${#tail} == ${#content} )) && return
  local -a t=("${(@ps:$us:)${tail%%$rs*}}")
  (( $#t >= 2 )) || return
  local cr=$'
' lf=$'
' esc=$'\e['
  local -i height=${POWERLEVEL9K_INSTANT_PROMPT_COMMAND_LINES:-1}
  local -i prompt_height=${#${t[1]//[^$lf]}}
  (( height += prompt_height ))
  local out=${(%):-%b%k%f%s%u}
  out+="${(%):-$cr%E}"
  (( height )) && out+="${(pl.$height..$lf.)}$esc${height}A"
  out+="$terminfo[sc]"
  out+=${(%):-"$t[1]$t[2]"}
  print -rn -- "${out}${esc}?2004h" || return
  if (( $+commands[stty] )); then
    command stty -icanon 2>/dev/null
  fi
  local output=${TMPDIR:-/tmp}/p11k-instant-prompt-output-${(%):-%n}-$$
  : > $output 2>/dev/null || return
  local fd_null
  sysopen -ru fd_null /dev/null || return
  exec {__p11k_fd_0}<&0 {__p11k_fd_1}>&1 {__p11k_fd_2}>&2 0<&$fd_null 1>$output
  exec 2>&1 {fd_null}>&-
  typeset -g __p11k_instant_prompt_active=1
  typeset -g __p11k_instant_prompt_output=$output
  function _p11k_instant_prompt_cleanup() {
    (( ZSH_SUBSHELL == 0 && ${+__p11k_instant_prompt_active} )) || return 0
    unset __p11k_instant_prompt_active
    exec 0<&$__p11k_fd_0 1>&$__p11k_fd_1 2>&$__p11k_fd_2 {__p11k_fd_0}>&- {__p11k_fd_1}>&- {__p11k_fd_2}>&-
    unset __p11k_fd_0 __p11k_fd_1 __p11k_fd_2
    print -rn -- $terminfo[rc]${(%):-%b%k%f%s%u}$terminfo[ed]
    if [[ -s $__p11k_instant_prompt_output ]]; then
      command cat $__p11k_instant_prompt_output 2>/dev/null
    fi
    zshexit_functions=(${zshexit_functions:#_p11k_instant_prompt_cleanup})
    zmodload -F zsh/files b:zf_rm 2>/dev/null
    zf_rm -f -- $__p11k_instant_prompt_output 2>/dev/null
  }
  function _p11k_instant_prompt_precmd_first() {
    function _p11k_instant_prompt_sched_last() {
      (( ${+__p11k_instant_prompt_active} )) || return 0
      _p11k_instant_prompt_cleanup 1
      setopt no_local_options prompt_cr prompt_sp
    }
    zmodload zsh/sched 2>/dev/null
    sched +0 _p11k_instant_prompt_sched_last
    precmd_functions=(${(@)precmd_functions:#_p11k_instant_prompt_precmd_first})
  }
  zshexit_functions=(_p11k_instant_prompt_cleanup $zshexit_functions)
  precmd_functions=(_p11k_instant_prompt_precmd_first $precmd_functions)
  DISABLE_UPDATE_PROMPT=true
  typeset -gi __p11k_instant_prompt_sourced=1
} && unsetopt prompt_cr prompt_sp || true
P11K_EOF
    } 2>/dev/null || return 1
    zf_mv -f -- $tmp $root_file || return 1
  fi
  # 内容文件：同 key 记录已存在则清空重写（防无限膨胀）
  local sig=$PWD:${${SSH_CONNECTION:+1}:-0}:${(%):-%#}
  local tmp=$prompt_file.tmp.$$
  zf_mv -f -- $prompt_file $tmp 2>/dev/null
  if [[ "$(<$tmp)" == *$'\x1e'$sig$'\x1f'* ]] 2>/dev/null; then
    echo -n >$tmp || return 1
  fi
  print -rn -- $'\x1e'$sig$'\x1f'$_p11k__instant_prompt >>$tmp || return 1
  zf_mv -f -- $tmp $prompt_file || return 1
}

# precmd 里调用：会话内每 sig 只 dump 一次；instant 已激活时不 dump
function _p11k_maybe_dump_instant_prompt() {
  emulate -L zsh
  [[ $POWERLEVEL9K_INSTANT_PROMPT != off && $POWERLEVEL9K_DISABLE_INSTANT_PROMPT != true ]] || return
  (( ${+__p11k_instant_prompt_active} )) && return
  local sig=$PWD:${${SSH_CONNECTION:+1}:-0}:${(%):-%#}
  (( ${+_p11k_dumped_instant_prompt_sigs[$sig]} )) && return
  _p11k_set_instant_prompt || return
  _p11k_dump_instant_prompt || return
  _p11k_dumped_instant_prompt_sigs[$sig]=1
}

# transient prompt（对齐 p10k：POWERLEVEL9K_TRANSIENT_PROMPT=always/same-dir/off）。
# 命令执行后 prompt 缩为单行 prompt_char；same-dir 模式仅目录不变时触发。
function _p11k_zle_line_finish() {
  emulate -L zsh
  local mode=$(_p11k_p9k POWERLEVEL9K_TRANSIENT_PROMPT off)
  [[ $mode == (always|same-dir) ]] || return
  if [[ $mode == same-dir && $PWD != $__p11k_last_pwd ]]; then
    return
  fi
  local char=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_CONTENT_EXPANSION '❯')
  local fg=$(_p11k_p9k POWERLEVEL9K_PROMPT_CHAR_OK_VIINS_FOREGROUND green)
  # 保留 RPROMPT 不清（对齐 p10k：transient 时右提示保留）
  PROMPT="%F{$fg}$char%f "
  zle && zle .reset-prompt
}
add-zle-hook-widget line-finish _p11k_zle_line_finish 2>/dev/null

# vi 模式：keymap 切换时更新 prompt_char/vi_mode 并重绘
function _p11k_zle_keymap_select() {
  emulate -L zsh
  [[ $KEYMAP == vicmd ]] && __p11k_vi_mode=1 || __p11k_vi_mode=0
  _p11k_prompt
  zle && zle .reset-prompt
}
add-zle-hook-widget keymap-select _p11k_zle_keymap_select 2>/dev/null

# ────────────────────────── instant prompt 兜底加载 ──────────────────────────
# 通常用户在 .zshrc 顶部自行 source 缓存模板（效果最好：prompt 在 oh-my-zsh
# 加载前显示）。这里在主题加载时再尝试一次：模板内部有防重，已加载则跳过。
if [[ -t 0 && -t 1 && -o interactive && -o no_xtrace &&
      $POWERLEVEL9K_INSTANT_PROMPT != off && $POWERLEVEL9K_DISABLE_INSTANT_PROMPT != true &&
      ! ${+__p11k_instant_prompt_active} && ! ${+__p11k_instant_prompt_sourced} ]]; then
  local __p11k_instant_file=$__p11k_cache_dir/p11k-instant-prompt-${(%):-%n}.zsh
  [[ -r $__p11k_instant_file ]] && source $__p11k_instant_file 2>/dev/null
fi

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
