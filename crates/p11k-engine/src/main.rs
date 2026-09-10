//! p11k 引擎：pty 宿主 + 终端渲染层。
//!
//! 职责链：
//! 1. 用 portable-pty 起一个无主题 shell（ZDOTDIR 指向引擎生成的临时目录，
//!    .zshrc 里只设占位 PROMPT 和宣告钩子）。
//! 2. 透传：pty 输出 → 真实终端 stdout；真实终端 stdin → pty。引擎不解析
//!    pty 输出的 ANSI，shell 渲染什么（命令输出、zle、占位 prompt、补全
//!    菜单）引擎一概不管。
//! 3. prompt 窗口由 announce 驱动的两笔绘制：
//!    - `h`（precmd 宣告）：发 OSC133A 标记 + touch ack 放行 shell 输出占位，
//!      header 内容不在此时画。
//!    - `p`（zle-line-init 宣告，zle 已渲染完占位 prompt）：上移 header 行数
//!      回填真实 header，再回行首画真实前缀覆盖占位符。前缀只覆盖占位符
//!      所在列，即使引擎稍慢、用户已开始输入也不受影响。
//!
//! 几何协议：PROMPT 带 header 行数个换行 + "__" 占位符，让 shell 的 prompt
//! 几何把 header 行也算进去（zsh 才能用 reset-prompt 干净折叠 transient）。
//! precmd 写 announce 后轮询 ack，保证占位先输出、header 后回填。
//!
//! 部署：将 exec 启动加入用户 rc，并通过 P11K_ENGINE 避免递归
//! `[[ -z "$P11K_ENGINE" ]] && exec p11k --shell zsh`（见 README）。
//! - 正常（P11K_ENGINE 未设）：spawn 内部 shell 时设 `P11K_ENGINE=1`；
//! - 递归（P11K_ENGINE 已设）：用户 rc 的引导行无判断导致递归，启动空白
//!   shell 并打印修复提示，不加载用户 rc。
//!
//! 内部 shell 经 double-fork 孤儿化（脱离引擎进程树），引擎退出时 master
//! 关闭 → slave 挂断 → 内部 shell 收 SIGHUP 退出。header/前缀绘制时发 OSC
//! 133 A/B prompt markers，GUI 关窗确认据此判断光标停在 prompt、不再弹框。

mod config;
mod dir_shorten;
mod i18n;
mod presets;
mod render;
mod theme;
mod wizard;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// 真实终端 SIGWINCH（resize）标志：handler 只置位（signal-safe），主循环
/// 检查后同步内部 pty 尺寸 → 内部 zsh 收 SIGWINCH → TRAPWINCH 宣告 r →
/// 引擎清屏重画 prompt 窗口。
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_FLAG.store(true, Ordering::Relaxed);
}

use config::Config;
use i18n::t;
use p11k_gitstatus::{options::Options, protocol::field, repo::RepoCache};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use theme::{GitStatus, HeaderInfo};

/// 支持的 shell 类型。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
        }
    }
}

/// 从命令行参数解析 `--shell <name>`。
fn shell_from_args() -> Option<Shell> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--shell")?;
    match args.get(pos + 1).map(|s| s.as_str()) {
        Some("zsh") => Some(Shell::Zsh),
        Some("bash") => Some(Shell::Bash),
        Some("fish") => Some(Shell::Fish),
        _ => None,
    }
}

/// 从 `--config <path>` 读取 KDL 主题文件。
fn config_from_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--config")?;
    args.get(pos + 1).map(PathBuf::from)
}

/// 从 `--preset <name>` 取内置预设名(lean/classic/rainbow/pure)。
fn preset_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--preset")?;
    args.get(pos + 1).cloned()
}

/// 加载主题:--config 文件 > --preset 内置 > 缺省 lean;前两者失败打日志回退。
fn load_config() -> Config {
    let fallback = || Config::default_lean().expect("内置 lean 配置应合法");
    if let Some(path) = config_from_args() {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|src| Config::parse(&src))
        {
            Ok(c) => return c,
            Err(e) => eprintln!(
                "p11k: {}{path:?}: {e}",
                t("cannot read config, falling back to the built-in lean theme: ")
            ),
        }
    }
    if let Some(name) = preset_from_args() {
        if let Some(kind) = presets::by_name(&name) {
            return presets::build(kind);
        }
        eprintln!(
            "p11k: {}{name:?}",
            t(
                "unknown preset, falling back to the built-in lean theme (expected lean/classic/rainbow/pure): "
            )
        );
    }
    fallback()
}

/// 未指定 shell 时，回退到 $SHELL。
fn detect_shell() -> Shell {
    if let Some(s) = shell_from_args() {
        return s;
    }
    let she = std::env::var("SHELL").unwrap_or_default();
    match she.rsplit('/').next().unwrap_or("") {
        "bash" => Shell::Bash,
        "fish" => Shell::Fish,
        _ => Shell::Zsh,
    }
}

/// 生成给 shell 的 bootstrap .zshrc。
/// 时序：precmd 宣告 `h` → 轮询 ack → 返回后 zsh 渲染占位 PROMPT（zle）
/// → zle-line-init 宣告 `p` → 引擎回行首画前缀覆盖占位符。
/// 占位 prompt 由 zle 或其等价物渲染，主题均由引擎负责。
const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap —— 协议层 + 用户配置。
# ===== 引擎协议 =====
# 引擎未启用 transient 时该变量为空；清掉，避免外层环境残留的旧值让
# zle-line-finish 误折叠。
[[ -n "${P11K_TRANSIENT_PROMPT:-}" ]] || unset P11K_TRANSIENT_PROMPT
_p11k_status=0
_p11k_pwd=$PWD

