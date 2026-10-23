//! Command line, theme loading and shell selection.

use std::path::{Path, PathBuf};
use std::process;

use crate::config::Config;
use crate::geometry::Geometry;
use crate::i18n::{msgid, t};
use crate::presets;

/// The shell the engine proxies.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
    Pwsh,
}

impl Shell {
    pub fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
            Shell::Pwsh => "pwsh",
        }
    }

    /// Whether the line editor opens a fresh row when the cursor is not at column 0. zle and
    /// PSReadLine do; readline and fish repaint over the current row instead. The engine draws an
    /// input-line prefix during the instant phase, so this decides how far the real header has to
    /// step back to land on the rows the instant header occupies.
    pub fn prompt_starts_on_a_new_row(self) -> bool {
        matches!(self, Shell::Zsh | Shell::Pwsh)
    }
}

/// The `--shell <name>` argument, verbatim: an unrecognised name has to fall through to the
/// config and `$SHELL`, so the caller cannot resolve it here.
fn shell_name_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--shell")?;
    args.get(pos + 1).cloned()
}

/// Read the KDL theme file from `--config <path>`.
fn config_from_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--config")?;
    args.get(pos + 1).map(PathBuf::from)
}

/// Take a built-in preset name (lean/classic/rainbow/pure) from `--preset <name>`.
fn preset_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--preset")?;
    args.get(pos + 1).cloned()
}

/// Read and parse one KDL theme file.
pub fn load_config_file(path: &Path) -> Result<Config, String> {
    std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|src| Config::parse(&src))
}

/// The theme file the engine should read: `--config <path>` if given, otherwise the default
/// location, when a theme was written there (by `p11k configure` or by hand).
pub fn theme_file() -> Option<PathBuf> {
    config_from_args().or_else(|| {
        let path = crate::config::default_config_path();
        path.is_file().then_some(path)
    })
}

