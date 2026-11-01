//! Engine-side terminal state: the temp rc directory and raw mode on the real terminal.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
        Self::create_in(&crate::paths::state_home(), shell)
    }

    /// Create the run directory under `base`. Split out of [`StateDir::create`] so a test can
    /// use a temporary directory instead of the user's state home.
    fn create_in(base: &Path, shell: Shell) -> io::Result<Self> {
        // State, not temp: the run directory holds the rc files and the protocol files, and a
        // crash leaves it behind for a bug report. `Drop` removes it on a clean exit.
        let dir = create_run_dir(base)?;
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

/// Create `base/run-<pid>`, replacing a directory a crashed engine left behind.
///
/// The directory has to be new: the shell polls for the ack file, and a stale ack would answer
/// the first prompt with the wrong shape. A stale directory also collides with a reused pid, and
/// `mkfifo` then fails with EEXIST so the engine never starts.
fn create_run_dir(base: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(base)?;
    remove_stale_runs(base);
    let dir = base.join(format!("run-{}", std::process::id()));
    match fs::create_dir(&dir) {
        Ok(()) => Ok(dir),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_dir_all(&dir)?;
            fs::create_dir(&dir)?;
            Ok(dir)
        }
        Err(e) => Err(e),
    }
}

/// Remove `run-<pid>` directories whose process is gone.
///
/// A crash leaves the directory behind, and after a pid reuse it blocks the next start. A live
/// pid keeps its directory: another engine may own it.
fn remove_stale_runs(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let own_pid = std::process::id() as libc::pid_t;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|name| name.strip_prefix("run-"))
            .and_then(|rest| rest.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        // SAFETY: signal 0 only checks whether the pid exists.
        if unsafe { libc::kill(pid, 0) } == 0 {
            continue;
        }
        // ESRCH means no such process. Any other error (EPERM) means it exists but is not ours
        // to signal; keep it.
        if io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A base directory for one test, unique per test name and process.
    fn temp_base(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("p11k-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        base
    }

    /// A crashed engine can leave its run directory behind, and the kernel can then reuse its
    /// pid. The next start finds both the directory and an old ack file; it must replace them.
    #[test]
    fn a_stale_run_directory_is_replaced() {
        let base = temp_base("stale");
        let first = StateDir::create_in(&base, Shell::Zsh).unwrap();
        let dir = first.dir.clone();
        // Keep the directory on disk the way a crash does.
        std::mem::forget(first);
        fs::write(dir.join("ack"), "3\nstale\n1\n").unwrap();

        let second = StateDir::create_in(&base, Shell::Zsh).unwrap();
        assert!(
            !second.ack.exists(),
            "a stale ack file must not survive the restart"
        );
        assert!(second.dir.is_dir());

        drop(second);
        let _ = fs::remove_dir_all(&base);
    }

    /// Run directories of processes that no longer exist are garbage: a crash leaves them behind
    /// and nobody removes them.
    #[test]
    fn run_directories_of_dead_processes_are_removed() {
        let base = temp_base("dead");
        fs::create_dir_all(&base).unwrap();
        let dead = base.join("run-999999999");
        fs::create_dir(&dead).unwrap();

        let state = StateDir::create_in(&base, Shell::Zsh).unwrap();
        assert!(!dead.exists(), "a run directory without a process is stale");

        drop(state);
        let _ = fs::remove_dir_all(&base);
    }

    /// Another engine can run at the same time. Its directory belongs to a live pid and must not
    /// be touched.
    #[test]
    fn run_directories_of_live_processes_are_kept() {
        let base = temp_base("live");
        fs::create_dir_all(&base).unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let live = base.join(format!("run-{}", child.id()));
        fs::create_dir(&live).unwrap();

        let state = StateDir::create_in(&base, Shell::Zsh).unwrap();
        let kept = live.is_dir();
        let _ = child.kill();
        let _ = child.wait();
        assert!(kept, "a live pid keeps its run directory");

        drop(state);
        let _ = fs::remove_dir_all(&base);
    }
}