_p11k_precmd() {
  _p11k_status=$?
  _p11k_pwd=$PWD
  # 用于恢复 transient 折叠后的 PROMPT
  PROMPT='__'
  print -r -- "h"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd"$'\t'"${#jobstates}"$'\t'"$HISTCMD" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}

# zle 渲染完占位 prompt 后宣告 `p`：引擎收到后回行首画真实前缀覆盖占位符。
# zle hook 只写文件，引擎异步处理（poll ~5ms）。
_p11k_line_init() {
  [[ ${CONTEXT:-start} == cont ]] && return
  if [[ -n "${_p11k_user_line_init:-}" ]] && (( $+functions[$_p11k_user_line_init] )); then
    "$_p11k_user_line_init"   # 用户注册的 handler（如 autosuggestions）先跑
  fi
  print -r -- "p" >> "$P11K_ANNOUNCE"
  # 上报当前 keymap:vi 模式(prompt 就绪时 zle 重置为 viins)报 viins → INSERT;
  # emacs(main)报 main → vi_mode 段隐藏。切换(NORMAL/VISUAL)由 keymap-select 上报。
  if [[ ${options[vi]} == on ]]; then
    print -r -- "v"$'\t'"viins" >> "$P11K_ANNOUNCE"
  else
    print -r -- "v"$'\t'"main" >> "$P11K_ANNOUNCE"
  fi
  # prompt 已就绪，清除防递归标记
  unset P11K_ENGINE
}

# ===== 用户配置 =====
# 优先 $P11K_USER_ZSHRC，否则默认 $HOME/.zshrc。
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi

# ===== 协议不变量 =====
# PROMPT 为占位。
PROMPT='__'
RPROMPT=''
PROMPT2='> '
precmd_functions=(${precmd_functions:#_p11k_precmd} _p11k_precmd)
# zle-line-init 单 handler，保留用户注册的，再注册引擎宣告。
if [[ -n "${widgets[zle-line-init]:-}" && "${widgets[zle-line-init]}" != user:_p11k_line_init ]]; then
  _p11k_user_line_init=${widgets[zle-line-init]#user:}
fi
zle -N zle-line-init _p11k_line_init
# vi_mode:zle-keymap-select 时把当前 keymap 上报给引擎。
_p11k_vi_mode() {
  local m=$KEYMAP
  if [[ ${options[vi]} == on ]]; then
    case $KEYMAP in
      vicmd|vis|viopp) m=$KEYMAP;;
      *) m=viins;;  # zsh 在 insert 时可能报 main → 归一成 viins
    esac
  else
    m=main
  fi
  print -r -- "v"$'\t'"$m" >> "$P11K_ANNOUNCE"
}
if [[ -n "${widgets[zle-keymap-select]:-}" && "${widgets[zle-keymap-select]}" != user:_p11k_vi_mode ]]; then
  _p11k_user_vi_mode=${widgets[zle-keymap-select]#user:}
fi
zle -N zle-keymap-select _p11k_vi_mode
# transient:命令提交时把多行 header 折叠成单行 ❯。
# 使用 zsh 自行渲染折叠后的 prompt。
_p11k_line_finish() {
  if [[ -n "${_p11k_user_line_finish:-}" ]] && (( $+functions[$_p11k_user_line_finish] )); then
    "$_p11k_user_line_finish"
  fi
  if [[ -n "${P11K_TRANSIENT_PROMPT:-}" ]]; then
    PROMPT="$P11K_TRANSIENT_PROMPT"
    RPROMPT=''
    zle reset-prompt
  fi
}
if [[ -n "${widgets[zle-line-finish]:-}" && "${widgets[zle-line-finish]}" != user:_p11k_line_finish ]]; then
  _p11k_user_line_finish=${widgets[zle-line-finish]#user:}
fi
zle -N zle-line-finish _p11k_line_finish
# resize 宣告：SIGWINCH → zsh 延迟执行 TRAPWINCH，
# 检测到变化宣告 `r`，引擎清屏重画 prompt 窗口。
_p11k_last_cols=$COLUMNS
_p11k_last_rows=$LINES
TRAPWINCH() {
  if (( COLUMNS != _p11k_last_cols || LINES != _p11k_last_rows )); then
    _p11k_last_cols=$COLUMNS
    _p11k_last_rows=$LINES
    print -r -- "r" >> "$P11K_ANNOUNCE"
  fi
}

# 补全：compinit 激活 + complist/menu select。
autoload -Uz compinit && compinit
zmodload -i zsh/complist
setopt auto_menu complete_in_word always_to_end
unsetopt menu_complete
zstyle ':completion:*:*:*:*:*' menu select
zstyle ':completion:*' matcher-list 'm:{[:lower:][:upper:]}={[:upper:][:lower:]}' 'r:|=*' 'l:|=* r:|=*'
zstyle ':completion:*' special-dirs true
zstyle ':completion:*' list-colors ''
zstyle ':completion:*:cd:*' tag-order local-directories directory-stack path-directories
"#;

/// bash 没有 `p` 宣告，引擎 ack 后 bash 直接打印 PS1（`__`），引擎靠字节匹配
/// 画前缀覆盖。resize 用 trap WINCH 检测尺寸变化宣告 `r`。
const BASHRC_TEMPLATE: &str = r#"# p11k engine bootstrap (bash) —— 协议层 + 用户配置。
PS1='__'

# PROMPT_COMMAND 在 PS1 显示前执行：记录退出码、宣告 `h` 后等
# 引擎 ack 才返回。
_p11k_prompt_command() {
  local _st=$?
  printf 'h\t%s\t%s\t%s\n' "$_st" "$PWD" "$(jobs -p | wc -l)" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}
PROMPT_COMMAND=_p11k_prompt_command

# resize：SIGWINCH 后 bash 更新 COLUMNS/LINES，trap 检测变化宣告 `r`。
_p11k_last_cols=$COLUMNS
_p11k_last_rows=$LINES
_p11k_winch() {
  if [[ $COLUMNS != $_p11k_last_cols || $LINES != $_p11k_last_rows ]]; then
    _p11k_last_cols=$COLUMNS
    _p11k_last_rows=$LINES
    printf 'r\n' >> "$P11K_ANNOUNCE"
  fi
}
trap '_p11k_winch' WINCH

# ===== 用户配置 =====
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi

# ===== 协议不变量 =====
PS1='__'
PROMPT_COMMAND=_p11k_prompt_command
trap '_p11k_winch' WINCH
"#;

/// fish 协议层（XDG_CONFIG_HOME 重定向注入）。fish_prompt 占位 + 宣告。
///
/// fish 没有 precmd/zle/PROMPT_COMMAND：宣告 h 放在 `fish_prompt`
/// 里，等 ack 后返回占位；引擎字节匹配占位画前缀覆盖（同
/// bash，无 `p` 宣告）。resize 用 `--on-signal WINCH`。
const FISH_TEMPLATE: &str = r#"# p11k engine bootstrap (fish) —— 协议层 + 用户配置。

set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES

# fish_prompt 在渲染主 prompt 时调用。两种情形：
# - 尺寸变化（resize 重绘）：宣告 `r`。
# - 正常 prompt：宣告 `h` 后等引擎 ack 才返回。
function fish_prompt
    set -l _st $status
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        printf '__'
        return
    end
    printf 'h\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    printf '__'
end

# ===== 用户配置 =====
if test -n "$P11K_USER_ZSHRC"; and test -r "$P11K_USER_ZSHRC"
    source "$P11K_USER_ZSHRC"
else if test -r "$HOME/.config/fish/config.fish"
    source "$HOME/.config/fish/config.fish"
end

# ===== 协议不变量 =====
set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES
function fish_prompt
    set -l _st $status
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        printf '__'
        return
    end
    printf 'h\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    printf '__'
end
"#;

fn main() -> anyhow::Result<()> {
    // 文案按 locale 取翻译(默认英文,中文见 po/zh_CN.po)。
    i18n::init();
    // `p11k configure`：进入交互配置向导，不 spawn shell。
    if std::env::args().any(|a| a == "configure") {
        return crate::wizard::run();
    }

    // 递归检测：P11K_ENGINE 已设 = 本引擎是被内部 shell 的 rc 引导再次调用的
    // 多余实例（用户 rc 里引导行忘了加判断，或写错）。降级为 exec 一个
    // 干净 shell，并提示用户修复。
    if std::env::var_os("P11K_ENGINE").is_some() {
        eprintln!(
            "p11k: {}\n\
             p11k: {}\n\
             p11k:   [[ -z \"$P11K_ENGINE\" ]] && exec p11k --shell zsh",
            t("recursive launch detected (p11k started inside a p11k session)"),
            t("make sure the bootstrap line in ~/.zshrc is guarded, e.g.:")
        );
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new("zsh").arg("-f").exec();
        eprintln!("p11k: {}{err}", t("cannot exec zsh -f: "));
        std::process::exit(1);
    }

    let shell = detect_shell();
    // 先加载配置以算输入行前缀宽度,再据此生成等宽占位符。
    let config = load_config();
    // prompt_char 各态(正常/ERROR)提示符必须等宽。
    // 不等宽是配置错误,直接在真实终端报错,再 exec 干净 shell
    if let Err(e) = crate::render::check_prompt_char_widths(&config) {
        eprintln!("p11k: {}{e}", t("config error: "));
        eprintln!(
            "p11k: {}",
            t(
                "every prompt_char state must be the same width (the prompt width is fixed at startup). Fix the config and start p11k again."
            )
        );
        use std::os::unix::process::CommandExt;
        let err = match shell {
            Shell::Zsh => std::process::Command::new("zsh").arg("-f").exec(),
            Shell::Bash => std::process::Command::new("bash")
                .args(["--noprofile", "--norc"])
                .exec(),
            Shell::Fish => std::process::Command::new("fish").arg("--no-config").exec(),
        };
        eprintln!("p11k: {}{err}", t("cannot exec a clean shell: "));
        std::process::exit(1);
    }
    let prefix = crate::render::input_prefix(&config, None);
    // 占位符带 header 行数个换行:shell 的 prompt 几何因此把 header 行也算进去,
    // zsh 才能用 reset-prompt 干净折叠 transient。
    // marker 是换行之后那段可见占位字节,bash/fish 靠匹配它定位占位已输出。
    let header_rows = config.layout.left.len().max(config.layout.right.len());
    let marker = "_".repeat(prefix.width.max(1));
    let placeholder = format!("{}{}", "\n".repeat(header_rows), marker);
    let state = StateDir::create(shell, &placeholder)?;
    log(&format!(
        "engine start: dir={} announce={} ack={} placeholder={:?} shell={:?}",
        state.dir.display(),
        state.announce.display(),
        state.ack.display(),
        placeholder,
        shell
    ));

    // 真实终端（stdin 所在的 pty）必须设为 raw 模式：关掉 ISIG/ICANON/ECHO，
    // 让所有字节原样透传给 pty 里的 shell。否则 ^C 会在真实终端一侧被内核
    // 转成 SIGINT 杀掉引擎（shell 根本收不到），输入还会被内核回显造成双回显。
    // shell 自己的终端（portable-pty 的 slave）由 shell 自己管理 termios。
    let _raw = RawTerminal::enter(libc::STDIN_FILENO)?;

    let (rows, cols, xpix, ypix) = tty_size().unwrap_or((24, 80, 0, 0));
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: xpix,
        pixel_height: ypix,
    })?;

    let mut cmd = CommandBuilder::new(shell.name());
    match shell {
        // zsh：ZDOTDIR 指向引擎目录，读其中的 .zshrc。
        Shell::Zsh => {
            cmd.env("ZDOTDIR", &state.dir);
        }
        // bash：--rcfile 指定协议层（交互 bash 才读 rcfile）。
        Shell::Bash => {
            cmd.arg("--rcfile");
            cmd.arg(&state.rc);
        }
        Shell::Fish => {
            // fish 通过 XDG_CONFIG_HOME 重定向，读 $XDG_CONFIG_HOME/fish/config.fish。
            cmd.env("XDG_CONFIG_HOME", &state.dir);
        }
    }
    cmd.env("P11K_ANNOUNCE", &state.announce);
    cmd.env("P11K_ACK", &state.ack);
    // portable-pty 的 spawn_command 默认把 current_dir 设成 HOME；这里显式
    // 用引擎启动时的 cwd，让内部 shell 落在用户当初 `exec p11k` 的目录
    cmd.cwd(std::env::current_dir()?);
    // 递归标志：内部 shell 及其子进程若再 exec p11k，入口检测到即降级，
    cmd.env("P11K_ENGINE", "1");
    // transient 是 zsh 独占能力，把引擎预计算的单行提示符(zsh %F 转义)交给 shell,
    // zle-line-finish 里换 PROMPT + reset-prompt 同步折叠。
    if config.layout.transient_prompt && shell == Shell::Zsh {
        cmd.env(
            "P11K_TRANSIENT_PROMPT",
            crate::render::transient_prompt_zsh(&config),
        );
    } else {
        // 关掉时必须显式清空：shell 端只看这个变量是否非空，若外层环境残留
        // 了旧值（例如从上一个 p11k 会话继承），配置说关也照样折叠。
        cmd.env("P11K_TRANSIENT_PROMPT", "");
    }

    // double-fork 孤儿化：内部 shell 脱离引擎进程树（父变 init）。kitty 关闭
    // 窗口时检测的是它 child 的子孙进程，孤儿不在树里 → 不弹"确认关闭"。
    // 引擎仍持 master fd 读写（fd 不因孤儿而断）；引擎退出时 master 关闭 →
    // slave 挂断 → 内部 shell 收 SIGHUP 退出，无需再记 PID 去 kill。
    let mid = unsafe { libc::fork() };
    if mid < 0 {
        anyhow::bail!("fork failed: {}", io::Error::last_os_error());
    }
    if mid == 0 {
        // 中间进程：spawn 内部 shell（父 = 本进程），随即退出使其孤儿化。
        let status = match pair.slave.spawn_command(cmd) {
            Ok(_) => 0,
            Err(_) => 1,
        };
        unsafe { libc::_exit(status) };
    }
    // 引擎：回收中间进程，丢弃 slave 引用（slave 已被内部 shell 接管）。
    let mut _st = 0;
    unsafe { libc::waitpid(mid, &mut _st, 0) };
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let master_fd = pair
        .master
        .as_raw_fd()
        .expect("pty master fd is available on unix");
    let mut writer = pair.master.take_writer()?;

    // 预创建 announce 文件，保证 shell 的 >> 追加不报错。
    File::create(&state.announce)?;

    // 捕获真实终端的 SIGWINCH：prompt 显示期间 resize 也能被引擎感知（同步
    // 内部 pty → TRAPWINCH 宣告 r → 清屏重画），不必等下一次 prompt。
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            handle_sigwinch as extern "C" fn(libc::c_int) as usize,
        );
    }

    let mut ann_processed: u64 = 0; // announce 文件已消费字节数
    let mut last_size = (rows, cols, xpix, ypix);
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    // 异步 git 状态。
    // worker 线程独占 RepoCache，主循环经 channel 发请求/收结果——大仓库
    // 首次扫描 1~2s 也不会卡住 prompt 显示。
    let (req_tx, req_rx) = mpsc::channel::<GitRequest>();
    let (res_tx, res_rx) = mpsc::channel::<GitResult>();
    // 按照 p10k 惯例进行调用;计数上限与 dirty 跳过阈值来自 vcs 段属性
    // (p10k 里是全局的 POWERLEVEL9K_VCS_*_MAX_NUM 与
    // POWERLEVEL9K_VCS_MAX_INDEX_SIZE_DIRTY,-1 = 不限)。先取值再 spawn,
    // 免得把整个 config 移动进线程。
    let dirty_cap = vcs_int_prop(&config, "max-index-size-dirty", -1);
    let max_staged = vcs_int_prop(&config, "max-num-staged", -1);
    let max_unstaged = vcs_int_prop(&config, "max-num-unstaged", -1);
    let max_conflicted = vcs_int_prop(&config, "max-num-conflicted", -1);
    let max_untracked = vcs_int_prop(&config, "max-num-untracked", -1);
    std::thread::spawn(move || {
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(32);
        let opts = Options {
            max_num_staged: max_staged,
            max_num_unstaged: max_unstaged,
            max_num_conflicted: max_conflicted,
            max_num_untracked: max_untracked,
            dirty_max_index_size: dirty_cap,
            num_threads,
            ..Default::default()
        };
        let mut cache = RepoCache::new(&opts);
        while let Ok(req) = req_rx.recv() {
            // 排空队列，只处理最新请求（丢弃积压的旧 cwd）。
            let mut latest = req;
            while let Ok(newer) = req_rx.try_recv() {
                latest = newer;
            }
            let status = git_status(&mut cache, &latest.cwd);
            if res_tx
                .send(GitResult {
                    generation: latest.generation,
                    status,
                })
                .is_err()
            {
                break; // 主循环退出，channel 关闭
            }
        }
    });

    let mut git_gen: u64 = 0; // 发起请求的序号，只使用最新结果
    // 当前 cwd 的上次 git 状态。
    let mut last_vcs: Option<(String, Option<GitStatus>)> = None;
    // 当前 prompt 的 info。
    let mut current_info: Option<HeaderInfo> = None;
    // 光标是否停在输入行（p 宣告后、用户回车前），异步结果仅在此时重画，
    // 否则 redraw 的 \e[1A 会画到命令输出上。
    let mut at_prompt = false;
    // 上次回车的时刻，用于下次 precmd 计算命令耗时。
    let mut last_enter: Option<std::time::Instant> = None;
    // resize 后延迟补画 prompt：等 zle 重绘 占位符+buffer 透传完（约 60ms）再画前缀，
    // 避免画 prompt 早于 zle 重绘被占位符覆盖、或光标停在 buffer 前。
    let mut resize_prompt_at: Option<std::time::Instant> = None;
    // bash 无 zle-line-init（无 `p` 宣告）：ack 后 bash 直接打印 PS1 占位符，
    // 引擎在透传流里匹配占位符画前缀覆盖。占位符前的内容累积到缓冲。
    let mut pending_placeholder = false;
    let mut placeholder_buf: Vec<u8> = Vec::new();

    // instant header：不等内部 shell 加载完较慢的用户 rc，立即用引擎 cwd 画
    // 占位 header + prompt，打开窗口即见 prompt；内部 shell 第一次 precmd 后
    // 换成真实状态。
    let instant_info = HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
        exec_seconds: 0.0,
        jobs: 0,
        history: 0,
    };
    // 返回的是每行的显示宽度：万一画的时候终端比 pty 窄（窗口刚建好、尺寸还没
    // 同步过来），内容会折行，擦除时得按**当时的列宽**折算实际占用行数，否则会
    // 留下半截 header。
    let mut instant_widths =
        theme::render_header_cfg(&mut stdout, cols as usize, &config, &instant_info, None)?;
    theme::render_prompt(&mut stdout, &prefix.text)?;
    stdout.flush()?;
    let mut instant_drawn = true;
    /// instant header 在给定列宽下实际占用的终端行数（内容宽于终端时终端折行）。
    fn instant_rows_at(widths: &[usize], cols: u16) -> usize {
        let c = cols.max(1) as usize;
        widths.iter().map(|w| w.div_ceil(c).max(1)).sum()
    }
    // instant 阶段内部 shell 透传给终端的内容：字节数 + 换行数。换行数 = 0
    // 表示屏幕上只有我们画的那份 header（可以就地擦掉重画）；否则说明 rc 真的
    // 打了行出来，那些行插在我们下方，只能保留。
    let mut instant_bytes = 0usize;
    let mut instant_newlines = 0usize;

    loop {
        // resize 信号：同步内部 pty 尺寸（含 pixel）。zsh/bash 靠各自的
        // TRAPWINCH/trap WINCH 宣告 r 后重画；fish 交互时不触发 signal event、
        // 重绘也不输出字节、不重调 fish_prompt，只能引擎主动重画。
        if RESIZE_FLAG.swap(false, Ordering::Relaxed) {
            // 重新套一遍 raw：tmux 之类的复用器在新建/调整 pane 时会重设 pane pty
            // 的 termios，ECHO 一旦被打开，引擎自己往终端写的东西会被回显回 stdin，
            // 再被当输入转发进 pty —— 屏幕上就会多出一份 header。
            let _ = RawTerminal::enter(libc::STDIN_FILENO);
            let (r, c, xp, yp) = tty_size().unwrap_or(last_size);
            if (r, c, xp, yp) != last_size {
                pair.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: xp,
                    pixel_height: yp,
                })?;
                last_size = (r, c, xp, yp);
                log(&format!("resized pty to {}x{} ({}x{} px)", r, c, xp, yp));
                // instant 期间还没有 shell 的 `r` 宣告来触发重画，屏幕上是按旧
                // 列宽画的那份（可能已经折行、位置全错），这里自己重画一遍。
                // 只有当 instant 阶段没别的输出时才敢清屏重来——终端 reflow 之后
                // "上移几行"已经不可靠，这是唯一稳妥的做法。
                if instant_drawn && instant_newlines == 0 {
                    write!(stdout, "\x1b[2J\x1b[H")?;
                    instant_widths = theme::render_header_cfg(
                        &mut stdout,
                        c as usize,
                        &config,
                        &instant_info,
                        None,
                    )?;
                    theme::render_prompt(&mut stdout, &prefix.text)?;
                    stdout.flush()?;
                }
            }
        }

        // prompt 窗口（announce 驱动）。在 poll/透传之前处理：fish 的
        // resize 里 `r` 宣告和占位符输出几乎同时，先 drain 让 pending 就位，
        // 再透传 pty 时字节匹配占位符（否则占位符先透传、pending 后设，漏匹配）。
        // - `h`（precmd 宣告，shell 在等 ack）：画 header，touch ack 放行。
        // - `p`（zle-line-init 宣告）：回行首画前缀覆盖占位符。
        // - `r`（resize 宣告）：重画 prompt 窗口。
        for msg in drain_announce(&state.announce, &mut ann_processed) {
            // 尺寸变了就同步给 pty，并让右对齐用新宽度。
            let (r, c, xp, yp) = tty_size().unwrap_or(last_size);
            if (r, c, xp, yp) != last_size {
                pair.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: xp,
                    pixel_height: yp,
                })?;
                last_size = (r, c, xp, yp);
            }
            match msg {
                AnnMsg::Header(mut info) => {
                    log(&format!(
                        "h: exit={:?} cwd={:?} jobs={}",
                        info.exit_code, info.cwd, info.jobs
                    ));
                    // 命令耗时 = 上次回车 → 本次 precmd（首 prompt 无 last_enter 则为 0）。
                    info.exec_seconds = last_enter
                        .take()
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    at_prompt = false; // 新 prompt 周期：header 回填前光标不在输入行
                    if instant_drawn {
                        // 第一次 precmd：把 instant header 换成真实状态。
                        //
                        // 平时（这段时间内部 shell 没输出）只需要**擦掉自己画的那
                        // 几行**：上移到 header 首行、`\x1b[J` 擦到屏末——光标下方
                        // 只有我们画的 header + 输入行。这样不动屏幕上方的既有内容
                        // （上一个会话的输出、窗口横幅、`exec p11k` 之前的打印），
                        // 也不像 `\x1b[2J` 那样整屏闪一下。p10k 同样不清屏。
                        //
                        // 若期间内部 shell 有输出（rc 的 echo/警告/报错），那些行就
                        // 插在我们下方，"上移 k 行"会落进输出里——这时什么都不做，
                        // 保留输出，让真 prompt 接在它后面（p10k 也保留并给警告）。
                        // 打开 P11K_INSTANT_LOG 可以打一行日志看是否命中。
                        instant_drawn = false;
                        // 现场重新问一次终端宽度：画 instant header 时如果尺寸还没
                        // 同步过来（窗口刚建好、SIGWINCH 还没到），内容是按旧宽度画
                        // 的、在真实窗口里已经折行，得按**真实**列宽折算行数。
                        let (rows_cap, cols_now, ..) = tty_size().unwrap_or(last_size);
                        let rows = instant_rows_at(&instant_widths, cols_now);
                        if instant_newlines == 0 && rows < rows_cap as usize {
                            write!(stdout, "\x1b[{rows}A\r\x1b[J")?;
                        } else if instant_newlines == 0 {
                            // 窗口比 header 还矮：header 自己就把屏滚了，位置不可靠，
                            // 退回整屏清。
                            write!(stdout, "\x1b[2J\x1b[H")?;
                        } else if std::env::var_os("P11K_INSTANT_LOG").is_some() {
                            log(&format!(
                                "instant header kept: {instant_newlines} line(s) / {instant_bytes} bytes of shell output during init"
                            ));
                        }
                    } else if config.layout.prompt_add_newline > 0 {
                        // 宽松布局：连续 prompt 之间留 N 个空行（header 前先空出来）。
                        for _ in 0..config.layout.prompt_add_newline {
                            write!(stdout, "\r\n\r\n")?;
                        }
                    }
                    // 先发 prompt 开始标记(早于 shell 的多行占位);header 内容不在这
                    // 画,否则会与占位自带换行重复推进光标。
                    theme::prompt_start(&mut stdout)?;
                    stdout.flush()?;
                    File::create(&state.ack)?; // 放行 precmd → shell 输出占位符
                    // bash/fish 无 `p` 宣告：ack 后 shell 打印占位 prompt，
                    if shell != Shell::Zsh {
                        pending_placeholder = true;
                        placeholder_buf.clear();
                    }
                    // 后台算 git，算完异步重画 header。
                    git_gen += 1;
                    let _ = req_tx.send(GitRequest {
                        generation: git_gen,
                        cwd: info.cwd.clone(),
                    });
                    current_info = Some(info);
                    log("drew header, acked");
                }
                AnnMsg::Prompt => {
                    log("p: fill header + draw prompt prefix");
                    // zle 已渲染完多行占位(header 行数空行 + 占位符),光标停在占位符后:
                    // 上移 header 行数回填真实 header,再回行首覆盖占位符为前缀。
                    let info = current_info.as_ref().unwrap_or(&instant_info);
                    let vcs = last_vcs
                        .as_ref()
                        .filter(|(cwd, _)| cwd == &info.cwd)
                        .and_then(|(_, s)| s.as_ref());
                    theme::redraw_header_cfg(
                        &mut stdout,
                        last_size.1 as usize,
                        &config,
                        info,
                        vcs,
                    )?;
                    // 前缀按当前退出码动态生成。
                    let text = crate::render::input_prefix(
                        &config,
                        current_info.as_ref().and_then(|i| i.exit_code),
                    )
                    .text;
                    theme::render_prompt(&mut stdout, &text)?;
                    stdout.flush()?;
                    at_prompt = true; // 输入行就绪
                }
                AnnMsg::Resize => {
                    // 尺寸检查在循环开头已 resize pty。先更新 header，
                    // 延迟 ~60ms 再补画 prompt（等 zle 重绘 占位prompt+buffer 透传完）。
                    if at_prompt {
                        log("r: redraw header, defer prompt");
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_header_cfg(
                            &mut stdout,
                            last_size.1 as usize,
                            &config,
                            current_info.as_ref().unwrap_or(&instant_info),
                            vcs,
                        )?;
                        stdout.flush()?;
                        resize_prompt_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_millis(60));
                    } else {
                        log("r: skip redraw (not at prompt)");
                    }
                }
                AnnMsg::VimMode(mode) => {
                    *crate::render::CURRENT_VI_MODE.lock().unwrap() = mode;
                    if at_prompt {
                        log("v: update mode, redraw header");
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_header_cfg(
                            &mut stdout,
                            last_size.1 as usize,
                            &config,
                            current_info.as_ref().unwrap_or(&instant_info),
                            vcs,
                        )?;
                        stdout.flush()?;
                    }
                }
            }
        }

        let mut fds = [
            libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // 5ms 超时：兼作 announce 文件的轮询节奏。
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 5) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }

        // 真实终端输入 → pty。
        // 注意：关闭时 poll 可能只报 POLLHUP 不带 POLLIN，此时 read 返回 0。
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 4096];
            match stdin.read(&mut buf) {
                Ok(0) => break, // 真实终端关闭
                Ok(n) => {
                    // 用户回车 → 光标离开输入行，异步 git 结果
                    // 不再重画 header（否则 \e[1A 会画错行）。
                    if buf[..n].iter().any(|&b| b == b'\r' || b == b'\n') {
                        at_prompt = false;
                        last_enter = Some(std::time::Instant::now()); // 命令开始计时
                    }
                    writer.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // pty 输出 → 真实终端：透传。
        // 主题由 announce 驱动的两笔绘制完成。
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell 退出
                Ok(n) => {
                    if pending_placeholder {
                        placeholder_buf.extend_from_slice(&buf[..n]);
                        if let Some(pos) = find_bytes(&placeholder_buf, marker.as_bytes()) {
                            let end = pos + marker.len();
                            // 先透传占位(多行空行 + 占位符),光标推进到占位符后。
                            stdout.write_all(&placeholder_buf[..end])?;
                            // 上移 header 行数回填真实 header,再回行首覆盖占位符。
                            let info = current_info.as_ref().unwrap_or(&instant_info);
                            let vcs = last_vcs
                                .as_ref()
                                .filter(|(cwd, _)| cwd == &info.cwd)
                                .and_then(|(_, s)| s.as_ref());
                            theme::redraw_header_cfg(
                                &mut stdout,
                                last_size.1 as usize,
                                &config,
                                info,
                                vcs,
                            )?;
                            let text = crate::render::input_prefix(
                                &config,
                                current_info.as_ref().and_then(|i| i.exit_code),
                            )
                            .text;
                            theme::render_prompt(&mut stdout, &text)?;
                            stdout.write_all(&placeholder_buf[end..])?;
                            placeholder_buf.clear();
                            pending_placeholder = false;
                            at_prompt = true; // bash/fish 的"prompt 就绪"（等价 zsh 的 p 宣告）
                        } else if placeholder_buf.len() > 8192 {
                            stdout.write_all(&placeholder_buf)?;
                            placeholder_buf.clear();
                            pending_placeholder = false;
                        }
                    } else {
                        // instant 阶段统计透传内容：有**换行**说明内部 shell 真在屏
                        // 上打了东西（rc 的 echo/警告/报错），此时不能擦自己的行
                        // （行位置已经被顶下去了）；只有控制序列（设标题之类，用户
                        // rc 里很常见）不算，照旧走干净擦行。p10k 会保留并给警告。
                        if instant_drawn {
                            instant_bytes += n;
                            instant_newlines += buf[..n].iter().filter(|b| **b == b'\n').count();
                        }
                        stdout.write_all(&buf[..n])?;
                    }
                    stdout.flush()?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // 异步 git 结果：只使用最新 generation；且只在光标停在输入行时重画 header
        while let Ok(res) = res_rx.try_recv() {
            if res.generation != git_gen {
                continue; // 过期结果
            }
            let cwd = current_info
                .as_ref()
                .map(|i| i.cwd.clone())
                .unwrap_or_default();
            last_vcs = Some((cwd.clone(), res.status.clone()));
            if at_prompt {
                if let Some(info) = &current_info {
                    let vcs = res.status.as_ref();
                    theme::redraw_header_cfg(
                        &mut stdout,
                        last_size.1 as usize,
                        &config,
                        info,
                        vcs,
                    )?;
                    stdout.flush()?;
                }
            }
            // 输入行未就绪:last_vcs 已更新,`p`/marker 回填时自然带上,无需补画。
        }

        // resize 后延迟补画 prompt。
        if let Some(deadline) = resize_prompt_at {
            if std::time::Instant::now() >= deadline {
                resize_prompt_at = None;
                let text = crate::render::input_prefix(
                    &config,
                    current_info.as_ref().and_then(|i| i.exit_code),
                )
                .text;
                theme::render_prompt(&mut stdout, &text)?;
                stdout.flush()?;
                log("deferred prompt drawn");
            }
        }
    }

    // 引擎退出：不显式 kill。reader/writer/master 随函数返回一起 drop，master
    // fd 关闭 → slave 挂断 → 内部 shell（若仍活着）收 SIGHUP 退出。孤儿化后
    // 即使引擎被 SIGKILL，内核也会关 master fd，内部 shell 同样收到 SIGHUP。
    Ok(())
}

/// 引擎的临时工作目录：rc 文件（按 shell）+ announce/ack 文件。
struct StateDir {
    dir: PathBuf,
    /// 协议层 rc 文件路径（zsh 用 ZDOTDIR 指向它；bash 用 --rcfile）。
    rc: PathBuf,
    announce: PathBuf,
    ack: PathBuf,
}

impl StateDir {
    fn create(shell: Shell, placeholder: &str) -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("p11k-{}", process::id()));
        fs::create_dir_all(&dir)?;

        // 每 shell 一份协议层 rc。保留接口用于读取临时 rc 调试。
        let rc = match shell {
            Shell::Zsh => {
                let rc = dir.join(".zshrc");
                let content = match std::env::var("P11K_ZSHRC") {
                    Ok(path) => {
                        fs::read_to_string(&path).unwrap_or_else(|_| ZSHRC_TEMPLATE.to_string())
                    }
                    Err(_) => ZSHRC_TEMPLATE.to_string(),
                };
                fs::write(&rc, content.replace("__", placeholder))?;
                rc
            }
            Shell::Bash => {
                let rc = dir.join(".bashrc");
                fs::write(&rc, BASHRC_TEMPLATE.replace("__", placeholder))?;
                rc
            }
            Shell::Fish => {
                // fish 通过 XDG_CONFIG_HOME 重定向，读 $XDG_CONFIG_HOME/fish/config.fish。
                let fish_dir = dir.join("fish");
                fs::create_dir_all(&fish_dir)?;
                let rc = fish_dir.join("config.fish");
                fs::write(&rc, FISH_TEMPLATE.replace("__", placeholder))?;
                rc
            }
        };
        Ok(Self {
            announce: dir.join("announce"),
            ack: dir.join("ack"),
            dir,
            rc,
        })
    }
}