/// Load the theme: `--config` file > `--preset` > the theme file in the config directory >
/// the built-in lean theme. The first three report and fall back on failure.
pub fn load_config() -> Config {
    let fallback = || Config::default_lean().expect("built-in lean config should be valid");
    if let Some(path) = config_from_args() {
        match load_config_file(&path) {
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
    let default_path = crate::config::default_config_path();
    if default_path.is_file() {
        match load_config_file(&default_path) {
            Ok(c) => return c,
            Err(e) => eprintln!(
                "p11k: {}{default_path:?}: {e}",
                t("cannot read config, falling back to the built-in lean theme: ")
            ),
        }
    }
    fallback()
}

/// `p11k reload`: re-read the theme file of the session this shell belongs to.
///
/// The engine exports `P11K_CONFIG` and `P11K_ENGINE_PID` into the inner shell. The file is
/// parsed here first, so a syntax error is reported by this command in the ordinary way
/// instead of leaving the prompt silently unchanged; the engine then reads it again and keeps
/// the theme it already has if the file stopped parsing in between.
pub fn reload_engine() -> anyhow::Result<()> {
    let pid = std::env::var("P11K_ENGINE_PID")
        .ok()
        .and_then(|p| p.parse::<libc::pid_t>().ok());
    let Some(pid) = pid else {
        eprintln!(
            "p11k: {}",
            t("not inside a p11k session (P11K_ENGINE_PID is unset)")
        );
        process::exit(1);
    };
    let path = std::env::var("P11K_CONFIG").unwrap_or_default();
    if path.is_empty() {
        eprintln!(
            "p11k: {}",
            t("this session has no theme file: it was started with --preset or the built-in theme")
        );
        process::exit(1);
    }
    let path = Path::new(&path);
    let new_config = match load_config_file(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("p11k: {path:?}: {e}");
            process::exit(1);
        }
    };
    // The inner shell prints the header rows and the prefix from the values the engine handed it
    // at startup, so a theme that changes either of them cannot take effect here. The check runs
    // in this command rather than in the engine because only the command has a terminal to report
    // on: the engine would have to write the message into the prompt area.
    let (handed, shell) = match (
        std::env::var("P11K_SHELL"),
        std::env::var("P11K_PROMPT_LEAD"),
        std::env::var("P11K_PREFIX_OK"),
        std::env::var("P11K_PREFIX_ERR"),
    ) {
        (Ok(shell_name), Ok(lead), Ok(prefix_ok), Ok(prefix_err)) => {
            match shell_by_name(&shell_name) {
                Some(shell) => {
                    let handed = Geometry {
                        header_rows: lead.matches('\n').count(),
                        prompt_lead: lead,
                        prefix_ok,
                        prefix_err,
                    };
                    (handed, shell)
                }
                None => {
                    eprintln!(
                        "p11k: {}",
                        t("unknown shell in P11K_SHELL; cannot check the theme against it")
                    );
                    process::exit(1);
                }
            }
        }
        _ => {
            eprintln!(
                "p11k: {}",
                t(
                    "this session does not expose its prompt geometry; upgrade p11k or restart the session"
                )
            );
            process::exit(1);
        }
    };
    let wanted = Geometry::new(&new_config, shell);
    if let Some(conflict) = handed.reload_conflict(&wanted) {
        eprintln!("p11k: {}", conflict.message());
        process::exit(1);
    }
    // SIGUSR1 only raises a flag; the engine re-reads the file on its next loop pass.
    if unsafe { libc::kill(pid, libc::SIGUSR1) } != 0 {
        eprintln!(
            "p11k: {}",
            t("cannot signal the engine (the session may have exited)")
        );
        process::exit(1);
    }
    Ok(())
}

/// Which shell to proxy: `--shell <name>` if given, else the `shell` node of the theme,
/// else `$SHELL`.
///
/// An unrecognised name from the theme is reported and ignored, so a typo falls back to
/// `$SHELL` rather than silently proxying the wrong shell. Detecting PowerShell via
/// `PSModulePath`/`PSHOME` was tried before (pwsh does not change `$SHELL`, so starting pwsh
/// from zsh still reports /bin/zsh), but CI runner environments happen to carry such
/// variables, which made every default-shell launch look like pwsh and fail to start.
pub fn detect_shell(config: &Config) -> Shell {
    let cli = shell_name_from_args();
    if let Some(name) = config.shell.as_deref()
        && shell_by_name(name).is_none()
    {
        eprintln!(
            "p11k: {}{name:?}",
            t(
                "unknown shell in the config, falling back to $SHELL (expected zsh/bash/fish/pwsh): "
            )
        );
    }
    let env = std::env::var("SHELL").unwrap_or_default();
    pick_shell(cli.as_deref(), config.shell.as_deref(), &env).unwrap_or(Shell::Zsh)
}

/// The three sources of the shell name, in order of precedence. `None` when not one of them
/// names a shell p11k can proxy; the caller then warns and picks a default.
pub fn pick_shell(cli: Option<&str>, configured: Option<&str>, env: &str) -> Option<Shell> {
    cli.and_then(shell_by_name)
        .or_else(|| configured.and_then(shell_by_name))
        .or_else(|| shell_by_name(env.rsplit('/').next().unwrap_or("")))
}

/// Shell name → kind. `None` for a name p11k cannot proxy.
pub fn shell_by_name(name: &str) -> Option<Shell> {
    match name {
        "zsh" => Some(Shell::Zsh),
        "bash" => Some(Shell::Bash),
        "fish" => Some(Shell::Fish),
        "pwsh" | "powershell" => Some(Shell::Pwsh),
        _ => None,
    }
}

/// `--help` usage text. Each line goes through gettext (literals are tagged with `msgid`
/// so xgettext picks them up); empty lines are printed directly to keep an empty msgid out
/// of the po files.
pub fn print_help() {
    for line in [
        msgid("usage: p11k [options]"),
        msgid("       p11k configure"),
        msgid("       p11k reload"),
        "",
        msgid("Options:"),
        msgid(
            "  --shell <name>   inner shell to proxy: zsh, bash, fish or pwsh (default: the `shell` config node, else $SHELL)",
        ),
        msgid(
            "  --config <path>  KDL theme file (default: ~/.config/p11k/p11k.kdl, else the built-in lean theme)",
        ),
        msgid("  --preset <name>  built-in theme: lean, classic, rainbow or pure"),
        msgid("  --version        print the version and exit"),
        msgid("  --help           print this help and exit"),
        "",
        msgid("Commands:"),
        msgid("  configure        interactive theme wizard"),
        msgid("  reload           re-read the theme file of the running session"),
        "",
        msgid("Environment: P11K_ENGINE guards against recursion; P11K_USER_ZSHRC and"),
        msgid("P11K_USER_PROFILE override the rc that gets sourced; P11K_LOCALEDIR"),
        msgid("overrides the translation directory. The engine exports P11K_CONFIG and"),
        msgid("P11K_ENGINE_PID into the shell for `p11k reload`."),
        "",
        msgid("To use p11k as your theme, add one line to your shell rc — see README.md."),
    ] {
        if line.is_empty() {
            println!();
        } else {
            println!("{}", t(line));
        }
    }
}
