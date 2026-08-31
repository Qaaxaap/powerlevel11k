# zsh/gitstatus.zsh —— p11k 的最小 gitstatus 客户端。
#
# 启动 p11k-d（Rust 后端），实现握手、查询、异步响应回调。
# 协议细节（与调研报告一致）：
# - 请求：`<id>\x1f<dir>\x1f<diff>\x1e`（ASCII 31/30 分隔）
# - 响应：`<id>\x1f<1|0>` + 27 字段
# - pgid 握手：zsh 侧在 exec daemon 前写 20 位左对齐 pgid
#
# 流程（对齐 gitstatus.plugin.zsh）：
# 1. file_prefix = $TMPDIR/p11k.$EUID.$pid.$EPOCHSECONDS.$n
# 2. FIFO 双工：zsh→FIFO→daemon stdin；daemon stdout→pipe→zsh
# 3. 握手请求 `}hello` 验证
# 4. 每次 precmd 发查询；zle -F 异步读响应并重绘

# p11k-d 二进制路径：优先环境变量，否则主题目录的 target/release
typeset -g P11K_DAEMON=${P11K_DAEMON:-$__p11k_root_dir/target/release/p11k-d}
# 协议版本（对齐 p10k master 的 build.info；见 options.rs PROTOCOL_VERSION）
typeset -g __p11k_gitstatus_version='v1.5.5'

# 会话状态
typeset -g __p11k_gitstatus_fd=-1
typeset -g __p11k_gitstatus_file_prefix=''

# 响应解析产物（供 _p11k_seg_vcs 使用）
typeset -g __p11k_vcs_ready=''
typeset -g __p11k_vcs_branch=''
typeset -g __p11k_vcs_tag=''
typeset -gi __p11k_vcs_dirty=0
typeset -gi __p11k_vcs_unstaged=0
typeset -gi __p11k_vcs_untracked=0
typeset -gi __p11k_vcs_ahead=0
typeset -gi __p11k_vcs_behind=0
typeset -gi __p11k_vcs_stashes=0