/// announce 消息：prompt 窗口的绘制与 resize。
enum AnnMsg {
    /// `h\t<exit>\t<cwd>`：precmd 宣告，画 header。
    Header(HeaderInfo),
    /// `p`：zle-line-init 宣告，画输入行前缀覆盖。
    Prompt,
    /// `r`：TRAPWINCH 宣告，resize pty + 清屏重画。
    Resize,
    /// `v\t<keymap>`：zle-keymap-select 宣告，更新编辑模式并重画 header。
    VimMode(String),
}

/// 异步 git 请求。
struct GitRequest {
    generation: u64,
    cwd: String,
}

/// 异步 git 结果。
struct GitResult {
    generation: u64,
    status: Option<GitStatus>,
}

/// 读 announce 文件的新行并解析为 prompt 消息。
/// 行格式：`h\t<exit_code>\t<cwd>` 或 `p`。
fn drain_announce(path: &Path, processed: &mut u64) -> Vec<AnnMsg> {
    let mut out = Vec::new();
    let Ok(mut f) = OpenOptions::new().read(true).open(path) else {
        return out;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return out;
    };
    if len <= *processed {
        return out;
    }
    use std::io::Seek;
    if f.seek(io::SeekFrom::Start(*processed)).is_err() {
        return out;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return out;
    }
    *processed = len;

    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        let mut parts = line.split('\t');
        match parts.next() {
            Some("h") => {
                let code = parts.next().and_then(|s| s.trim().parse::<i32>().ok());
                let cwd = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                let jobs = parts
                    .next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let history = parts
                    .next()
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                out.push(AnnMsg::Header(HeaderInfo {
                    exit_code: code,
                    cwd,
                    exec_seconds: 0.0, // 由主循环用 last_enter 填入
                    jobs,
                    history,
                }));
            }
            Some("p") => out.push(AnnMsg::Prompt),
            Some("r") => out.push(AnnMsg::Resize),
            Some("v") => {
                let mode = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                out.push(AnnMsg::VimMode(mode));
            }
            _ => continue,
        }
    }
    out
}

