//! p11k 引擎（B 架构）：pty 宿主 + 终端渲染层。
//!
//! 职责链：
//! 1. 用 portable-pty 起一个无主题 shell（ZDOTDIR 指向引擎生成的临时目录，
//!    .zshrc 里只设占位 PROMPT 和宣告钩子）。
//! 2. 透传：pty 输出 → 真实终端 stdout；真实终端 stdin → pty。引擎不解析
//!    pty 输出的 ANSI（不交给库、不写状态机），shell 渲染什么（命令输出、
//!    zle、占位 prompt、补全菜单）引擎一概不管，vim 等原样透传。
//! 3. prompt 窗口由 announce 驱动的两笔绘制（不碰 pty 输出流）：
//!    - `h`（precmd 宣告）：画多行 header（"上面的行"按原来顺序渲染），
//!      touch ack 放行 shell 输出占位 prompt。
//!    - `p`（zle-line-init 宣告，zle 已渲染完占位 prompt）：回行首画真实
//!      前缀顶掉占位符（输入行这一行延后绘制）。前缀只覆盖输入行前 2 列
//!      （占位符所在列），即使引擎稍慢、用户已开始输入也不受影响。
//!
//! 几何协议：PROMPT 是纯 ASCII 占位（`aa`，宽 2 = 前缀 `❯ ` 宽），无换行、
//! 无颜色、无 Unicode——zle 对 prompt 宽度的计算精确，这是补全重绘不错位
//! 的前提。precmd 写 announce 后轮询 ack（precmd 不是 zle hook，阻塞安全），
//! 保证 header 先画、占位 prompt 后输出。
//!
//! 部署（主题形态，exec 引导）：用户 rc 里放一行
//! `[[ -z "$P11K_ENGINE" ]] && exec p11k --shell zsh`（见 README）。引擎入口据此：
//! - 正常（P11K_ENGINE 未设）：spawn 内部 shell 时设 `P11K_ENGINE=1`；
//! - 递归（P11K_ENGINE 已设）：用户 rc 的引导行漏了判断 → 降级 exec 干净
//!   shell 并打印修复提示，不 panic、不加载用户 rc，内部 shell（父）照常。
//!
//! 内部 shell 经 double-fork 孤儿化（脱离引擎进程树），引擎退出时 master
//! 关闭 → slave 挂断 → 内部 shell 收 SIGHUP 退出。header/前缀绘制时发 OSC
//! 133 A/B prompt markers，kitty 关窗确认据此判断光标停在 prompt、不再弹框。
//!
//! 已知局限（M0 记录，后续处理）：
//! - resize / Ctrl-L 全屏重绘时 zle 重画占位 prompt，但没有 `p` 宣告跟随，
//!   header 会被冲掉、占位符残留（zle 不知道 header 存在）。
//! - 上一个命令未换行（echo -n）时光标在行中，header 会接在残留后。
//! - 用户 rc 里的主题（ZSH_THEME=p10k）未经处理会抢渲染，靠文档/安装器
//!   引导用户移除（现阶段测试用 P11K_USER_ZSHRC 过滤副本）。

mod theme;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 真实终端 SIGWINCH（resize）标志：handler 只置位（signal-safe），主循环
/// 检查后同步内部 pty 尺寸 → 内部 zsh 收 SIGWINCH → zle pre-redraw 宣告 r →
/// 引擎清屏重画 prompt 窗口。
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_FLAG.store(true, Ordering::Relaxed);
}

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use p11k_gitstatus::{options::Options, protocol::field, repo::RepoCache};
use theme::{GitStatus, HeaderInfo};

/// 支持的 shell 类型。识别靠特征环境变量（引擎 exec 继承的原始环境）：逐个
/// 检查该 shell 的特征变量，非空即命中。数据驱动，加 shell = 加一个枚举
/// 变体 + 一个特征变量名。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    fn all() -> &'static [Shell] {
        &[Shell::Zsh, Shell::Bash, Shell::Fish]
    }

    /// 该 shell 的特征环境变量名（exec 后仍保留在引擎环境里）。
    fn marker(self) -> &'static str {
        match self {
            Shell::Zsh => "ZSH_VERSION",
            Shell::Bash => "BASH_VERSION",
            Shell::Fish => "FISH_VERSION",
        }
    }

    #[allow(dead_code)] // 后续 spawn 对应 shell 时使用
    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
        }
    }
}

