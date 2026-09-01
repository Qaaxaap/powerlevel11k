# zsh/gitstatus.zsh — p11k-d client speaking the gitstatusd wire protocol (v1.5.5).
#
# Requests:  <id>\x1f<dir>\x1f<diff>\x1e
# Responses: <id>\x1f<1|0> [\x1f<field>...] \x1e   (repo: flag 1 + 27 fields)
# pgid handshake: zsh writes the daemon's 20-byte left-aligned pgid before use.

typeset -g P11K_DAEMON=${P11K_DAEMON:-$__p11k_root_dir/target/release/p11k-d}
typeset -g __p11k_gitstatus_version='v1.5.5'

typeset -g __p11k_gitstatus_fd=-1        # response pipe read end
typeset -gi __p11k_gitstatus_req_fd=-1   # request FIFO write end
typeset -g __p11k_gitstatus_pgid=''      # daemon process group (cleanup)
typeset -g __p11k_gitstatus_file_prefix=''

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
  local tmpdir=$TMPDIR
  [[ -n $tmpdir && -d $tmpdir && -w $tmpdir ]] || tmpdir=/tmp
  local file_prefix=$tmpdir/p11k.$EUID.$sysparams[pid].$EPOCHSECONDS.$((++__p11k_gitstatus_counter))
  __p11k_gitstatus_file_prefix=$file_prefix

  local daemon_log=${GITSTATUS_DAEMON_LOG:-/dev/null}
  [[ -z $daemon_log ]] && daemon_log=/dev/null

  # VCS limits from the p10k config (default -1 = unlimited).
  local -a daemon_args=(
    -s ${POWERLEVEL9K_VCS_STAGED_MAX_NUM:--1}
    -u ${POWERLEVEL9K_VCS_UNSTAGED_MAX_NUM:--1}
    -d ${POWERLEVEL9K_VCS_UNTRACKED_MAX_NUM:--1}
    -c ${POWERLEVEL9K_VCS_CONFLICTED_MAX_NUM:--1}
    -m ${POWERLEVEL9K_VCS_MAX_INDEX_SIZE_DIRTY:--1}
    -t ${GITSTATUS_NUM_THREADS:-32}
  )
  # -p: exit when this zsh dies (the daemon polls kill(zsh_pid, 0)).
  daemon_args+=(-p $sysparams[pid])
  # -e: count files inside untracked directories.
  (( ${POWERLEVEL9K_VCS_RECURSE_UNTRACKED_DIRS:-1} )) && daemon_args+=(-e)

  # Interactive shells run process-substitution children in their own group,
  # so `kill -- -$pgid` can target the daemon.
  setopt monitor 2>/dev/null

  # Runs inside the process substitution; stdout (pipe_fd) is the pipe write
  # end. Orphan the daemon: `&` then `exec =true` detaches it from zsh's
  # process tree; it exits on FIFO EOF when zsh dies.
  function _p11k_gitstatus_daemon() {
    local -i pipe_fd
    exec 0<&- {pipe_fd}>&1 1>>$daemon_log 2>&1 || return
    local pgid=$sysparams[pid]
    [[ $pgid == <1-> ]] || return
    builtin cd -q / || return
    trap '' PIPE
    command mkfifo -- $file_prefix.fifo || return
    print -rnu $pipe_fd -- ${(l:20:)pgid} || return
    exec <$file_prefix.fifo || return
    zf_rm -- $file_prefix.fifo || return
    $P11K_DAEMON -G $__p11k_gitstatus_version \
      "${(@)daemon_args}" >&$pipe_fd &
    exec =true
  }

  sysopen -r -o cloexec -u resp_fd <(_p11k_gitstatus_daemon) || return 1

  # Read the 20-byte left-aligned pgid and store it numeric.
  local pgid=''
  while (( $#pgid < 20 )); do
    [[ -t $resp_fd ]]
    sysread -s $((20 - $#pgid)) -t 1 -i $resp_fd 'pgid[$#pgid+1]' || return 1
  done
  [[ $pgid == ' '#<1-> ]] || return 1
  __p11k_gitstatus_pgid=$(( pgid ))

  # FIFO write end; the daemon's `exec <fifo` blocks until this opens.
  sysopen -w -o cloexec -u req_fd -- $file_prefix.fifo || return 1
  __p11k_gitstatus_req_fd=$req_fd
  __p11k_gitstatus_fd=$resp_fd

  # Handshake: }hello.
  local resp=''
  print -rn -- $'}hello\x1f\x1e' >&$req_fd || return 1
  while true; do
    [[ -t $resp_fd ]]
    sysread -s 1 -t 1 -i $resp_fd 'resp[$#resp+1]' || return 1
    [[ $resp == *$'\x1e' ]] && break
  done
  [[ $resp == $'}hello\x1f0\x1e' ]] || return 1

  zle -F $resp_fd _p11k_gitstatus_on_readable 2>/dev/null
  return 0
}

# Query the current directory (called from precmd). Restarts the daemon if
# it died.
function _p11k_gitstatus_query() {
  (( __p11k_gitstatus_fd > 0 )) || _p11k_gitstatus_start || return 1
  # Single-flight: skip if a query is in flight. Check pending before
  # overwriting req_id, otherwise an old response can never match.
  if (( ${+__p11k_gitstatus_pending} )); then
    # Resend after 5s if the response was lost.
    local -F _p11k_now=$EPOCHREALTIME
    (( _p11k_now - __p11k_gitstatus_pending_since < 5 )) && return 0
    unset __p11k_gitstatus_pending
  fi
  __p11k_gitstatus_req_id=$EPOCHREALTIME
  __p11k_gitstatus_pending=1
  __p11k_gitstatus_pending_since=$EPOCHREALTIME
  # GIT_DIR mode: dir field is :GIT_DIR (from_dotgit).
  local qdir=$PWD
  if [[ -n $GIT_DIR ]]; then
    if [[ $GIT_DIR == /* ]]; then
      qdir=":$GIT_DIR"
    else
      qdir=":$PWD/$GIT_DIR"
    fi
  fi
  # \x1f/\x1e must be built with $'...' (double quotes keep them literal).
  if ! print -rn -- "$__p11k_gitstatus_req_id"$'\x1f'"$qdir"$'\x1e' >&$__p11k_gitstatus_req_fd 2>/dev/null; then
    # Daemon gone (EPIPE): close fds, restart on the next precmd.
    exec {__p11k_gitstatus_fd}>&- 2>/dev/null
    exec {__p11k_gitstatus_req_fd}>&- 2>/dev/null
    __p11k_gitstatus_fd=-1
    __p11k_gitstatus_req_fd=-1
    unset __p11k_gitstatus_pending
    _p11k_gitstatus_start
  fi
}

# zle -F callback: read any buffered responses, split on \x1e.
function _p11k_gitstatus_on_readable() {
  local fd=$1
  local -a resp
  local chunk=''
  while sysread -s 4096 -t 0 -i $fd 'chunk[$#chunk+1]' 2>/dev/null; do
    :
  done
  local msg
  while [[ $chunk == *$'\x1e'* ]]; do
    msg=${chunk%%$'\x1e'*}
    chunk=${chunk#*$'\x1e'}
    _p11k_gitstatus_process "$msg"
  done
}

# Handle one response. The p flag makes (@ps:) parse \x1f as a byte.
function _p11k_gitstatus_process() {
  emulate -L zsh
  local msg=$1
  local -a f
  f=("${(@ps:\x1f:)msg}")
  ((${#f} < 2)) && return
  [[ $f[1] == $__p11k_gitstatus_req_id ]] || return
  unset __p11k_gitstatus_pending
  if [[ $f[2] == 0 ]]; then
    unset __p11k_vcs_ready
    unset __p11k_vcs_branch
    __p11k_vcs_dirty=0
  else
    # Repo response: id + flag + 27 fields = 29 parts. field[i] = f[3+i].
    #   LOCAL_BRANCH=2 → f[5];  NUM_UNSTAGED=9  → f[12]
    #   NUM_UNTRACKED=11 → f[14]; COMMITS_AHEAD=12 → f[15]
    #   COMMITS_BEHIND=13 → f[16]; STASHES=14 → f[17]; TAG=15 → f[18]
    ((${#f} < 29)) && return
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
  # Redraw on every response (matches p10k's vcs resume).
  _p11k_prompt
  _p11k_reset_prompt
}

function _p11k_reset_prompt() {
  zle && zle .reset-prompt 2>/dev/null || true
}

# Session exit: close fds, kill the daemon group.
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