/// 引擎诊断日志，写入固定文件避免污染透传流。
fn log(msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/p11k-engine.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// 在 `haystack` 里找子切片 `needle` 的首位置。
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// 用 p11k-gitstatus 计算当前目录的 git 状态（非 repo 返回 None）。
///
/// 同进程复用 `RepoCache`：repo 句柄按 gitdir 缓存，`build_fields` 复用
/// staged-diff 缓存（HEAD 不变时）与 libgit2 内部缓存，避免每次 prompt 全量
/// 重扫。
fn git_status(cache: &mut RepoCache, cwd: &str) -> Option<GitStatus> {
    let repo = cache.get_or_open(cwd.as_bytes(), false)?;
    let f = repo.build_fields(false);
    let index_size = parse_field(&f[field::INDEX_SIZE]);
    Some(GitStatus {
        branch: String::from_utf8_lossy(&f[field::LOCAL_BRANCH]).into_owned(),
        commit: String::from_utf8_lossy(&f[field::COMMIT]).into_owned(),
        staged: parse_field(&f[field::NUM_STAGED]),
        unstaged: parse_field(&f[field::NUM_UNSTAGED]),
        conflicted: parse_field(&f[field::NUM_CONFLICTED]),
        untracked: parse_field(&f[field::NUM_UNTRACKED]),
        ahead: parse_field(&f[field::COMMITS_AHEAD]),
        behind: parse_field(&f[field::COMMITS_BEHIND]),
        push_ahead: parse_field(&f[field::PUSH_COMMITS_AHEAD]),
        push_behind: parse_field(&f[field::PUSH_COMMITS_BEHIND]),
        stashes: parse_field(&f[field::STASHES]),
        action: String::from_utf8_lossy(&f[field::ACTION]).into_owned(),
        tag: String::from_utf8_lossy(&f[field::TAG]).into_owned(),
        remote_branch: String::from_utf8_lossy(&f[field::REMOTE_BRANCH]).into_owned(),
        commit_summary: String::from_utf8_lossy(&f[field::COMMIT_SUMMARY]).into_owned(),
        index_size,
        remote_url: String::from_utf8_lossy(&f[field::REMOTE_URL]).into_owned(),
    })
}

/// 取 `vcs` 段上的整数属性（p10k 的全局 `POWERLEVEL9K_VCS_*` 参数在 KDL 里
/// 落在 vcs 段上）；缺省或不合法时用 `default`。
fn vcs_int_prop(config: &Config, key: &str, default: i64) -> i64 {
    match config.segment("vcs").prop(key) {
        Some(crate::config::Prop::Int(n)) => *n,
        _ => default,
    }
}

/// 字段是 SafePrint 后的十进制字符串，解析为 usize；失败按 0 处理。
fn parse_field(b: &[u8]) -> usize {
    std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// 真实终端 raw 模式：进入时保存原始 termios，Drop 时恢复。
/// 与 tmux/screen 等终端复用程序的职责相同：字节全透传，信号与回显
/// 由 pty 内的 shell 在自己的终端上处理。
struct RawTerminal {
    fd: i32,
    orig: Option<libc::termios>,
}

impl RawTerminal {
    fn enter(fd: i32) -> io::Result<Self> {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 {
                return Ok(Self { fd, orig: None });
            }
            let orig = t;
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
            t.c_oflag &= !(libc::OPOST);
            t.c_cflag |= libc::CS8;
            if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                fd,
                orig: Some(orig),
            })
        }
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Some(orig) = self.orig {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &orig);
            }
        }
    }
}

/// 真实终端尺寸（stdout 的 TIOCGWINSZ），含 pixel 尺寸。
fn tty_size() -> Option<(u16, u16, u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_row, ws.ws_col, ws.ws_xpixel, ws.ws_ypixel))
    } else {
        None
    }
}