/// 从命令行参数解析 `--shell <name>`（引导行显式标明 shell，exec 后唯一
/// 可靠信号：版本变量不 export、父进程是终端、$SHELL 是登录 shell）。
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

/// 识别当前 shell：优先 `--shell` 参数；否则逐个检查特征环境变量（exec 后
/// 通常不可靠，仅作兜底）；全不中回退 `$SHELL` 的 basename（登录 shell）。
fn detect_shell() -> Shell {
    if let Some(s) = shell_from_args() {
        return s;
    }
    for &shell in Shell::all() {
        if std::env::var_os(shell.marker()).is_some() {
            return shell;
        }
    }
    let she = std::env::var("SHELL").unwrap_or_default();
    match she.rsplit('/').next().unwrap_or("") {
        "bash" => Shell::Bash,
        "fish" => Shell::Fish,
        _ => Shell::Zsh,
    }
}

/// 占位 prompt（shell 侧 PROMPT 就是这个字符串）：宽 2 列的纯 ASCII。
/// 必须与 `theme::PROMPT_PREFIX`（输入行前缀 `❯ `）的可见宽度严格一致，
/// 否则 zle 重绘输入行的列偏移对不齐。字符内容不重要，宽度是协议。
const PLACEHOLDER: &str = "aa";

/// 生成给 shell 的 bootstrap .zshrc。
///
/// 结构：引擎协议（函数定义）→ source 用户配置（P11K_USER_ZSHRC，可选）
/// → 重申协议不变量（用户配置可能设 PROMPT/precmd，必须压回去）。
/// 时序：precmd 宣告 `h` → 轮询 ack（引擎画完 header 才 touch）→ 返回后
/// zsh 渲染占位 PROMPT（zle）→ zle-line-init 宣告 `p` → 引擎回行首画前缀
/// 顶掉占位符。占位 prompt 由 zle 渲染（zle 几何自洽），主题两笔都由引擎画。
const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap —— 协议层 + 用户配置。
# ===== 引擎协议：宣告钩子函数定义 =====
_p11k_status=0
_p11k_pwd=$PWD

# precmd：记录退出码和目录（$? 只有这里还是命令的退出码），宣告 `h`（画
# header）后等引擎 ack 才返回——header 先画、占位 prompt 后输出，顺序保证。
# precmd 不是 zle hook，sleep 轮询安全。
_p11k_precmd() {
  _p11k_status=$?
  _p11k_pwd=$PWD
  print -r -- "h"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}

# zle 渲染完占位 prompt（几何已建立、光标在占位符后）后宣告 `p`：引擎收到
# 后回行首画真实前缀顶掉占位符。zle hook 里不能跑外部命令（sleep 会挂 zle），
# 只写文件，引擎异步处理（poll ~5ms）。引擎的前缀只覆盖输入行前 2 列
# （占位符所在列），即使引擎稍慢、用户已输入，也不会碰到输入内容。
_p11k_line_init() {
  if [[ -n "${_p11k_user_line_init:-}" ]] && (( $+functions[$_p11k_user_line_init] )); then
    "$_p11k_user_line_init"   # 用户注册的 handler（如 autosuggestions）先跑
  fi
  print -r -- "p" >> "$P11K_ANNOUNCE"
  # 正式 prompt 已就绪，防递归标记不再需要：清掉后用户手动 exec zsh 重载
  # 配置时，新 zsh 无 P11K_ENGINE，引导行会重新 exec p11k 进引擎（主题恢复）。
  # 若不清，P11K_ENGINE=1 会让引导行跳过、落到无主题的普通 zsh。
  unset P11K_ENGINE
}

# ===== 用户配置（可选）：先于协议不变量加载 =====
# 优先 $P11K_USER_ZSHRC（开发/测试用的过滤副本），否则默认 $HOME/.zshrc。
# 注意：真实 rc 里若加载主题（ZSH_THEME=p10k），主题 hook 会抢渲染——见
# README，迁移到 p11k 后应注释掉 ZSH_THEME。下面的"协议不变量"只压回占位
# PROMPT 并链式保留 precmd/zle hook，无法关掉主题自身的渲染逻辑。
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi

