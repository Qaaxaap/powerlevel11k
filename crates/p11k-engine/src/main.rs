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
//! `[[ -z "$P11K_ENGINE" ]] && exec p11k`（见 README）。引擎入口据此：
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

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use theme::HeaderInfo;

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

fn main() -> anyhow::Result<()> {
    // 递归检测：P11K_ENGINE 已设 = 本引擎是被内部 shell 的 rc 引导再次调用的
    // 多余实例（用户 rc 里引导行忘了加判断，或写错）。不 panic、不加载用户
    // rc，降级为 exec 一个干净 shell（不读 rc，不触发引导），并提示用户修。
    // 内部 shell 是父进程，本实例 exec 干净 shell 后它照常继续。
    if std::env::var_os("P11K_ENGINE").is_some() {
        eprintln!(
            "p11k: 检测到递归加载（已在 p11k 会话内又启动了 p11k）。\n\
             p11k: 请确认 ~/.zshrc 里的引导行带判断，例如：\n\
             p11k:   [[ -z \"$P11K_ENGINE\" ]] && exec p11k"
        );
        use std::os::unix::process::CommandExt;
        // exec 替换本进程为干净 shell（-f = 不读任何 rc，避免再次触发引导）。
        let err = std::process::Command::new("zsh").arg("-f").exec();
        eprintln!("p11k: exec zsh -f 失败: {err}");
        std::process::exit(1);
    }

    let state = StateDir::create()?;
    log(&format!(
        "engine start: dir={} announce={} ack={} placeholder={:?}",
        state.dir.display(),
        state.announce.display(),
        state.ack.display(),
        PLACEHOLDER
    ));

    // 真实终端（stdin 所在的 pty）必须设为 raw 模式：关掉 ISIG/ICANON/ECHO，
    // 让所有字节原样透传给 pty 里的 shell。否则 ^C 会在真实终端一侧被内核
    // 转成 SIGINT 杀掉引擎（shell 根本收不到），输入还会被内核回显造成双回显。
    // shell 自己的终端（portable-pty 的 slave）由 shell 自己管理 termios。
    let _raw = RawTerminal::enter(libc::STDIN_FILENO)?;

    let (rows, cols) = tty_size().unwrap_or((24, 80));
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new("zsh");
    cmd.env("ZDOTDIR", &state.dir);
    cmd.env("P11K_ANNOUNCE", &state.announce);
    cmd.env("P11K_ACK", &state.ack);
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

    let mut ann_processed: u64 = 0; // announce 文件已消费字节数
    let mut last_size = (rows, cols);
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
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
                Ok(n) => writer.write_all(&buf[..n])?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // pty 输出 → 真实终端：纯透传。zsh 渲染什么（命令输出、zle、占位
        // prompt、补全菜单）引擎一概不管，真实终端照画；主题由 announce
        // 驱动的两笔绘制完成（h → header 行；p → 输入行前缀顶掉占位符）。
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell 退出
                Ok(n) => {
                    stdout.write_all(&buf[..n])?;
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
            let (r, c) = tty_size().unwrap_or(last_size);
            if (r, c) != last_size {
                pair.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: 0,
                    pixel_height: 0,
                })?;
                last_size = (r, c);
            }
            match msg {
                AnnMsg::Header(info) => {
                    log(&format!(
                        "h: exit={:?} cwd={:?}",
                        info.exit_code, info.cwd
                    ));
                    theme::render_header(&mut stdout, c as usize, &info)?;
                    stdout.flush()?;
                    File::create(&state.ack)?; // 放行 precmd → zsh 输出占位符
                    log("drew header, acked");
                }
                AnnMsg::Prompt => {
                    log("p: draw prompt prefix");
                    theme::render_prompt(&mut stdout)?;
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

/// 引擎的临时工作目录：ZDOTDIR（.zshrc）+ announce/ack 文件。
struct StateDir {
    dir: PathBuf,
    announce: PathBuf,
    ack: PathBuf,
}

impl StateDir {
    fn create() -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("p11k-{}", process::id()));
        fs::create_dir_all(&dir)?;
        // 默认用内置模板（无主题 + precmd 宣告）。P11K_ZSHRC 可指向外部
        // .zshrc（比如手动改过的用户配置副本），引擎原样使用。
        let zshrc = match std::env::var("P11K_ZSHRC") {
            Ok(path) => fs::read_to_string(&path)
                .unwrap_or_else(|_| ZSHRC_TEMPLATE.to_string()),
            Err(_) => ZSHRC_TEMPLATE.to_string(),
        };
        fs::write(dir.join(".zshrc"), zshrc)?;
        Ok(Self {
            announce: dir.join("announce"),
            ack: dir.join("ack"),
            dir,
        })
    }
}

/// announce 消息：prompt 窗口的两笔绘制。
enum AnnMsg {
    /// `h\t<exit>\t<cwd>`：precmd 宣告（zsh 在等 ack），画 header 行。
    Header(HeaderInfo),
    /// `p`：zle-line-init 宣告（占位 prompt 已显示），画输入行前缀顶掉。
    Prompt,
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

/// 真实终端尺寸（stdout 的 TIOCGWINSZ）。
fn tty_size() -> Option<(u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_row, ws.ws_col))
    } else {
        None
    }
}
