//! Engine-side terminal state: the temp rc directory and raw mode on the real terminal.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::cli::Shell;
use crate::templates::{BASHRC_TEMPLATE, FISH_TEMPLATE, PWSH_TEMPLATE, ZSHRC_TEMPLATE};

/// The engine's temp working dir: rc files (per shell) + the announce/ack files.
pub struct StateDir {
    pub dir: PathBuf,
    /// Path of the protocol rc file (zsh points ZDOTDIR at it; bash uses --rcfile).
    pub rc: PathBuf,
    pub announce: PathBuf,
    pub ack: PathBuf,
}

impl StateDir {
    pub fn create(shell: Shell) -> io::Result<Self> {
        // State, not temp: the run directory holds the rc files and the protocol files, and a
        // crash leaves it behind for a bug report. `Drop` removes it on a clean exit.
        let dir = crate::paths::state_home().join(format!("run-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        // A FIFO, not a plain file: the engine waits on it with poll instead of re-reading a
        // file every few milliseconds. The shell writes with `>>`, which opens the write end
        // and blocks only until the engine's read end is available.
        let announce = dir.join("announce");
        let c_path = std::ffi::CString::new(announce.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "announce path"))?;
        // SAFETY: NUL-terminated path, mode 0600 inside the state directory.
        if unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) } != 0 {
            return Err(io::Error::last_os_error());
        }

        // One protocol rc per shell. The temp rc read path is kept for debugging.
        let rc = match shell {
            Shell::Zsh => {
                let rc = dir.join(".zshrc");
                let content = match std::env::var("P11K_ZSHRC") {
                    Ok(path) => {
                        fs::read_to_string(&path).unwrap_or_else(|_| ZSHRC_TEMPLATE.to_string())
                    }
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
                // fish redirects via XDG_CONFIG_HOME and reads $XDG_CONFIG_HOME/fish/config.fish.
                let fish_dir = dir.join("fish");
                fs::create_dir_all(&fish_dir)?;
                let rc = fish_dir.join("config.fish");
                fs::write(&rc, FISH_TEMPLATE)?;
                rc
            }
            Shell::Pwsh => {
                // pwsh has no rcfile convention (-Command dot-sources it).
                let rc = dir.join("p11k.ps1");
                fs::write(&rc, PWSH_TEMPLATE)?;
                rc
            }
        };
        Ok(Self {
            announce,
            ack: dir.join("ack"),
            dir,
            rc,
        })
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Raw mode for the real terminal: save the original termios on entry and restore it on
/// Drop. Same job as tmux/screen and other terminal multiplexers: pass every byte through
/// and let the shell in the pty handle signals and echo on its own terminal.
pub struct RawTerminal {
    fd: i32,
    orig: Option<libc::termios>,
}

impl RawTerminal {
    pub fn enter(fd: i32) -> io::Result<Self> {
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

/// Real terminal size (TIOCGWINSZ on stdout), including pixel dimensions.
pub fn tty_size() -> Option<(u16, u16, u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_row, ws.ws_col, ws.ws_xpixel, ws.ws_ypixel))
    } else {
        None
    }
}