function _p11k_gitstatus_start() {
  emulate -L zsh -o no_aliases -o extended_glob
  (( __p11k_gitstatus_fd > 0 )) && return 0
  [[ -x $P11K_DAEMON ]] || {
    print -u2 "p11k: daemon not found: $P11K_DAEMON (build with 'cargo build --release')"
    return 1
  }
  zmodload zsh/system zsh/datetime 2>/dev/null

  local -i pipe_fd
  local file_prefix=$TMPDIR/p11k.$EUID.$sysparams[pid].$EPOCHSECONDS.$((++__p11k_gitstatus_counter))
  __p11k_gitstatus_file_prefix=$file_prefix

  # 进程替换内：写 pgid → daemon stdin 接 FIFO → daemon stdout 回管道
  {
    exec 0<&- {pipe_fd}>&1 1>>/dev/null 2>&1 || return
    local pgid=$sysparams[pid]
    [[ $pgid == <1-> ]] || return
    builtin cd -q / || return
    command mkfifo -- $file_prefix.fifo || return
    print -rnu $pipe_fd -- ${(l:20:)pgid} || return
    exec <$file_prefix.fifo || return
    zf_rm -- $file_prefix.fifo || return
    HOME=$HOME $P11K_DAEMON -G $__p11k_gitstatus_version \
      -s -1 -u -1 -d -1 -c -1 -m -1 -t 32 >&$pipe_fd
  } 2>/dev/null &!

  # 读 pgid（20 字节）与响应 fd
  local pgid=''
  while (( $#pgid < 20 )); do
    sysread -s $((20 - $#pgid)) -t 1 -i $pipe_fd 'pgid[$#pgid+1]' || return 1
  done
  [[ $pgid == ' '#<1-> ]] || return 1
  __p11k_gitstatus_fd=$pipe_fd

  # 握手
  local resp=''
  print -rn -- $'}hello\x1f\x1e' >&$pipe_fd || return 1
  while true; do
    sysread -s 1 -t 1 -i $pipe_fd 'resp[$#resp+1]' || return 1
    [[ $resp == *$'\x1e' ]] && break
  done
  [[ $resp == $'}hello\x1f0\x1e' ]] || return 1

  # 响应 fd 挂 zle -F（异步回调）
  zle -F $pipe_fd _p11k_gitstatus_on_readable 2>/dev/null
  return 0
}

# 发送查询（precmd 调用）。diff 字段缺省（全量统计）。
# daemon 未启动或已崩溃时自动（重）启动。
function _p11k_gitstatus_query() {
  (( __p11k_gitstatus_fd > 0 )) || _p11k_gitstatus_start || return 1
  __p11k_gitstatus_req_id=$EPOCHREALTIME
  # 单飞行：上一查询未完成时跳过（对齐 p10k 的异步语义）
  (( ${+__p11k_gitstatus_pending} )) && return 0
  __p11k_gitstatus_pending=1
  if ! print -rn -- "$__p11k_gitstatus_req_id\x1f$PWD\x1e" >&$__p11k_gitstatus_fd 2>/dev/null; then
    # daemon 已退出（EPIPE 等）：清理 fd 并重启，下个 precmd 恢复
    exec {__p11k_gitstatus_fd}>&- 2>/dev/null
    __p11k_gitstatus_fd=-1
    unset __p11k_gitstatus_pending
    _p11k_gitstatus_start
  fi
}

# zle -F 回调：读响应（可能多条），按 id 路由
function _p11k_gitstatus_on_readable() {
  local fd=$1
  local -a resp
  local chunk=''
  while sysread -s 4096 -t 0 -i $fd 'chunk[$#chunk+1]' 2>/dev/null; do
    :
  done
  # 按 MSG_SEP 切分
  local msg
  while [[ $chunk == *$'\x1e'* ]]; do
    msg=${chunk%%$'\x1e'*}
    chunk=${chunk#*$'\x1e'}
    _p11k_gitstatus_process "$msg"
  done
}

# 处理一条响应：`<id>\x1f<1|0>\x1f<27 字段...>`
function _p11k_gitstatus_process() {
  emulate -L zsh
  local msg=$1
  local -a f
  f=("${(@s:\x1f:)msg}")
  ((${#f} < 2)) && return
  # id 匹配
  [[ $f[1] == $__p11k_gitstatus_req_id ]] || return
  unset __p11k_gitstatus_pending
  if [[ $f[2] == 0 ]]; then
    # 非仓库：清空 vcs
    unset __p11k_vcs_ready
    unset __p11k_vcs_branch
    __p11k_vcs_dirty=0
    return
  fi
  ((${#f} < 30)) && return
  # 字段映射（协议索引 +2）：
  # 4 branch, 11 unstaged, 13 untracked, 14 ahead, 15 behind,
  # 16 stashes, 17 tag
  __p11k_vcs_ready=1
  __p11k_vcs_branch=$f[4]
  __p11k_vcs_unstaged=$f[11]
  __p11k_vcs_untracked=$f[13]
  __p11k_vcs_ahead=$f[14]
  __p11k_vcs_behind=$f[15]
  __p11k_vcs_stashes=$f[16]
  __p11k_vcs_tag=$f[17]
  (( __p11k_vcs_dirty = __p11k_vcs_unstaged + __p11k_vcs_untracked > 0 ))
  # 状态变化时重绘（对齐 p10k 的异步回填行为）
  if [[ $__p11k_vcs_last_display != "$__p11k_vcs_branch:$__p11k_vcs_dirty" ]]; then
    __p11k_vcs_last_display="$__p11k_vcs_branch:$__p11k_vcs_dirty"
    _p11k_prompt
    _p11k_reset_prompt
  fi
}

# 重绘当前 prompt（对齐 p10k 的 _p9k_reset_prompt）
function _p11k_reset_prompt() {
  zle && zle .reset-prompt 2>/dev/null || true
}

# 会话退出时清理
function _p11k_gitstatus_stop() {
  (( __p11k_gitstatus_fd > 0 )) || return
  zle -F $__p11k_gitstatus_fd 2>/dev/null
  exec {__p11k_gitstatus_fd}>&-
  __p11k_gitstatus_fd=-1
}
add-zsh-hook zshexit _p11k_gitstatus_stop