# ===== 协议不变量：source 后重申（用户配置可能设 PROMPT/precmd/hooks）=====
# PROMPT 是纯 ASCII 占位（宽 2 列，与引擎前缀 ❯ 同宽），zle 的几何
# （prompt 宽度、输入行列偏移）由此自洽；真实 prompt 由引擎绘制。
PROMPT='aa'
RPROMPT=''
precmd_functions=(${precmd_functions:#_p11k_precmd} _p11k_precmd)
# zle-line-init 单 handler：保留用户注册的（链式调用），再注册我们的宣告。
if [[ -n "${widgets[zle-line-init]:-}" && "${widgets[zle-line-init]}" != user:_p11k_line_init ]]; then
  _p11k_user_line_init=${widgets[zle-line-init]#user:}
fi
zle -N zle-line-init _p11k_line_init
# resize 宣告：SIGWINCH → zsh 延迟执行 TRAPWINCH（此时 COLUMNS/LINES 已更新），
# 检测到变化宣告 `r`，引擎清屏重画 prompt 窗口（zle 的 resize 重绘会冲掉
# header；引擎 poll ~5ms 后处理，晚于 zle 重绘完成，重画不被冲掉）。
# 注意：zle-line-pre-redraw 在 SIGWINCH 重绘时不触发，不能用它。
_p11k_last_cols=$COLUMNS
_p11k_last_rows=$LINES
TRAPWINCH() {
  if (( COLUMNS != _p11k_last_cols || LINES != _p11k_last_rows )); then
    _p11k_last_cols=$COLUMNS
    _p11k_last_rows=$LINES
    print -r -- "r" >> "$P11K_ANNOUNCE"
  fi
}

# 补全：compinit 激活 + complist/menu select（对齐 oh-my-zsh 的 completion.zsh，
# 菜单行为正常）。占位符宽度 = 前缀宽度，菜单/重绘列偏移由 zle 精确计算。
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

/// bash 协议层（`--rcfile` 注入）。占位 PS1 + PROMPT_COMMAND 宣告。
///
/// bash 没有 zle（没有 zle-line-init），所以没有 `p` 宣告：引擎 ack 后 bash
/// 直接打印 PS1（`aa`），引擎靠字节匹配 `aa` 画前缀顶掉（见主循环）。resize
/// 用 trap WINCH 检测尺寸变化宣告 `r`。
const BASHRC_TEMPLATE: &str = r#"# p11k engine bootstrap (bash) —— 协议层 + 用户配置。
PS1='aa'

# PROMPT_COMMAND 在 PS1 显示前执行：记录退出码、宣告 `h`（画 header）后等
# 引擎 ack 才返回。bash 里 sleep 延迟 prompt 显示，正是 ack 机制需要的。
_p11k_prompt_command() {
  local _st=$?
  printf 'h\t%s\t%s\n' "$_st" "$PWD" >> "$P11K_ANNOUNCE"
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

# ===== 用户配置（可选）：先于协议不变量加载 =====
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi

# ===== 协议不变量：source 后重申（用户配置可能改 PS1/PROMPT_COMMAND）=====
PS1='aa'
PROMPT_COMMAND=_p11k_prompt_command
trap '_p11k_winch' WINCH
"#;

/// fish 协议层（XDG_CONFIG_HOME 重定向注入）。fish_prompt 占位 + 宣告。
///
/// fish 没有 precmd/zle/PROMPT_COMMAND：宣告 h 放在 `fish_prompt`（渲染 prompt
/// 时调用）里，等 ack 后返回占位 `aa`；引擎字节匹配 `aa` 画前缀顶掉（同
/// bash，无 `p` 宣告）。resize 用 `--on-signal WINCH`。
const FISH_TEMPLATE: &str = r#"# p11k engine bootstrap (fish) —— 协议层 + 用户配置。

# fish_prompt 在渲染主 prompt 时调用：保存退出码、宣告 `h`（画 header）后等
# 引擎 ack 才返回。sleep 延迟 prompt 显示，正是 ack 机制需要的。
function fish_prompt
    set -l _st $status
    printf 'h\t%s\t%s\n' $_st $PWD >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    # 占位 prompt（不带换行）：引擎在透传流里匹配 aa 画前缀顶掉。
    printf 'aa'
end

# resize：SIGWINCH 后 fish 更新 COLUMNS/LINES，声明 r。
set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES
function _p11k_winch --on-signal WINCH
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
    end
end

# ===== 用户配置（可选）：先于协议不变量加载 =====
if test -n "$P11K_USER_ZSHRC"; and test -r "$P11K_USER_ZSHRC"
    source "$P11K_USER_ZSHRC"
else if test -r "$HOME/.config/fish/config.fish"
    source "$HOME/.config/fish/config.fish"
end

# ===== 协议不变量：source 后重申（用户配置可能重定义 fish_prompt）=====
function fish_prompt
    set -l _st $status
    printf 'h\t%s\t%s\n' $_st $PWD >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    printf 'aa'
end
"#;

fn main() -> anyhow::Result<()> {
    // 递归检测：P11K_ENGINE 已设 = 本引擎是被内部 shell 的 rc 引导再次调用的
    // 多余实例（用户 rc 里引导行忘了加判断，或写错）。不 panic、不加载用户
    // rc，降级为 exec 一个干净 shell（不读 rc，不触发引导），并提示用户修。
    // 内部 shell 是父进程，本实例 exec 干净 shell 后它照常继续。
    if std::env::var_os("P11K_ENGINE").is_some() {
        eprintln!(
            "p11k: 检测到递归加载（已在 p11k 会话内又启动了 p11k）。\n\
             p11k: 请确认 ~/.zshrc 里的引导行带判断，例如：\n\
             p11k:   [[ -z \"$P11K_ENGINE\" ]] && exec p11k --shell zsh"
        );
        use std::os::unix::process::CommandExt;
        // exec 替换本进程为干净 shell（-f = 不读任何 rc，避免再次触发引导）。
        let err = std::process::Command::new("zsh").arg("-f").exec();
        eprintln!("p11k: exec zsh -f 失败: {err}");
        std::process::exit(1);
    }

    let shell = detect_shell();
    let state = StateDir::create(shell)?;
    log(&format!(
        "engine start: dir={} announce={} ack={} placeholder={:?} shell={:?}",
        state.dir.display(),
        state.announce.display(),
        state.ack.display(),
        PLACEHOLDER,
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
    // （git 状态等按此目录计算）。
    cmd.cwd(std::env::current_dir()?);
    // 递归标志：内部 shell 及其子进程若再 exec p11k，入口检测到即降级，
    // 不会无限套娃（引导行 `[[ -z $P11K_ENGINE ]] && exec p11k` 也因此跳过）。
    cmd.env("P11K_ENGINE", "1");

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
    // 内部 pty → zle pre-redraw 宣告 r → 清屏重画），不必等下一次 prompt。
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

    // 异步 git 状态（对齐 p10k：从不阻塞等 git，先画缓存的旧状态，算完刷新）。
    // worker 线程独占 RepoCache，主循环经 channel 发请求/收结果——大仓库
    // （nixpkgs）首次扫描 1~2s 也不会卡住 prompt 显示。
    let (req_tx, req_rx) = mpsc::channel::<GitRequest>();
    let (res_tx, res_rx) = mpsc::channel::<GitResult>();
    std::thread::spawn(move || {
        // 对齐 p10k 的 gitstatus 调用（-s -1 -u -1 -d -1 -c -1）：计数无上限，
        // 由 prompt 层自行决定截断。默认的 cap=1 会把 nixpkgs 的 10 个
        // untracked 显示成 ?1。
        let mut opts = Options::default();
        opts.max_num_staged = -1;
        opts.max_num_unstaged = -1;
        opts.max_num_conflicted = -1;
        opts.max_num_untracked = -1;
        // 扫描并行度：p10k 用 2*cpu（cap 32）。
        opts.num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(32);
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

    let mut git_gen: u64 = 0; // 发起请求的序号，只认最新结果
    // 当前 cwd 的上次 git 状态（跨 prompt 复用：先画旧的，异步刷新）。
    let mut last_vcs: Option<(String, Option<GitStatus>)> = None;
    // 当前 prompt 的 info（异步结果回来时用它重画 header 行 2）。
    let mut current_info: Option<HeaderInfo> = None;
    // 光标是否停在输入行（p 宣告后、用户回车前）——异步结果只在此时重画，
    // 否则 redraw 的 \e[1A 会画到命令输出上。
    let mut at_prompt = false;
    // bash 无 zle-line-init（无 `p` 宣告）：ack 后 bash 直接打印 PS1 `aa`，
    // 引擎在透传流里匹配 `aa` 画前缀顶掉。占位符前的内容累积到缓冲。
    let mut pending_placeholder = false;
    let mut placeholder_buf: Vec<u8> = Vec::new();
    // fish resize：引擎 resize pty 后等 fish 重绘输出 `aa`，匹配后重画整个
    // prompt 窗口（header 被 fish 重绘冲掉/旧宽度残留），而不是只画 ❯。
    let mut pending_redraw = false;

    // instant header（对齐 p10k instant prompt）：不等内部 shell 加载完
    // oh-my-zsh，立即用引擎 cwd 画占位 header + ❯，开窗即见 prompt；内部
    // shell 第一次 precmd 后再清屏刷新成真正状态（exit/git）。
    let instant_info = HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
    };
    theme::render_header(&mut stdout, cols as usize, &instant_info, None)?;
    theme::render_prompt(&mut stdout)?;
    stdout.flush()?;
    let mut instant_drawn = true;

    loop {
        // resize 信号：同步内部 pty 尺寸（含 pixel）。zsh/bash 靠各自的
        // TRAPWINCH/trap WINCH 宣告 r 后重画；fish 交互时不触发 signal event、
        // 重绘也不输出字节、不重调 fish_prompt，只能引擎主动重画。
        if RESIZE_FLAG.swap(false, Ordering::Relaxed) {
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
                if shell == Shell::Fish {
                    // fish 不立即 redraw：resize pty 后 fish 会重绘输出 `aa`，
                    // 引擎匹配它后再重画（避免早于 fish 重绘被覆盖）。
                    pending_placeholder = true;
                    placeholder_buf.clear();
                    pending_redraw = true;
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
                    // 用户回车（提交命令）→ 光标离开输入行，异步 git 结果
                    // 不再重画 header（否则 \e[1A 会画错行）。
                    if buf[..n].iter().any(|&b| b == b'\r' || b == b'\n') {
                        at_prompt = false;
                    }
                    writer.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // pty 输出 → 真实终端：纯透传。zsh 渲染什么（命令输出、zle、占位
        // prompt、补全菜单）引擎一概不管，真实终端照画；主题由 announce
        // 驱动的两笔绘制完成（h → header 行；p → 输入行前缀顶掉占位符）。
        // bash 无 `p` 宣告：pending_placeholder 时在流里匹配占位符 `aa` 画前缀。
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell 退出
                Ok(n) => {
                    if pending_placeholder {
                        placeholder_buf.extend_from_slice(&buf[..n]);
                        if let Some(pos) = find_bytes(&placeholder_buf, PLACEHOLDER.as_bytes()) {
                            let end = pos + PLACEHOLDER.len();
                            // 占位符（及之前的序列）先透传，再画前缀顶掉。
                            stdout.write_all(&placeholder_buf[..end])?;
                            if pending_redraw {
                                // fish resize：fish 重绘已输出 aa（旧 header 冲掉
                                // 或旧宽度残留），重画整个 prompt 窗口。
                                pending_redraw = false;
                                let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                                theme::redraw_full(
                                    &mut stdout,
                                    last_size.1 as usize,
                                    current_info.as_ref(),
                                    vcs,
                                )?;
                                log("fish resize: matched aa, redrawn full");
                            } else {
                                theme::render_prompt(&mut stdout)?;
                            }
                            stdout.write_all(&placeholder_buf[end..])?;
                            placeholder_buf.clear();
                            pending_placeholder = false;
                            at_prompt = true; // bash/fish 的"prompt 就绪"（等价 zsh 的 p 宣告）
                        } else if placeholder_buf.len() > 8192 {
                            // 防御：占位符迟迟不出现（异常配置），别憋着输出。
                            stdout.write_all(&placeholder_buf)?;
                            placeholder_buf.clear();
                            pending_placeholder = false;
                        }
                    } else {
                        stdout.write_all(&buf[..n])?;
                    }
                    stdout.flush()?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // prompt 窗口（announce 驱动）：
        // - `h`（precmd 宣告，zsh 在等 ack）：先画 header 行（占位 prompt 尚
        //   未输出，"上面的行"按原来顺序渲染），touch ack 放行。
        // - `p`（zle-line-init 宣告，占位 prompt 已显示、光标在占位符后）：
        //   回行首画真实前缀顶掉占位符——输入行这一行延后绘制，只覆盖前
        //   2 列，用户输入从列 2 开始，永不触碰。
        for msg in drain_announce(&state.announce, &mut ann_processed) {
            // 尺寸变了就同步给 pty（shell 重排），并让右对齐用新宽度。
            // pixel 一并透传，否则 resize 后内部 pty 的 ws_xpixel 又归零
            // （icat 等读 TIOCGWINSZ 的命令会失效）。
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
                AnnMsg::Header(info) => {
                    log(&format!(
                        "h: exit={:?} cwd={:?}",
                        info.exit_code, info.cwd
                    ));
                    at_prompt = false; // 新 prompt 周期：画 header 前光标不在输入行
                    // 立即用缓存的旧状态画 header（不阻塞）；cwd 匹配才有，否则空。
                    let vcs = last_vcs
                        .as_ref()
                        .filter(|(cwd, _)| cwd == &info.cwd)
                        .and_then(|(_, s)| s.as_ref());
                    if instant_drawn {
                        // 第一次 precmd：清屏把 instant header 刷新成真正状态
                        // （启动不久，屏幕上没有要保留的历史，清屏无害）。
                        instant_drawn = false;
                        theme::render_header_cleared(&mut stdout, c as usize, &info, vcs)?;
                    } else {
                        theme::render_header(&mut stdout, c as usize, &info, vcs)?;
                    }
                    stdout.flush()?;
                    File::create(&state.ack)?; // 放行 precmd → zsh 输出占位符
                    // bash/fish 无 `p` 宣告：ack 后 shell 打印占位 prompt（aa），
                    // 引擎在透传流里匹配它画前缀（zsh 靠 zle-line-init 的 `p`）。
                    if shell != Shell::Zsh {
                        pending_placeholder = true;
                        placeholder_buf.clear();
                    }
                    // 后台算 git，算完异步重画 header 行 2。
                    git_gen += 1;
                    let _ = req_tx.send(GitRequest {
                        generation: git_gen,
                        cwd: info.cwd.clone(),
                    });
                    current_info = Some(info);
                    log("drew header, acked");
                }
                AnnMsg::Prompt => {
                    log("p: draw prompt prefix");
                    theme::render_prompt(&mut stdout)?;
                    stdout.flush()?;
                    at_prompt = true; // 输入行就绪
                }
                AnnMsg::Resize => {
                    // zle 已清输入行并重画占位 prompt（尺寸检查在循环开头已
                    // resize pty）。只在光标停在输入行时上移重画 header+前缀
                    // （header 在输入行上方 2 行）；命令执行中 resize 不重画，
                    // 等下一次 prompt 自然画，避免上移踩到命令输出。
                    if at_prompt {
                        log("r: redraw prompt window");
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_full(
                            &mut stdout,
                            last_size.1 as usize,
                            current_info.as_ref(),
                            vcs,
                        )?;
                        stdout.flush()?;
                    } else {
                        log("r: skip redraw (not at prompt)");
                    }
                }
            }
        }

        // 异步 git 结果：只认最新 generation；且只在光标停在输入行时重画
        // header 行 2（否则 redraw 的上移会踩到命令输出）。
        while let Ok(res) = res_rx.try_recv() {
            if res.generation != git_gen {
                continue; // 过期结果（期间又出了新 prompt）
            }
            let cwd = current_info
                .as_ref()
                .map(|i| i.cwd.clone())
                .unwrap_or_default();
            last_vcs = Some((cwd, res.status.clone()));
            if at_prompt {
                if let Some(info) = &current_info {
                    let vcs = res.status.as_ref();
                    theme::redraw_vcs(&mut stdout, last_size.1 as usize, info, vcs)?;
                    stdout.flush()?;
                }
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
    fn create(shell: Shell) -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("p11k-{}", process::id()));
        fs::create_dir_all(&dir)?;

        // 每 shell 一份协议层 rc。P11K_ZSHRC 可指向外部 .zshrc（zsh 专用，
        // 比如手动改过的用户配置副本），引擎原样使用。
        let rc = match shell {
            Shell::Zsh => {
                let rc = dir.join(".zshrc");
                let content = match std::env::var("P11K_ZSHRC") {
                    Ok(path) => fs::read_to_string(&path)
                        .unwrap_or_else(|_| ZSHRC_TEMPLATE.to_string()),
                    Err(_) => ZSHRC_TEMPLATE.to_string(),
                };
                fs::write(&rc, content)?;
                rc
            }
            Shell::Bash => {
                let rc = dir.join(".bashrc");
                fs::write(&rc, BASHRC_TEMPLATE)?;
                rc
            }
            Shell::Fish => {
                // fish 通过 XDG_CONFIG_HOME 重定向，读 $XDG_CONFIG_HOME/fish/config.fish。
                let fish_dir = dir.join("fish");
                fs::create_dir_all(&fish_dir)?;
                let rc = fish_dir.join("config.fish");
                fs::write(&rc, FISH_TEMPLATE)?;
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
    /// `h\t<exit>\t<cwd>`：precmd 宣告（zsh 在等 ack），画 header 行。
    Header(HeaderInfo),
    /// `p`：zle-line-init 宣告（占位 prompt 已显示），画输入行前缀顶掉。
    Prompt,
    /// `r`：zle-line-pre-redraw 宣告（尺寸变化），resize pty + 清屏重画。
    Resize,
}

/// 异步 git 请求（主循环 → worker 线程）。
struct GitRequest {
    generation: u64,
    cwd: String,
}

/// 异步 git 结果（worker 线程 → 主循环）。
struct GitResult {
    generation: u64,
    status: Option<GitStatus>,
}

/// 读 announce 文件的新行并解析为 prompt 消息。
/// 行格式：`h\t<exit_code>\t<cwd>`（precmd）或 `p`（zle-line-init）。
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
                out.push(AnnMsg::Header(HeaderInfo {
                    exit_code: code,
                    cwd,
                }));
            }
            Some("p") => out.push(AnnMsg::Prompt),
            Some("r") => out.push(AnnMsg::Resize),
            _ => continue,
        }
    }
    out
}

/// 引擎诊断日志（M0 调试用，写入固定文件避免污染透传流）。
fn log(msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/p11k-engine.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// 在 `haystack` 里找子切片 `needle` 的首位置（字节匹配）。
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
/// 重扫。M0 同步调用（大仓库首次扫描会拖慢 prompt，后续异步化）。
fn git_status(cache: &mut RepoCache, cwd: &str) -> Option<GitStatus> {
    let repo = cache.get_or_open(cwd.as_bytes(), false)?;
    let f = repo.build_fields(false);
    Some(GitStatus {
        branch: String::from_utf8_lossy(&f[field::LOCAL_BRANCH]).into_owned(),
        staged: parse_field(&f[field::NUM_STAGED]),
        unstaged: parse_field(&f[field::NUM_UNSTAGED]),
        conflicted: parse_field(&f[field::NUM_CONFLICTED]),
        untracked: parse_field(&f[field::NUM_UNTRACKED]),
        ahead: parse_field(&f[field::COMMITS_AHEAD]),
        behind: parse_field(&f[field::COMMITS_BEHIND]),
        stashes: parse_field(&f[field::STASHES]),
    })
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
                // 不是 tty（如管道）也能透传，只是没有 raw 语义。
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
            Ok(Self { fd, orig: Some(orig) })
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
/// pixel 要透传给内部 pty，否则 icat 这类靠 TIOCGWINSZ 读 ws_xpixel/ws_ypixel
/// 的命令会认为终端不支持 pixel 报告而报错。
fn tty_size() -> Option<(u16, u16, u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_row, ws.ws_col, ws.ws_xpixel, ws.ws_ypixel))
    } else {
        None
    }
}
