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
typeset -g __p11k_gitstatus_fd=-1        # 响应 fd（daemon stdout 的 pipe 读端）
typeset -gi __p11k_gitstatus_req_fd=-1   # 请求 fd（FIFO 写端）
typeset -g __p11k_gitstatus_pgid=''      # daemon 进程组 id（清理用）
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

  local -i resp_fd req_fd
  # TMPDIR 未设置/不可写时回退 /tmp（对齐原版 gitstatus.plugin.zsh）
  local tmpdir=$TMPDIR
  [[ -n $tmpdir && -d $tmpdir && -w $tmpdir ]] || tmpdir=/tmp
  local file_prefix=$tmpdir/p11k.$EUID.$sysparams[pid].$EPOCHSECONDS.$((++__p11k_gitstatus_counter))
  __p11k_gitstatus_file_prefix=$file_prefix

  # daemon stderr 日志（对齐原版 GITSTATUS_DAEMON_LOG；默认 /dev/null）
  local daemon_log=${GITSTATUS_DAEMON_LOG:-/dev/null}
  [[ -z $daemon_log ]] && daemon_log=/dev/null

  # 透传 p10k 的 VCS 上限配置（默认 -1=无限，对齐 p10k 传给 daemon 的值）
  local -a daemon_args=(
    -s ${POWERLEVEL9K_VCS_STAGED_MAX_NUM:--1}
    -u ${POWERLEVEL9K_VCS_UNSTAGED_MAX_NUM:--1}
    -d ${POWERLEVEL9K_VCS_UNTRACKED_MAX_NUM:--1}
    -c ${POWERLEVEL9K_VCS_CONFLICTED_MAX_NUM:--1}
    -m ${POWERLEVEL9K_VCS_MAX_INDEX_SIZE_DIRTY:--1}
    -t ${GITSTATUS_NUM_THREADS:-32}
  )
  # -e：递归统计 untracked 目录内的文件（对齐 POWERLEVEL9K_VCS_RECURSE_UNTRACKED_DIRS）
  (( ${POWERLEVEL9K_VCS_RECURSE_UNTRACKED_DIRS:-1} )) && daemon_args+=(-e)

  # 进程替换子进程在交互 shell（monitor on）下是进程组长，kill -- -$pgid
  # 才能命中 daemon 进程组；显式确保（非交互下无法开启，静默忽略）。
  setopt monitor 2>/dev/null

  # daemon 进程体：运行在进程替换 <(...) 里，其 stdout 即 pipe 写端。
  # {pipe_fd}>&1 复制 stdout（pipe 写端）用于回写 pgid 与 daemon 输出；
  # stdin 换成 FIFO 读端，请求经 FIFO 传入。
  function _p11k_gitstatus_daemon() {
    local -i pipe_fd
    exec 0<&- {pipe_fd}>&1 1>>$daemon_log 2>&1 || return
    local pgid=$sysparams[pid]
    [[ $pgid == <1-> ]] || return
    builtin cd -q / || return
    # 忽略 SIGPIPE：父进程退出时写 pipe 不应杀掉本进程（对齐原版）
    trap '' PIPE
    command mkfifo -- $file_prefix.fifo || return
    print -rnu $pipe_fd -- ${(l:20:)pgid} || return
    exec <$file_prefix.fifo || return
    zf_rm -- $file_prefix.fifo || return
    HOME=$HOME $P11K_DAEMON -G $__p11k_gitstatus_version \
      "${(@)daemon_args}" >&$pipe_fd
  }

  # 进程替换：创建 pipe，父进程经 resp_fd 读 daemon 输出
  sysopen -r -o cloexec -u resp_fd <(_p11k_gitstatus_daemon) || return 1

  # 读 pgid（20 字节左对齐）
  local pgid=''
  while (( $#pgid < 20 )); do
    [[ -t $resp_fd ]]
    sysread -s $((20 - $#pgid)) -t 1 -i $resp_fd 'pgid[$#pgid+1]' || return 1
  done
  [[ $pgid == ' '#<1-> ]] || return 1
  # 存整数 pgid（对齐原版 gitstatus.plugin.zsh 的 `typeset -gi ... =pgid`）：
  # pgid 是 20 位左对齐（前导空格），`-i`/算术展开会按数字解析去掉空格；
  # 若存带空格的字符串，_p11k_gitstatus_stop 里 `<1->` 匹配失败，kill -- -$pgid
  # 永不执行，daemon 残留成孤儿（kitty 关闭窗口时提示 /bin/zsh 仍在运行）。
  __p11k_gitstatus_pgid=$(( pgid ))

  # 打开 FIFO 写端（请求通道；daemon 侧 exec <fifo 阻塞等待此处打开）
  sysopen -w -o cloexec -u req_fd -- $file_prefix.fifo || return 1
  __p11k_gitstatus_req_fd=$req_fd
  __p11k_gitstatus_fd=$resp_fd

  # 握手
  local resp=''
  print -rn -- $'}hello\x1f\x1e' >&$req_fd || return 1
  while true; do
    [[ -t $resp_fd ]]
    sysread -s 1 -t 1 -i $resp_fd 'resp[$#resp+1]' || return 1
    [[ $resp == *$'\x1e' ]] && break
  done
  [[ $resp == $'}hello\x1f0\x1e' ]] || return 1

  # 响应 fd 挂 zle -F（异步回调）
  zle -F $resp_fd _p11k_gitstatus_on_readable 2>/dev/null
  return 0
}

# 发送查询（precmd 调用）。diff 字段缺省（全量统计）。
# daemon 未启动或已崩溃时自动（重）启动。
function _p11k_gitstatus_query() {
  (( __p11k_gitstatus_fd > 0 )) || _p11k_gitstatus_start || return 1
  # 单飞行：上一查询未完成时跳过。注意必须先检查 pending 再更新 req_id——
  # 若先覆盖 req_id 再跳过，旧响应的 id 与 req_id 不匹配会永远无法清除
  # pending，后续查询全部被跳过，vcs 不再更新。
  if (( ${+__p11k_gitstatus_pending} )); then
    # 超时保护：响应丢失/daemon 卡住时强制重发（5 秒）
    local -F _p11k_now=$EPOCHREALTIME
    (( _p11k_now - __p11k_gitstatus_pending_since < 5 )) && return 0
    unset __p11k_gitstatus_pending
  fi
  __p11k_gitstatus_req_id=$EPOCHREALTIME
  __p11k_gitstatus_pending=1
  __p11k_gitstatus_pending_since=$EPOCHREALTIME
  # GIT_DIR 模式：dir 字段用 :GIT_DIR 前缀（from_dotgit，对齐原版）
  local qdir=$PWD
  if [[ -n $GIT_DIR ]]; then
    if [[ $GIT_DIR == /* ]]; then
      qdir=":$GIT_DIR"
    else
      qdir=":$PWD/$GIT_DIR"
    fi
  fi
  # 请求：`<id>\x1f<dir>\x1f<diff>\x1e`（\x1f/\x1e 须用 $'...' 拼，
  # 双引号内 \x1f 是字面文本）。diff 字段缺省（全量统计）。
  if ! print -rn -- "$__p11k_gitstatus_req_id"$'\x1f'"$qdir"$'\x1e' >&$__p11k_gitstatus_req_fd 2>/dev/null; then
    # daemon 已退出（EPIPE 等）：清理 fd 并重启，下个 precmd 恢复
    exec {__p11k_gitstatus_fd}>&- 2>/dev/null
    exec {__p11k_gitstatus_req_fd}>&- 2>/dev/null
    __p11k_gitstatus_fd=-1
    __p11k_gitstatus_req_fd=-1
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
  # (@ps:) 的 p 让 \x1f 按转义解析（缺 p 则 \x1f 是字面 4 字符，不切分）
  f=("${(@ps:\x1f:)msg}")
  ((${#f} < 2)) && return
  # id 匹配
  [[ $f[1] == $__p11k_gitstatus_req_id ]] || return
  unset __p11k_gitstatus_pending
  if [[ $f[2] == 0 ]]; then
    # 非仓库：清空 vcs（对齐 p10k 的 _p9k_vcs_status_purge）
    unset __p11k_vcs_ready
    unset __p11k_vcs_branch
    __p11k_vcs_dirty=0
  else
    # 仓库响应：id + flag + 27 字段 = 29 段
    ((${#f} < 29)) && return
    # 字段映射：f[1]=id, f[2]=flag(1/0), f[3]=field[0]（workdir），
    # 故 field[i] = f[3+i]。索引见 protocol.rs field 模块。
    #   LOCAL_BRANCH=2 → f[5];  NUM_UNSTAGED=9  → f[12]
    #   NUM_UNTRACKED=11 → f[14]; COMMITS_AHEAD=12 → f[15]
    #   COMMITS_BEHIND=13 → f[16]; STASHES=14 → f[17]; TAG=15 → f[18]
    __p11k_vcs_ready=1
    __p11k_vcs_branch=$f[5]
    __p11k_vcs_unstaged=$f[12]
    __p11k_vcs_untracked=$f[14]
    __p11k_vcs_ahead=$f[15]
    __p11k_vcs_behind=$f[16]
    __p11k_vcs_stashes=$f[17]
    __p11k_vcs_tag=$f[18]
    (( __p11k_vcs_dirty = __p11k_vcs_unstaged + __p11k_vcs_untracked > 0 ))
  fi
  # 每次响应都重绘（对齐 p10k _p9k_vcs_resume 末尾的无条件 _p9k_reset_prompt）。
  # 不要用"内容变了才重绘"的缓存：repo→非 repo 或 untracked 计数变化时
  # 缓存不失效，vcs 会卡在过期/缺失状态（cd 往返后 ?10 消失的根因）。
  _p11k_prompt
  _p11k_reset_prompt
}

# 重绘当前 prompt（对齐 p10k 的 _p9k_reset_prompt）
function _p11k_reset_prompt() {
  zle && zle .reset-prompt 2>/dev/null || true
}

# 会话退出时清理：关 fd + 杀 daemon 进程组（避免孤儿残留）
function _p11k_gitstatus_stop() {
  (( __p11k_gitstatus_fd > 0 )) || return
  zle -F $__p11k_gitstatus_fd 2>/dev/null
  exec {__p11k_gitstatus_fd}>&- 2>/dev/null
  exec {__p11k_gitstatus_req_fd}>&- 2>/dev/null
  if [[ $__p11k_gitstatus_pgid == <1-> ]]; then
    kill -- -$__p11k_gitstatus_pgid 2>/dev/null
  fi
  __p11k_gitstatus_fd=-1
  __p11k_gitstatus_req_fd=-1
}
add-zsh-hook zshexit _p11k_gitstatus_stop
