//! p11k 引擎（M0，B 架构）：pty 宿主 + 透明代理 + prompt 窗口主题。
//!
//! 职责链：
//! 1. 用 portable-pty 起一个无主题 zsh（ZDOTDIR 指向引擎生成的临时目录，
//!    .zshrc 里只设输入行 PROMPT 和一个 precmd 宣告钩子）。
//! 2. 双向透传：pty 输出 → 真实终端 stdout；真实终端 stdin → pty。
//!    vim / 命令输出由真实终端（kitty）直接渲染，引擎零介入。
//! 3. shell 的 precmd 宣告"要出 prompt 了"（append 到 announce 文件）；
//!    引擎收到后：排空 pty → 画 header（主题）→ touch ack 文件放行。
//!    precmd 轮询到 ack 才返回，zle 随后渲染输入行——保证 header 先画、
//!    输入行后画，两侧几何不打架。
//!
//! 已知局限（M0 记录，后续处理）：
//! - 引擎退出即 pty master 关闭，shell 收到 SIGHUP 一起退出（透传架构固有，
//!   后续可用 keepalive 子进程接管 pty）。
//! - resize 时 zle 全屏重绘会冲掉 header（zle 不知道 header 存在），M0 不做
//!   重画，记录在案。

mod theme;

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use theme::{Cursor, HeaderInfo};

/// 引擎画 header 的行数。必须与下面 PROMPT 占位的空行数一致。
const HEADER_ROWS: usize = 2;

/// 生成给 shell 的 bootstrap .zshrc。
///
/// 几何协议：PROMPT 渲染「占位」——HEADER_ROWS 个空行 + 输入行 ❯，让
/// zle 认为 prompt 占 HEADER_ROWS+1 行，与真实终端（引擎画 header 后）一致。
/// 顺序：precmd 记录状态 → zle 渲染占位（几何建立）→ zle-line-init 宣告 →
/// 引擎把 header 内容填进占位行 → ack 放行输入。
const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap —— 无主题 shell。
# 几何：PROMPT 是占位（header 空行 + ❯ 输入行），zle 认为 prompt 占
# HEADER_ROWS+1 行，与真实终端一致；引擎在 zle-line-init 后填充 header。
PROMPT=$'\n\n\e[1;32m❯\e[0m '
RPROMPT=''

_p11k_status=0
_p11k_pwd=$PWD

# precmd：只记录退出码和目录（$? 只有这里还是命令的退出码）。
_p11k_precmd() {
  _p11k_status=$?
  _p11k_pwd=$PWD
}
precmd_functions=(_p11k_precmd $precmd_functions)

# zle 渲染完占位（几何已建立，光标在 ❯ 后）后宣告。注意：不能在 zle hook
# 里跑外部命令（sleep 等会挂掉 zle），所以只写文件，引擎异步填充 header
# （引擎 poll 间隔 ~5ms，通常早于用户输入）。
_p11k_line_init() {
  print -r -- "f"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd" >> "$P11K_ANNOUNCE"
}
zle -N zle-line-init _p11k_line_init

# 补全：默认流式 list 菜单重绘时不把光标移回输入行（zsh 固有错位，
# 裸 zsh -f 也复现）；oh-my-zsh 的 menu select 模式带光标管理
# （\e[A 移回 + 重绘），必须显式启用。
zstyle ':completion:*:*:*:*:*' menu select
zstyle ':completion:*' special-dirs true
zstyle ':completion:*:cd:*' tag-order local-directories directory-stack path-directories
"#;

fn main() -> anyhow::Result<()> {
    let state = StateDir::create()?;
    log(&format!(
        "engine start: dir={} announce={} ack={}",
        state.dir.display(),
        state.announce.display(),
        state.ack.display()
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
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let master_fd = pair
        .master
        .as_raw_fd()
        .expect("pty master fd is available on unix");
    let mut writer = pair.master.take_writer()?;

    // 预创建 announce 文件，保证 shell 的 >> 追加不报错。
    File::create(&state.announce)?;

    let mut cursor = Cursor::default();
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

        // pty 输出 → 真实终端（透传）+ 游标跟踪。
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell 退出
                Ok(n) => {
                    stdout.write_all(&buf[..n])?;
                    stdout.flush()?;
                    cursor.feed(&buf[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // prompt 时刻：zle 渲染完占位（几何建立），引擎填充 header 内容。
        for info in drain_announce(&state.announce, &mut ann_processed) {
            log(&format!(
                "got announce: exit={:?} cwd={:?}",
                info.exit_code, info.cwd
            ));
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

            // 填充：光标此刻在输入行（❯ 后，zle 刚渲染完占位），保存位置，
            // 上移 HEADER_ROWS 行画内容，再恢复。zle 不等待，引擎异步完成
            // （poll 间隔 ~5ms，早于用户输入）。
            theme::fill_header(&mut stdout, c as usize, HEADER_ROWS, &info)?;
            stdout.flush()?;
            log("filled header");
        }
    }

    let _ = child.kill();
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

/// 读 announce 文件的新行并解析为 prompt 信息。
/// 行格式：`p\t<exit_code>\t<cwd>`。
fn drain_announce(path: &Path, processed: &mut u64) -> Vec<HeaderInfo> {
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
        // `f` = fill：zle 渲染完占位，请求填充 header。
        if parts.next() != Some("f") {
            continue;
        }
        let code = parts.next().and_then(|s| s.trim().parse::<i32>().ok());
        let cwd = parts
            .next()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        out.push(HeaderInfo {
            exit_code: code,
            cwd,
        });
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
