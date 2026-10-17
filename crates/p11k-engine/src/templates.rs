//! The per-shell bootstrap rc files: the protocol layer each shell sources, plus the prompt
//! placeholder the engine backfills.

/// Generate the bootstrap .zshrc handed to the shell.
/// Sequence: precmd announces `h` → polls ack → zsh renders the placeholder PROMPT (zle)
/// → zle-line-init announces `p` → the engine returns to line start and paints the prefix
/// over the placeholder. The placeholder prompt is rendered by zle or its equivalent; the
/// theme is always the engine's job.
pub const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap -- protocol layer + user config.
# ===== engine protocol =====
# Empty when the engine does not enable transient; clear it so a stale value
# left in the outer environment cannot make zle-line-finish fold wrongly.
[[ -n "${P11K_TRANSIENT_PROMPT:-}" ]] || unset P11K_TRANSIENT_PROMPT
_p11k_status=0
_p11k_pwd=$PWD

_p11k_precmd() {
  _p11k_status=$?
  # Capture $pipestatus right away: any command below resets it.
  _p11k_pipes="${pipestatus}"
  _p11k_pwd=$PWD
  # Used to restore PROMPT after the transient fold; carries the prompt-ready signal too.
  PROMPT="$P11K_PLACEHOLDER"$'\e]1337;p11k;p\a'
  print -r -- "h"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd"$'\t'"${#jobstates}"$'\t'"$HISTCMD"$'\t'"$_p11k_pipes" >> "$P11K_ANNOUNCE"
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
# PROMPT carries the placeholder plus the zero-width prompt-ready signal (OSC 1337;p11k;p).
# The signal is what a serialised prompt protocol keys on: it occupies no columns and the
# engine reads it out of the byte stream.
PROMPT="$P11K_PLACEHOLDER"$'\e]1337;p11k;p\a'
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

/// bash has no `p` announce: after the ack bash prints PS1 (the `P11K_PLACEHOLDER` value)
/// directly, and the engine byte-matches it to paint the prefix over it. resize detects size
/// changes via trap WINCH and announces `r`.
pub const BASHRC_TEMPLATE: &str = r#"# p11k engine bootstrap (bash) -- protocol layer + user config.
PS1="$P11K_PLACEHOLDER"'\[\e]1337;p11k;p\a\]'

# PROMPT_COMMAND runs before PS1 is displayed: record the exit code, announce `h`,
# then wait for the engine ack before returning.
_p11k_prompt_command() {
  local _st=$?
  # Capture $PIPESTATUS right away: any command below resets it.
  local _pipes="${PIPESTATUS[*]}"
  printf 'h\t%s\t%s\t%s\t%s\t%s\n' "$_st" "$PWD" "$(jobs -p | wc -l)" "" "$_pipes" >> "$P11K_ANNOUNCE"
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
PS1="$P11K_PLACEHOLDER"'\[\e]1337;p11k;p\a\]'
PROMPT_COMMAND=_p11k_prompt_command
trap '_p11k_winch' WINCH
"#;

/// fish protocol layer (injected via XDG_CONFIG_HOME redirection). fish_prompt placeholder + announce.
///
/// fish has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in `fish_prompt`, which
/// returns the placeholder after the ack; the engine byte-matches the placeholder and
/// paints the prefix over it (same as bash, no `p` announce). resize uses `--on-signal WINCH`.
pub const FISH_TEMPLATE: &str = r#"# p11k engine bootstrap (fish) -- protocol layer + user config.

set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES

# fish_prompt is called when the main prompt is rendered. Two cases:
# - size change (resize repaint): announce `r`.
# - normal prompt: announce `h`, then wait for the engine ack before returning.
function fish_prompt
    set -l _st $status
    # Capture $pipestatus right away: any command below resets it.
    set -l _pipes "$pipestatus"
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        printf '%s\e]1337;p11k;p\a' "$P11K_PLACEHOLDER"
        return
    end
    printf 'h\t%s\t%s\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) "" "$_pipes" >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    printf '%s\e]1337;p11k;p\a' "$P11K_PLACEHOLDER"
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
    # Capture $pipestatus right away: any command below resets it.
    set -l _pipes "$pipestatus"
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        printf '%s\e]1337;p11k;p\a' "$P11K_PLACEHOLDER"
        return
    end
    printf 'h\t%s\t%s\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) "" "$_pipes" >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    printf '%s\e]1337;p11k;p\a' "$P11K_PLACEHOLDER"
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
pub const PWSH_TEMPLATE: &str = r#"# p11k engine bootstrap (pwsh) -- protocol layer + user config.
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
    "$env:P11K_PLACEHOLDER`e]1337;p11k;p`a"
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The placeholder reaches the shell through `P11K_PLACEHOLDER`, so its width and its row
    /// count stay the engine's to decide. No template may carry a literal `__` again: a copy
    /// baked into a template would desync the geometry the moment the theme changes it.
    #[test]
    fn templates_take_the_placeholder_from_the_environment() {
        for (name, template) in [
            ("ZSHRC_TEMPLATE", ZSHRC_TEMPLATE),
            ("BASHRC_TEMPLATE", BASHRC_TEMPLATE),
            ("FISH_TEMPLATE", FISH_TEMPLATE),
            ("PWSH_TEMPLATE", PWSH_TEMPLATE),
        ] {
            assert!(
                template.contains("P11K_PLACEHOLDER"),
                "{name} must print the placeholder the engine passes in"
            );
            assert!(
                !template.contains("__"),
                "{name} must not carry a placeholder of its own"
            );
        }
    }
}
