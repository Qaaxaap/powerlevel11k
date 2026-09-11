//! p11k engine: pty host + terminal rendering layer.
//!
//! Chain of responsibility:
//! 1. Start a themeless shell with portable-pty (ZDOTDIR points at a temp dir the engine
//!    generates; .zshrc sets only a placeholder PROMPT and the announce hooks).
//! 2. Pass through: pty output → real terminal stdout; real terminal stdin → pty. The
//!    engine does not parse ANSI from the pty: command output, zle, placeholder prompt
//!    and completion menus are all rendered by the shell, untouched.
//! 3. The prompt window is painted in two announce-driven strokes:
//!    - `h` (precmd announce): emit the OSC133A marker + touch ack to let the shell print
//!      the placeholder; the header is not drawn here.
//!    - `p` (zle-line-init announce, zle has rendered the placeholder prompt): move up
//!      header rows to backfill the real header, then return to line start and paint the
//!      real prefix over the placeholder. The prefix covers only the placeholder columns,
//!      so a slow engine or a user already typing does not matter.
//!
//! Geometry protocol: PROMPT carries header_rows newlines + a "__" placeholder so the
//! shell's prompt geometry counts the header rows too.
//! precmd writes announce then polls ack, ensuring the placeholder is printed first and
//! the header backfilled after.
//!
//! Deployment: add the exec launch to the user rc and use P11K_ENGINE to avoid recursion
//! `[[ -z "$P11K_ENGINE" ]] && exec p11k --shell zsh` (see README).
//! - Normal (P11K_ENGINE unset): set `P11K_ENGINE=1` when spawning the inner shell;
//! - Recursive (P11K_ENGINE set): an unguarded bootstrap line in the user rc recursed;
//!   start a clean shell and print a repair hint without loading the user rc.
//!
//! The inner shell is orphaned via double-fork (it leaves the engine process tree). When
//! the engine exits, master closes → slave hangs up → the inner shell gets SIGHUP and
//! exits. While drawing header/prefix the engine emits OSC 133 A/B prompt markers; GUI
//! close confirmation uses them to tell the cursor is at the prompt and stops asking.

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

/// Real terminal SIGWINCH (resize) flag: the handler only sets it (signal-safe); the main
/// loop then syncs the inner pty size → the inner zsh gets SIGWINCH → TRAPWINCH announces r
/// → the engine clears and repaints the prompt window.
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_FLAG.store(true, Ordering::Relaxed);
}

use config::Config;
use i18n::{msgid, t};
use p11k_gitstatus::{options::Options, protocol::field, repo::RepoCache};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use theme::{GitStatus, HeaderInfo};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Shell {
    Zsh,
    Bash,
    Fish,
    Pwsh,
}

impl Shell {
    fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
            Shell::Pwsh => "pwsh",
        }
    }
}

fn shell_from_args() -> Option<Shell> {
    let args: Vec<String> = std::env::args().collect();
    let pos = args.iter().position(|a| a == "--shell")?;
    match args.get(pos + 1).map(|s| s.as_str()) {
        Some("zsh") => Some(Shell::Zsh),
        Some("bash") => Some(Shell::Bash),
        Some("fish") => Some(Shell::Fish),
        Some("pwsh") | Some("powershell") => Some(Shell::Pwsh),
        _ => None,
    }
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

/// Load the theme: --config file > --preset built-in > default lean; the first two log and
/// fall back on failure.
fn load_config() -> Config {
    let fallback = || Config::default_lean().expect("built-in lean config should be valid");
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

/// Fall back to $SHELL when no shell is given.
///
/// Only `$SHELL` is read. Detecting PowerShell via `PSModulePath`/`PSHOME` was tried
/// before (pwsh does not change `$SHELL`, so starting pwsh from zsh still reports
/// /bin/zsh), but CI runner environments happen to carry such variables, which made every
/// default-shell launch look like pwsh and fail to start. The install line always writes
/// `--shell <name>` explicitly; this is only the fallback when it is missing.
fn detect_shell() -> Shell {
    if let Some(s) = shell_from_args() {
        return s;
    }
    let she = std::env::var("SHELL").unwrap_or_default();
    match she.rsplit('/').next().unwrap_or("") {
        "bash" => Shell::Bash,
        "fish" => Shell::Fish,
        "pwsh" | "powershell" => Shell::Pwsh,
        _ => Shell::Zsh,
    }
}

/// Generate the bootstrap .zshrc handed to the shell.
/// Sequence: precmd announces `h` → polls ack → zsh renders the placeholder PROMPT (zle)
/// → zle-line-init announces `p` → the engine returns to line start and paints the prefix
/// over the placeholder. The placeholder prompt is rendered by zle or its equivalent; the
/// theme is always the engine's job.
const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap -- protocol layer + user config.
# ===== engine protocol =====
# Empty when the engine does not enable transient; clear it so a stale value
# left in the outer environment cannot make zle-line-finish fold wrongly.
[[ -n "${P11K_TRANSIENT_PROMPT:-}" ]] || unset P11K_TRANSIENT_PROMPT
_p11k_status=0
_p11k_pwd=$PWD

_p11k_precmd() {
  _p11k_status=$?
  _p11k_pwd=$PWD
  # Used to restore PROMPT after the transient fold
  PROMPT='__'
  print -r -- "h"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd"$'\t'"${#jobstates}"$'\t'"$HISTCMD" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}

# Announce `p` after zle renders the placeholder prompt; the engine then paints the real
# prefix over it at line start. The zle hook only writes the file; the engine polls (~5ms).
_p11k_line_init() {
  [[ ${CONTEXT:-start} == cont ]] && return
  if [[ -n "${_p11k_user_line_init:-}" ]] && (( $+functions[$_p11k_user_line_init] )); then
    "$_p11k_user_line_init"   # user-registered handler (e.g. autosuggestions) runs first
  fi
  print -r -- "p" >> "$P11K_ANNOUNCE"
  # Report the current keymap: vi mode (zle resets to viins at prompt-ready) reports viins →
  # INSERT; emacs (main) reports main → the vi_mode segment hides. Switching (NORMAL/VISUAL)
  # is reported by keymap-select.
  if [[ ${options[vi]} == on ]]; then
    print -r -- "v"$'\t'"viins" >> "$P11K_ANNOUNCE"
  else
    print -r -- "v"$'\t'"main" >> "$P11K_ANNOUNCE"
  fi
  # Prompt is ready, clear the anti-recursion flag
  unset P11K_ENGINE
}

# ===== user config =====
# Prefer $P11K_USER_ZSHRC, otherwise default to $HOME/.zshrc.
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi

# ===== protocol invariants =====
# PROMPT is a placeholder.
PROMPT='__'
RPROMPT=''
PROMPT2='> '
precmd_functions=(${precmd_functions:#_p11k_precmd} _p11k_precmd)
# zle-line-init takes one handler: keep the user's, then register the engine announce.
if [[ -n "${widgets[zle-line-init]:-}" && "${widgets[zle-line-init]}" != user:_p11k_line_init ]]; then
  _p11k_user_line_init=${widgets[zle-line-init]#user:}
fi
zle -N zle-line-init _p11k_line_init
# vi_mode: report the current keymap to the engine on zle-keymap-select.
_p11k_vi_mode() {
  local m=$KEYMAP
  if [[ ${options[vi]} == on ]]; then
    case $KEYMAP in
      vicmd|vis|viopp) m=$KEYMAP;;
      *) m=viins;;  # zsh may report main while inserting → normalize to viins
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
# transient: on command submit fold the multi-line header into a single ❯ line.
# Let zsh render the folded prompt itself.
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
# resize announce: SIGWINCH → zsh defers the TRAPWINCH run,
# detects the change, announces `r`; the engine clears and repaints the prompt window.
_p11k_last_cols=$COLUMNS
_p11k_last_rows=$LINES
TRAPWINCH() {
  if (( COLUMNS != _p11k_last_cols || LINES != _p11k_last_rows )); then
    _p11k_last_cols=$COLUMNS
    _p11k_last_rows=$LINES
    print -r -- "r" >> "$P11K_ANNOUNCE"
  fi
}

# Completion: compinit plus complist/menu select.
# Do not touch the completion coloring (list-colors) here: setting it empty makes zsh fall
# back to its built-in default, directories turn bold red, unlike the user rc
# (oh-my-zsh derives di=01;34 blue from LS_COLORS).
autoload -Uz compinit && compinit
zmodload -i zsh/complist
setopt auto_menu complete_in_word always_to_end
unsetopt menu_complete
zstyle ':completion:*:*:*:*:*' menu select
zstyle ':completion:*' matcher-list 'm:{[:lower:][:upper:]}={[:upper:][:lower:]}' 'r:|=*' 'l:|=* r:|=*'
zstyle ':completion:*' special-dirs true
zstyle ':completion:*:cd:*' tag-order local-directories directory-stack path-directories
"#;

/// bash has no `p` announce: after the ack bash prints PS1 (`__`) directly, and the engine
/// byte-matches it to paint the prefix over it. resize detects size changes via trap WINCH
/// and announces `r`.
const BASHRC_TEMPLATE: &str = r#"# p11k engine bootstrap (bash) -- protocol layer + user config.
PS1='__'

# PROMPT_COMMAND runs before PS1 is displayed: record the exit code, announce `h`,
# then wait for the engine ack before returning.
_p11k_prompt_command() {
  local _st=$?
  printf 'h\t%s\t%s\t%s\n' "$_st" "$PWD" "$(jobs -p | wc -l)" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}
PROMPT_COMMAND=_p11k_prompt_command

# resize: after SIGWINCH bash updates COLUMNS/LINES; the trap detects the change, announces `r`.
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

# ===== user config =====
if [[ -n "${P11K_USER_ZSHRC:-}" && -r "$P11K_USER_ZSHRC" ]]; then
  source "$P11K_USER_ZSHRC"
elif [[ -r "$HOME/.bashrc" ]]; then
  source "$HOME/.bashrc"
fi

# ===== protocol invariants =====
PS1='__'
PROMPT_COMMAND=_p11k_prompt_command
trap '_p11k_winch' WINCH
"#;

/// fish protocol layer (injected via XDG_CONFIG_HOME redirection). fish_prompt placeholder + announce.
///
/// fish has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in `fish_prompt`, which
/// returns the placeholder after the ack; the engine byte-matches the placeholder and
/// paints the prefix over it (same as bash, no `p` announce). resize uses `--on-signal WINCH`.
const FISH_TEMPLATE: &str = r#"# p11k engine bootstrap (fish) -- protocol layer + user config.

set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES

# fish_prompt is called when the main prompt is rendered. Two cases:
# - size change (resize repaint): announce `r`.
# - normal prompt: announce `h`, then wait for the engine ack before returning.
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

# ===== user config =====
if test -n "$P11K_USER_ZSHRC"; and test -r "$P11K_USER_ZSHRC"
    source "$P11K_USER_ZSHRC"
else if test -r "$HOME/.config/fish/config.fish"
    source "$HOME/.config/fish/config.fish"
end

# ===== protocol invariants =====
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

/// pwsh protocol layer (injected via `-NoProfile -NoExit -Command ". <rc>"`).
///
/// pwsh likewise has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in the prompt
/// function, which returns the placeholder only after the engine ack → the engine
/// byte-matches the placeholder in the pass-through stream and paints the prefix over it
/// (same as bash/fish, no `p` announce). resize compares the window size inside prompt and
/// then announces `r`: pwsh does not turn SIGWINCH into a hookable event.
///
/// The body is a standalone function and prompt only forwards: after a user profile
/// overrides prompt, the tail points it back at the engine implementation.
const PWSH_TEMPLATE: &str = r#"# p11k engine bootstrap (pwsh) -- protocol layer + user config.
$P11kAnnounce = $env:P11K_ANNOUNCE
$P11kAck = $env:P11K_ACK
$script:P11kCols = $Host.UI.RawUI.WindowSize.Width
$script:P11kRows = $Host.UI.RawUI.WindowSize.Height

function global:P11kEnginePrompt {
    # $? must be read on the first line: any later statement in prompt overwrites it.
    $P11kCode = if ($?) { 0 } elseif ($global:LASTEXITCODE -is [int]) { $global:LASTEXITCODE } else { 1 }
    $P11kSize = $Host.UI.RawUI.WindowSize
    if ($P11kSize.Width -ne $script:P11kCols -or $P11kSize.Height -ne $script:P11kRows) {
        $script:P11kCols = $P11kSize.Width
        $script:P11kRows = $P11kSize.Height
        # Announcing only `r` and returning would leave the placeholder on screen: the engine
        # may not be in prompt state right now (just pressed enter), so it neither moves up to
        # backfill nor paints over the placeholder. Hence fall through to the normal announce.
        [IO.File]::AppendAllText($script:P11kAnnounce, "r`n")
    }
    $P11kJobs = @(Get-Job -ErrorAction SilentlyContinue).Count
    # Send the command number, not the history count: zsh reports HISTCMD, and
    # HistoryInfo.Id is PowerShell's nearest equivalent (session-unique and
    # monotonic). .Count drops when MaximumHistoryCount trims; Id survives that.
    # After Clear-History there is nothing left to read, so it reports 0.
    $P11kHist = if ($h = Get-History -Count 1) { $h.Id } else { 0 }
    [IO.File]::AppendAllText($script:P11kAnnounce, "h`t$P11kCode`t$($PWD.Path)`t$P11kJobs`t$P11kHist`n")
    # Wait for the engine ack: the placeholder is printed first, the header is backfilled after.
    while (-not [IO.File]::Exists($script:P11kAck)) { Start-Sleep -Milliseconds 5 }
    [IO.File]::Delete($script:P11kAck)
    # Prompt is ready, clear the anti-recursion flag (same as zsh's zle-line-init): a manual
    # p11k started later in the session is then not treated as a recursive load.
    if ($null -ne $env:P11K_ENGINE) { $env:P11K_ENGINE = $null }
    '__'
}

function global:prompt { P11kEnginePrompt }

# ===== user config =====
$P11kUserProfile = if ($env:P11K_USER_PROFILE) { $env:P11K_USER_PROFILE } else { $PROFILE }
if ($P11kUserProfile -and (Test-Path -LiteralPath $P11kUserProfile)) {
    # A failed dot-source must not break the later protocol invariants that re-declare prompt.
    try { . $P11kUserProfile } catch { Write-Warning "p11k: cannot load $P11kUserProfile`: $_" }
}

# ===== protocol invariants =====
# The user profile may replace prompt; point it back to the engine implementation here.
$P11kAnnounce = $env:P11K_ANNOUNCE
$P11kAck = $env:P11K_ACK
function global:prompt { P11kEnginePrompt }
"#;

/// `--help` usage text. Each line goes through gettext (literals are tagged with `msgid`
/// so xgettext picks them up); empty lines are printed directly to keep an empty msgid out
/// of the po files.
fn print_help() {
    for line in [
        msgid("usage: p11k [options]"),
        msgid("       p11k configure"),
        "",
        msgid("Options:"),
        msgid("  --shell <name>   inner shell to proxy: zsh, bash, fish or pwsh (default: $SHELL)"),
        msgid("  --config <path>  KDL theme file (default: the built-in lean theme)"),
        msgid("  --preset <name>  built-in theme: lean, classic, rainbow or pure"),
        msgid("  --version        print the version and exit"),
        msgid("  --help           print this help and exit"),
        "",
        msgid("Commands:"),
        msgid("  configure        interactive theme wizard"),
        "",
        msgid("Environment: P11K_ENGINE guards against recursion; P11K_USER_ZSHRC and"),
        msgid("P11K_USER_PROFILE override the rc that gets sourced; P11K_LOCALEDIR"),
        msgid("overrides the translation directory."),
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

fn main() -> anyhow::Result<()> {
    // Text is translated by locale (English by default; Chinese in po/zh_CN.po).
    i18n::init();
    // `--version`/`--help` are handled before touching any terminal state so they work in
    // pipes and environments without a tty (CI, version checks from scripts).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("p11k {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    // `p11k configure`: enter the interactive theme wizard without spawning a shell.
    if std::env::args().any(|a| a == "configure") {
        return crate::wizard::run();
    }

    // Recursion check: P11K_ENGINE set means this engine is a redundant instance started
    // again by the inner shell's rc bootstrap (the user rc bootstrap line is unguarded or
    // wrong). Degrade to exec'ing a clean shell and tell the user to fix it.
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
    // Load the config first to compute the input-line prefix width, then build a
    // placeholder of the same width.
    let config = load_config();
    // Every prompt_char state (normal/ERROR) must be the same width.
    // Unequal widths are a config error: report on the real terminal, then exec a clean shell.
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
            Shell::Pwsh => std::process::Command::new("pwsh")
                .args(["-NoProfile", "-NoLogo"])
                .exec(),
        };
        eprintln!("p11k: {}{err}", t("cannot exec a clean shell: "));
        std::process::exit(1);
    }
    let prefix = crate::render::input_prefix(&config, None);
    // The placeholder carries header_rows newlines so the shell's prompt geometry counts
    // the header rows too; only then can zsh fold transient cleanly with reset-prompt.
    // marker is the visible placeholder bytes after the newlines; bash/fish match it to
    // tell the placeholder has been printed.
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

    // The real terminal (the pty stdin lives on) must be in raw mode: turn off
    // ISIG/ICANON/ECHO so every byte is passed through to the shell in the pty. Otherwise
    // ^C is turned into SIGINT by the kernel on the real terminal side and kills the engine
    // (the shell never sees it), and the kernel also echoes input, doubling every character.
    // The shell's own terminal (portable-pty's slave) has its termios managed by the shell.
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
        // zsh: ZDOTDIR points at the engine dir, which holds the .zshrc it reads.
        Shell::Zsh => {
            cmd.env("ZDOTDIR", &state.dir);
        }
        // bash: --rcfile selects the protocol layer (only interactive bash reads rcfile).
        Shell::Bash => {
            cmd.arg("--rcfile");
            cmd.arg(&state.rc);
        }
        Shell::Fish => {
            // fish redirects via XDG_CONFIG_HOME and reads $XDG_CONFIG_HOME/fish/config.fish.
            cmd.env("XDG_CONFIG_HOME", &state.dir);
        }
        Shell::Pwsh => {
            // pwsh has no --rcfile/ZDOTDIR: -NoProfile disables the default profile and
            // -Command dot-sources the engine's protocol script; -NoExit keeps it interactive.
            cmd.arg("-NoProfile");
            cmd.arg("-NoLogo");
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(format!(". '{}'", state.rc.display()));
        }
    }
    cmd.env("P11K_ANNOUNCE", &state.announce);
    cmd.env("P11K_ACK", &state.ack);
    // portable-pty's spawn_command defaults current_dir to HOME; set the engine's startup
    // cwd explicitly so the inner shell lands where the user ran `exec p11k`.
    cmd.cwd(std::env::current_dir()?);
    // Recursion flag: if the inner shell or a child execs p11k again, the entry check
    // degrades it.
    cmd.env("P11K_ENGINE", "1");
    // transient is zsh-only: hand the shell the single-line prompt the engine precomputes
    // (zsh %F escapes); zle-line-finish swaps PROMPT + reset-prompt to fold it in sync.
    if config.layout.transient_prompt && shell == Shell::Zsh {
        cmd.env(
            "P11K_TRANSIENT_PROMPT",
            crate::render::transient_prompt_zsh(&config),
        );
    } else {
        cmd.env("P11K_TRANSIENT_PROMPT", "");
    }

    // double-fork orphaning: the inner shell leaves the engine process tree (its parent
    // becomes init). When closing a window kitty looks at the descendants of its child; an
    // orphan is not in that tree → no "confirm close" dialog. The engine still reads and
    // writes the master fd (orphaning does not break it); when the engine exits, master
    // closes → slave hangs up → the inner shell gets SIGHUP and exits, so there is no PID
    // to record and kill.
    let mid = unsafe { libc::fork() };
    if mid < 0 {
        anyhow::bail!("fork failed: {}", io::Error::last_os_error());
    }
    if mid == 0 {
        // Middle process: spawn the inner shell (parent = this process), then exit at once
        // so it is orphaned. Only async-signal-safe work is allowed here: the engine is
        // multi-threaded, so calling malloc after fork (println/gettext both do) risks
        // deadlock. The error code goes back to the parent to print.
        let status = match pair.slave.spawn_command(cmd) {
            Ok(_) => 0,
            Err(_) => 1,
        };
        unsafe { libc::_exit(status) };
    }
    // Engine: reap the middle process and drop the slave reference (the inner shell has
    // taken it over).
    let mut _st = 0;
    unsafe { libc::waitpid(mid, &mut _st, 0) };
    // A non-zero middle process means the inner shell failed to start. This used to be
    // silent, leaving only "the shell produced no output" upstream with no way to tell a
    // failed spawn from anything else.
    if _st != 0 {
        eprintln!("p11k: {}", t("the inner shell failed to start"));
    }
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let master_fd = pair
        .master
        .as_raw_fd()
        .expect("pty master fd is available on unix");
    let mut writer = pair.master.take_writer()?;

    // Pre-create the announce file so the shell's >> appends never fail.
    File::create(&state.announce)?;

    // Catch SIGWINCH on the real terminal so the engine also notices a resize while the
    // prompt is displayed (sync the inner pty → TRAPWINCH announces r → clear and repaint)
    // instead of waiting for the next prompt.
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            handle_sigwinch as extern "C" fn(libc::c_int) as usize,
        );
    }

    let mut ann_processed: u64 = 0; // bytes already consumed from the announce file
    let mut last_size = (rows, cols, xpix, ypix);
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    // Async git status.
    // A worker thread owns the RepoCache and the main loop sends requests / receives
    // results over channels, so a 1-2s first scan of a large repo never stalls the prompt.
    let (req_tx, req_rx) = mpsc::channel::<GitRequest>();
    let (res_tx, res_rx) = mpsc::channel::<GitResult>();
    // The call follows p10k conventions; count caps and the dirty skip threshold come from
    // vcs segment properties (in p10k these are the global POWERLEVEL9K_VCS_*_MAX_NUM and
    // POWERLEVEL9K_VCS_MAX_INDEX_SIZE_DIRTY, -1 = unlimited). Read the values before
    // spawning so the whole config is not moved into the thread.
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
            // Drain the queue and handle only the latest request (drop backlogged old cwds).
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
                break; // the main loop exited, channel closed
            }
        }
    });

    let mut git_gen: u64 = 0; // request sequence number; only the latest result is used
    // Last git status for the current cwd.
    let mut last_vcs: Option<(String, Option<GitStatus>)> = None;
    let mut current_info: Option<HeaderInfo> = None;
    // Whether the cursor is on the input line (after the p announce, before the user
    // presses enter). Async results repaint only then, otherwise redraw's \e[1A lands on
    // command output.
    let mut at_prompt = false;
    // Time of the last enter, used by the next precmd to compute command duration.
    let mut last_enter: Option<std::time::Instant> = None;
    // Defer the prompt repaint after a resize: wait for zle's repainted placeholder+buffer
    // to pass through (~60ms) before painting the prefix, so the prompt is not painted
    // before zle repaints (and gets covered by the placeholder) or ahead of the buffer.
    let mut resize_prompt_at: Option<std::time::Instant> = None;
    // bash has no zle-line-init (no `p` announce): after the ack bash prints the PS1
    // placeholder directly and the engine matches it in the pass-through stream to paint
    // the prefix over it. Content before the placeholder is accumulated in a buffer.
    let mut pending_placeholder = false;
    let mut placeholder_buf: Vec<u8> = Vec::new();
    // Whether the user has typed since the current prompt became ready: the empty enter
    // sent to pwsh on shrink runs only while the input line is still empty, otherwise it
    // would submit a half-written command.
    let mut typed_since_prompt = false;
    // Last raw bytes judged to be user input; diagnostics logging only (escaped).
    let mut last_input: Vec<u8> = Vec::new();
    // Time of the last empty enter sent on pwsh's behalf (dragging a window fires SIGWINCH
    // repeatedly, so it must be throttled).
    let mut last_pwsh_nudge: Option<std::time::Instant> = None;

    // instant header: don't wait for the inner shell to finish loading a slow user rc; draw
    // a placeholder header + prompt from the engine cwd right away so the prompt is visible
    // as soon as the window opens, then swap in the real state after the first inner precmd.
    let instant_info = HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
        exec_seconds: 0.0,
        jobs: 0,
        history: 0,
    };
    // The return value is the display width of each line: if the terminal is narrower than
    // the pty when drawing (a fresh window whose size has not synced yet), content wraps and
    // erasing must convert to actual rows using the column width of that moment, otherwise
    // half a header is left behind.
    let mut instant_widths =
        theme::render_header_cfg(&mut stdout, cols as usize, &config, &instant_info, None)?;
    theme::render_prompt(&mut stdout, &prefix.text)?;
    stdout.flush()?;
    let mut instant_drawn = true;
    /// Terminal rows the instant header actually occupies at the given column count;
    /// content wider than the terminal wraps onto more rows.
    fn instant_rows_at(widths: &[usize], cols: u16) -> usize {
        let c = cols.max(1) as usize;
        widths.iter().map(|w| w.div_ceil(c).max(1)).sum()
    }
    // What the inner shell passed through to the terminal during the instant phase: byte
    // count + newline count. Zero newlines means the only header on screen is ours (safe to
    // erase in place and repaint); otherwise the rc really printed something, those lines
    // sit below ours and must be kept.
    let mut instant_bytes = 0usize;
    let mut instant_newlines = 0usize;

    loop {
        // resize signal: sync the inner pty size (including pixels). zsh/bash/fish all
        // register signal hooks (TRAPWINCH / trap WINCH / --on-signal WINCH) that announce
        // `r` on receipt so the engine relayouts at the new width; pwsh cannot hook SIGWINCH
        // and only compares the window size inside its prompt function, so the block below
        // sends it one empty enter on shrink.
        if RESIZE_FLAG.swap(false, Ordering::Relaxed) {
            // Re-enter raw mode: multiplexers like tmux reset the pane pty termios when
            // creating or resizing a pane. Once ECHO is on, what the engine writes to the
            // terminal is echoed back to stdin and forwarded into the pty as input — and a
            // second copy of the header shows up on screen.
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
                // During the instant phase no shell `r` announce triggers a repaint yet, so
                // the screen still holds the copy drawn at the old column count (possibly
                // wrapped, no longer in place); repaint it here proactively. Clear and redraw
                // only when the instant phase produced no other output: after a terminal
                // reflow "move up N rows" is unreliable, and this is the only safe option.
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
                // pwsh cannot hook SIGWINCH and only compares the window size inside its
                // prompt function, so after a shrink the screen still holds the
                // terminal-reflowed old content until the next prompt relayouts it. Send one
                // empty enter on the user's behalf (only while the input line is still empty)
                // so pwsh calls prompt again → announces r → the engine relayouts at the new
                // width.
                if shell == Shell::Pwsh
                    && at_prompt
                    && !typed_since_prompt
                    && last_pwsh_nudge
                        .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(150))
                {
                    log("resize: nudge pwsh with an empty line so it redraws");
                    let _ = writer.write_all(b"\r");
                    let _ = writer.flush();
                    last_pwsh_nudge = Some(std::time::Instant::now());
                } else if shell == Shell::Pwsh && at_prompt {
                    log(&format!(
                        "resize: no pwsh nudge (typed={typed_since_prompt}, last_input={:?})",
                        String::from_utf8_lossy(&last_input)
                    ));
                }
            }
        }

        // prompt window (announce-driven). Handled before poll/pass-through: in fish's
        // resize the `r` announce and the placeholder output are nearly simultaneous, so
        // drain first to arm pending, then byte-match the placeholder while passing the pty
        // through (otherwise the placeholder passes first and pending is set too late).
        // - `h` (precmd announce, shell waiting for ack): draw header, touch ack to release.
        // - `p` (zle-line-init announce): back to line start, paint the prefix over the placeholder.
        // - `r` (resize announce): repaint the prompt window.
        for msg in drain_announce(&state.announce, &mut ann_processed) {
            // Sync a size change to the pty and let right alignment use the new width.
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
                    // Command duration = last enter → this precmd (0 on the first prompt
                    // without last_enter).
                    info.exec_seconds = last_enter
                        .take()
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    at_prompt = false; // new prompt cycle: the cursor is not on the input line until the header is backfilled
                    if instant_drawn {
                        // First precmd: replace the instant header with the real state.
                        //
                        // Normally (the inner shell printed nothing meanwhile) it is enough to
                        // erase the few rows we drew: move up to the header's first row and
                        // `\x1b[J` to the end of screen — below the cursor there is only our
                        // header + the input line. That leaves existing content above untouched
                        // (previous session output, window banner, anything printed before
                        // `exec p11k`) and does not flash the whole screen like `\x1b[2J`.
                        // p10k likewise does not clear the screen.
                        //
                        // If the inner shell did print (rc echo/warning/error), those rows sit
                        // below ours and "move up k rows" lands inside the output — then do
                        // nothing, keep the output and let the real prompt follow it (p10k also
                        // keeps it and warns). Set P11K_INSTANT_LOG to log which branch was hit.
                        instant_drawn = false;
                        // Query the terminal width again here: if the size had not synced when
                        // the instant header was drawn (fresh window, SIGWINCH not delivered
                        // yet), the content was laid out at the old width and has already
                        // wrapped in the real window, so convert rows using the real columns.
                        let (rows_cap, cols_now, ..) = tty_size().unwrap_or(last_size);
                        let rows = instant_rows_at(&instant_widths, cols_now);
                        if instant_newlines == 0 && rows < rows_cap as usize {
                            write!(stdout, "\x1b[{rows}A\r\x1b[J")?;
                        } else if instant_newlines == 0 {
                            // The window is too short for the header: the header itself has
                            // scrolled the screen and its position is unreliable, so fall back
                            // to a full clear.
                            write!(stdout, "\x1b[2J\x1b[H")?;
                        } else if std::env::var_os("P11K_INSTANT_LOG").is_some() {
                            log(&format!(
                                "instant header kept: {instant_newlines} line(s) / {instant_bytes} bytes of shell output during init"
                            ));
                        }
                    } else if config.layout.prompt_add_newline > 0 {
                        // Relaxed layout: leave N blank rows between consecutive prompts
                        // (blanked before the header).
                        for _ in 0..config.layout.prompt_add_newline {
                            write!(stdout, "\r\n\r\n")?;
                        }
                    }
                    // Emit the prompt-start marker before the shell's multi-line placeholder;
                    // the header is not drawn here, or it would advance the cursor twice on
                    // top of the placeholder's own newlines.
                    theme::prompt_start(&mut stdout)?;
                    stdout.flush()?;
                    File::create(&state.ack)?; // release precmd → the shell prints the placeholder
                    if shell != Shell::Zsh {
                        pending_placeholder = true;
                        placeholder_buf.clear();
                    }
                    // Compute git in the background and repaint the header asynchronously when
                    // it finishes.
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
                    // zle has rendered the multi-line placeholder (header_rows blank lines +
                    // the placeholder) and the cursor sits after it: move up header rows to
                    // backfill the real header, then return to line start and overwrite the
                    // placeholder with the prefix.
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
                    // The prefix is generated dynamically from the current exit code.
                    let text = crate::render::input_prefix(
                        &config,
                        current_info.as_ref().and_then(|i| i.exit_code),
                    )
                    .text;
                    theme::render_prompt(&mut stdout, &text)?;
                    stdout.flush()?;
                    at_prompt = true; // input line ready
                    typed_since_prompt = false;
                }
                AnnMsg::Resize => {
                    // The size check at the top of the loop already resized the pty.
                    // zsh: zle repaints the placeholder prompt and the backfill goes through
                    // the `p` announce; repaint the header here and defer the prefix by ~60ms
                    // (until zle's repainted placeholder+buffer has passed through).
                    // Other shells: after a resize the shell prints the placeholder from
                    // scratch (with new leading newlines) and the marker-matching path above
                    // backfills it — the placeholder is not out yet, so drawing now would land
                    // on the previous screen and leave the placeholder uncovered.
                    if at_prompt {
                        if shell == Shell::Zsh {
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
                            resize_prompt_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_millis(60),
                            );
                        } else {
                            log("r: wait for the shell to reprint the placeholder");
                            pending_placeholder = true;
                            placeholder_buf.clear();
                        }
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
        // 5ms timeout, which doubles as the announce file polling cadence.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 5) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }

        // Real terminal input → pty.
        // Note: on close poll may report POLLHUP without POLLIN, in which case read returns 0.
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 4096];
            match stdin.read(&mut buf) {
                Ok(0) => break, // real terminal closed
                Ok(n) => {
                    // The user pressed enter → the cursor left the input line and async git
                    // results no longer repaint the header (otherwise \e[1A lands in the wrong
                    // place).
                    if buf[..n].iter().any(|&b| b == b'\r' || b == b'\n') {
                        at_prompt = false;
                        typed_since_prompt = false;
                        last_enter = Some(std::time::Instant::now()); // command timing starts
                    } else if !is_terminal_reply(&buf[..n]) {
                        // Terminal replies (DSR cursor position and the like) are not user input.
                        typed_since_prompt = true;
                        last_input.clear();
                        last_input.extend_from_slice(&buf[..n.min(32)]);
                    }
                    writer.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // pty output → real terminal: pass through.
        // The theme is painted in the two announce-driven strokes.
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell exited
                Ok(n) => {
                    if pending_placeholder {
                        placeholder_buf.extend_from_slice(&buf[..n]);
                        if let Some(pos) = find_bytes(&placeholder_buf, marker.as_bytes()) {
                            let end = pos + marker.len();
                            // Pass the placeholder through first (blank lines + placeholder),
                            // moving the cursor past it.
                            stdout.write_all(&placeholder_buf[..end])?;
                            // Move up header rows to backfill the real header, then return to
                            // line start and overwrite the placeholder.
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
                            at_prompt = true; // bash/fish "prompt ready" (equivalent to zsh's p announce)
                            typed_since_prompt = false;
                        } else if placeholder_buf.len() > 8192 {
                            stdout.write_all(&placeholder_buf)?;
                            placeholder_buf.clear();
                            pending_placeholder = false;
                        }
                    } else {
                        // Count pass-through content during the instant phase: a newline means
                        // the inner shell really printed to the screen (rc echo/warning/error),
                        // in which case our own rows cannot be erased (they have been pushed
                        // down); control sequences alone (setting the title and such, common in
                        // user rcs) do not count and still take the clean-erase path. p10k keeps
                        // the output and warns.
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

        while let Ok(res) = res_rx.try_recv() {
            if res.generation != git_gen {
                continue; // stale result
            }
            let cwd = current_info
                .as_ref()
                .map(|i| i.cwd.clone())
                .unwrap_or_default();
            last_vcs = Some((cwd.clone(), res.status.clone()));
            if at_prompt && let Some(info) = &current_info {
                let vcs = res.status.as_ref();
                theme::redraw_header_cfg(&mut stdout, last_size.1 as usize, &config, info, vcs)?;
                stdout.flush()?;
            }
            // Input line not ready: last_vcs is already updated and the `p`/marker backfill
            // picks it up, so no extra paint.
        }

        // Deferred prompt repaint after a resize.
        if let Some(deadline) = resize_prompt_at
            && std::time::Instant::now() >= deadline
        {
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

    // Engine exit: no explicit kill. reader/writer/master are dropped as the function
    // returns, master fd closes → slave hangs up → the inner shell (if still alive) gets
    // SIGHUP and exits. Being orphaned, even a SIGKILLed engine has its master fd closed by
    // the kernel, so the inner shell still gets SIGHUP.
    Ok(())
}

/// The engine's temp working dir: rc files (per shell) + the announce/ack files.
struct StateDir {
    dir: PathBuf,
    /// Path of the protocol rc file (zsh points ZDOTDIR at it; bash uses --rcfile).
    rc: PathBuf,
    announce: PathBuf,
    ack: PathBuf,
}

impl StateDir {
    fn create(shell: Shell, placeholder: &str) -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("p11k-{}", process::id()));
        fs::create_dir_all(&dir)?;

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
                fs::write(&rc, content.replace("__", placeholder))?;
                rc
            }
            Shell::Bash => {
                let rc = dir.join(".bashrc");
                fs::write(&rc, BASHRC_TEMPLATE.replace("__", placeholder))?;
                rc
            }
            Shell::Fish => {
                // fish redirects via XDG_CONFIG_HOME and reads $XDG_CONFIG_HOME/fish/config.fish.
                let fish_dir = dir.join("fish");
                fs::create_dir_all(&fish_dir)?;
                let rc = fish_dir.join("config.fish");
                fs::write(&rc, FISH_TEMPLATE.replace("__", placeholder))?;
                rc
            }
            Shell::Pwsh => {
                // pwsh has no rcfile convention (-Command dot-sources it). The file name
                // deliberately omits `__`: the template's placeholder is injected by a literal
                // `__` replacement.
                let rc = dir.join("p11k.ps1");
                fs::write(&rc, PWSH_TEMPLATE.replace("__", placeholder))?;
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

/// Announce messages: drawing the prompt window and resize.
enum AnnMsg {
    /// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`: precmd announce, draws the header.
    Header(HeaderInfo),
    /// `p`: zle-line-init announce, paints the input-line prefix over the placeholder.
    Prompt,
    /// `r`: resize announce (zsh's TRAPWINCH, bash's trap WINCH, fish's `--on-signal
    /// WINCH`, pwsh compares the window size inside prompt); resize the pty + repaint.
    Resize,
    /// `v\t<keymap>`: zle-keymap-select announce, updates the editing mode and repaints the
    /// header.
    VimMode(String),
}

struct GitRequest {
    generation: u64,
    cwd: String,
}

struct GitResult {
    generation: u64,
    status: Option<GitStatus>,
}

/// Read new lines from the announce file and parse them into prompt messages.
/// Line format (first field is the message type, the rest are tab-separated):
/// `h\t<exit>\t<cwd>[\t<jobs>][\t<history>]`, `p`, `r`, `v\t<keymap>`.
/// Missing fields are tolerated: a failed `<exit>` parse is None, a missing `<cwd>` is an
/// empty string, missing or invalid `<jobs>`/`<history>` are 0. Only zsh and pwsh send
/// `<history>`; bash/fish have no such column.
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
                    exec_seconds: 0.0, // filled in by the main loop from last_enter
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

/// Engine diagnostics log, written to a fixed file to avoid polluting the pass-through
/// stream.
fn log(msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/p11k-engine.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// Terminal replies to program queries do not count as "the user typing on the input line".
///
/// PSReadLine periodically sends DSR (`\x1b[6n`) to query the cursor position and the
/// terminal's `\x1b[<row>;<col>R` reply arrives on stdin: if the engine fails to recognize
/// it, it takes the reply for a keypress and the empty-enter nudge on shrink never fires
/// again. The same goes for focus events `\x1b[I`/`\x1b[O` and device attribute replies
/// `\x1b[?...c`. Arrow keys (`\x1b[A`) and the like are real keypresses and must not count.
fn is_terminal_reply(buf: &[u8]) -> bool {
    let Some(rest) = buf.strip_prefix(b"\x1b[") else {
        return false;
    };
    let Some((&last, params)) = rest.split_last() else {
        return false;
    };
    matches!(last, b'R' | b'c' | b'I' | b'O')
        && params
            .iter()
            .all(|b| b.is_ascii_digit() || *b == b';' || *b == b'?')
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Compute the git status of the current directory with p11k-gitstatus (None when not a
/// repo).
///
/// `RepoCache` is reused within the process: repo handles are cached by gitdir and
/// `build_fields` reuses the staged-diff cache (while HEAD is unchanged) and libgit2's
/// internal caches, avoiding a full rescan on every prompt.
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

/// Read an integer property on the `vcs` segment (p10k's global `POWERLEVEL9K_VCS_*`
/// parameters land on the vcs segment in KDL); use `default` when missing or invalid.
fn vcs_int_prop(config: &Config, key: &str, default: i64) -> i64 {
    match config.segment("vcs").prop(key) {
        Some(crate::config::Prop::Int(n)) => *n,
        _ => default,
    }
}

/// Fields are decimal strings after SafePrint; parse them as usize, treating failure as 0.
fn parse_field(b: &[u8]) -> usize {
    std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Raw mode for the real terminal: save the original termios on entry and restore it on
/// Drop. Same job as tmux/screen and other terminal multiplexers: pass every byte through
/// and let the shell in the pty handle signals and echo on its own terminal.
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

/// Real terminal size (TIOCGWINSZ on stdout), including pixel dimensions.
fn tty_size() -> Option<(u16, u16, u16, u16)> {
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

    #[test]
    fn shell_names_match_the_cli_flag() {
        assert_eq!(Shell::Zsh.name(), "zsh");
        assert_eq!(Shell::Bash.name(), "bash");
        assert_eq!(Shell::Fish.name(), "fish");
        assert_eq!(Shell::Pwsh.name(), "pwsh");
    }

    /// pwsh has no zle: the engine byte-matches the placeholder in the pass-through stream,
    /// so the protocol layer must do all three of announce `h`, wait for the ack and return
    /// the placeholder (resize only appends an `r` and must not return early — then nothing
    /// backfills the placeholder and it stays on screen).
    #[test]
    fn pwsh_template_implements_the_placeholder_protocol() {
        let t = PWSH_TEMPLATE;
        assert!(
            t.contains("function global:prompt"),
            "must define the prompt function"
        );
        assert!(
            t.contains("P11kEnginePrompt"),
            "prompt must forward to the engine implementation"
        );
        assert!(t.contains(r#""h`t$P11kCode`t$($PWD.Path)`t$P11kJobs`t$P11kHist`n""#));
        assert!(t.contains(r#""r`n""#), "resize must announce r");
        assert!(
            t.contains("while (-not [IO.File]::Exists($script:P11kAck))"),
            "must wait for the engine ack, otherwise header refill and placeholder output get out of order"
        );
        assert!(
            t.contains("[IO.File]::Delete($script:P11kAck)"),
            "must clear the ack"
        );
        // The resize branch must not return: after a return there is no h announce and the
        // engine does not backfill. Code lines only — words like "returning" in a comment
        // must not fail the assertion.
        let resize_block = t
            .split("$P11kSize.Width -ne")
            .nth(1)
            .expect("template should have the resize check");
        let resize_code = resize_block
            .split("$P11kJobs")
            .next()
            .unwrap()
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !resize_code.contains("return"),
            "the resize branch must not return early"
        );
        // A failed user profile load must not break the protocol invariants.
        assert!(
            t.contains("try { . $P11kUserProfile } catch"),
            "sourcing must catch exceptions"
        );
    }

    /// The placeholder is injected by the literal `replace("__", placeholder)`: as many `__`
    /// in the template, as many copies of the placeholder. One stray `__` in a comment adds
    /// an extra copy and breaks the prompt geometry. The numbers below are design values and
    /// must be updated with the templates. pwsh has 1; do not change the others to match.
    #[test]
    fn templates_carry_the_expected_number_of_placeholder_slots() {
        for (name, template, want) in [
            ("ZSHRC_TEMPLATE", ZSHRC_TEMPLATE, 2),
            ("BASHRC_TEMPLATE", BASHRC_TEMPLATE, 2),
            ("FISH_TEMPLATE", FISH_TEMPLATE, 4),
            ("PWSH_TEMPLATE", PWSH_TEMPLATE, 1),
        ] {
            assert_eq!(
                template.matches("__").count(),
                want,
                "{name}: every `__` becomes the placeholder, so this count is part of the \
                 protocol — and a comment must never contain `__`"
            );
        }
    }

    /// DSR/DA/focus are terminal replies, not user typing — misjudging them disables pwsh's
    /// shrink nudge entirely (PSReadLine keeps querying the cursor position on a real
    /// terminal).
    #[test]
    fn terminal_replies_are_not_user_typing() {
        assert!(
            is_terminal_reply(b"\x1b[24;1R"),
            "DSR cursor position reply"
        );
        assert!(is_terminal_reply(b"\x1b[1;1R"));
        assert!(is_terminal_reply(b"\x1b[?1;2c"), "device attributes reply");
        assert!(is_terminal_reply(b"\x1b[I"), "focus in");
        assert!(is_terminal_reply(b"\x1b[O"), "focus out");
        assert!(!is_terminal_reply(b"\x1b[A"), "arrow keys are user typing");
        assert!(!is_terminal_reply(b"\x1b[H"));
        assert!(!is_terminal_reply(b"\x1b"), "bare Esc is user typing");
        assert!(
            !is_terminal_reply(b"\x1b[200~"),
            "bracketed paste start marker"
        );
        assert!(!is_terminal_reply(b"a"));
        assert!(!is_terminal_reply(b"\r"));
        assert!(!is_terminal_reply(b""));
    }
}
