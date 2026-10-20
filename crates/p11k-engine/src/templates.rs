//! The per-shell bootstrap rc files: the protocol layer each shell sources, plus the prompt
//! skeleton the engine backfills.

/// Generate the bootstrap .zshrc handed to the shell.
/// Sequence: precmd announces `h` → polls ack → zsh prints the prompt string (header blank
/// rows + zero-width signal + prefix) through zle → the engine backfills the header. The prompt
/// string is printed by zle or its equivalent; the theme is always the engine's job.
pub const ZSHRC_TEMPLATE: &str = r#"# p11k engine bootstrap -- protocol layer + user config.
# ===== engine protocol =====
# Empty when the engine does not enable transient; clear it so a stale value
# left in the outer environment cannot make zle-line-finish fold wrongly.
[[ -n "${P11K_TRANSIENT_PROMPT:-}" ]] || unset P11K_TRANSIENT_PROMPT
# Zero-width prompt-ready signal: `%{...%}` is how zle is told a sequence takes no columns.
# Without it the OSC bytes are counted as visible width and the buffer lands in the wrong column.
_p11k_signal=$'%{\e]1337;p11k;p\a%}'
_p11k_status=0
_p11k_pwd=$PWD

_p11k_precmd() {
  _p11k_status=$?
  # Capture $pipestatus right away: any command below resets it.
  _p11k_pipes="${pipestatus}"
  _p11k_pwd=$PWD
  # The prompt is the blank rows + prompt-ready signal + the prefix for this exit code. The
  # prefix belongs in PROMPT: zle draws the user's buffer right after it, and the engine
  # painting it later would overwrite the first characters on a completion redraw.
  #
  # The first prompt prints no blank rows: the engine already drew the instant header, and it
  # overwrites it in place when the signal arrives. Printing rows here would push that header
  # up and leave it on screen.
  local _lead="$P11K_PROMPT_LEAD"
  if [[ -n "${P11K_INSTANT:-}" ]]; then
    _lead=''
    unset P11K_INSTANT
  fi
  if (( _p11k_status == 0 )); then
    PROMPT="$_lead$_p11k_signal$P11K_PREFIX_OK"
  else
    PROMPT="$_lead$_p11k_signal$P11K_PREFIX_ERR"
  fi
  print -r -- "h"$'\t'"$_p11k_status"$'\t'"$_p11k_pwd"$'\t'"${#jobstates}"$'\t'"$HISTCMD"$'\t'"$_p11k_pipes" >> "$P11K_ANNOUNCE"
  until [[ -f "$P11K_ACK" ]]; do sleep 0.005; done
  rm -f "$P11K_ACK"
}

# zle calls this when it takes over the input line. The prompt is already on screen by then
# (the signal drew the header, the prompt string carries the prefix), so the hook only reports
# the keymap. It writes the file; the engine polls (~5ms).
_p11k_line_init() {
  [[ ${CONTEXT:-start} == cont ]] && return
  if [[ -n "${_p11k_user_line_init:-}" ]] && (( $+functions[$_p11k_user_line_init] )); then
    "$_p11k_user_line_init"   # user-registered handler (e.g. autosuggestions) runs first
  fi
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
# PROMPT carries the zero-width prompt-ready signal (OSC 1337;p11k;p) and the input-line prefix.
# The signal is what a serialised prompt protocol keys on: it occupies no columns and the
# engine reads it out of the byte stream.
PROMPT="$P11K_PROMPT_LEAD$_p11k_signal$P11K_PREFIX_OK"
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

/// bash has no `p` announce: after the ack bash prints PS1 (the prompt lead + signal + prefix)
/// directly, and the engine reads the signal out of the stream to backfill the header. resize
/// detects size changes via trap WINCH and announces `r`.
pub const BASHRC_TEMPLATE: &str = r#"# p11k engine bootstrap (bash) -- protocol layer + user config.
# Zero-width prompt-ready signal. `\[...\]` tells readline the sequence takes no columns; a bare
# OSC would be counted as visible width and the buffer would start in the wrong column.
_p11k_signal=$'\e]1337;p11k;p\a'
PS1="$P11K_PROMPT_LEAD\[$_p11k_signal\]"

# PROMPT_COMMAND runs before PS1 is displayed: record the exit code, announce `h`,
# then wait for the engine ack before returning.
_p11k_prompt_command() {
  local _st=$?
  # Capture $PIPESTATUS right away: any command below resets it.
  local _pipes="${PIPESTATUS[*]}"
  # The prefix belongs in PS1: readline draws the buffer right after it, and painting it later
  # would overwrite the first characters on a completion redraw.
  #
  # The first prompt prints no blank rows: the engine already drew the instant header and
  # overwrites it in place when the signal arrives. Printing rows here would push that header up
  # and leave it on screen.
  local _lead="$P11K_PROMPT_LEAD"
  if [[ -n "${P11K_INSTANT:-}" ]]; then
    _lead=''
    unset P11K_INSTANT
  fi
  # `\r` puts the prompt at column 0: the engine draws the instant prefix on the input line
  # before the shell starts, and without this readline would append the prompt after it.
  if [[ $_st -eq 0 ]]; then
    PS1='\r'"$_lead\[$_p11k_signal\]$P11K_PREFIX_OK"
  else
    PS1='\r'"$_lead\[$_p11k_signal\]$P11K_PREFIX_ERR"
  fi
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
PS1="$P11K_PROMPT_LEAD\[$_p11k_signal\]"
PROMPT_COMMAND=_p11k_prompt_command
trap '_p11k_winch' WINCH
"#;

/// fish protocol layer (injected via XDG_CONFIG_HOME redirection). fish_prompt + announce.
///
/// fish has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in `fish_prompt`, which prints
/// the prompt string after the ack; the engine reads the signal out of that string and backfills
/// the header. resize uses `--on-signal WINCH`.
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
    # The first prompt prints no blank rows: the engine already drew the instant header and
    # overwrites it in place when the signal arrives.
    set -l _lead "$P11K_PROMPT_LEAD"
    if test -n "$P11K_INSTANT"
        set _lead ''
        set -e P11K_INSTANT
    end
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        if test $_st -eq 0
            printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_OK"
        else
            printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_ERR"
        end
        return
    end
    printf 'h\t%s\t%s\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) "" "$_pipes" >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    if test $_st -eq 0
        printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_OK"
    else
        printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_ERR"
    end
end

# ===== user config =====
if test -n "$P11K_USER_ZSHRC"; and test -r "$P11K_USER_ZSHRC"
    source "$P11K_USER_ZSHRC"
else if test -r "$HOME/.config/fish/config.fish"
    source "$HOME/.config/fish/config.fish"
end

# ===== protocol invariants =====
# The greeting prints a line between the instant header and the first prompt, which pushes the
# header up and leaves a copy of it on screen. p11k owns the prompt area, so it is disabled.
set -g fish_greeting ''
set -g _p11k_last_cols $COLUMNS
set -g _p11k_last_rows $LINES
function fish_prompt
    set -l _st $status
    # Capture $pipestatus right away: any command below resets it.
    set -l _pipes "$pipestatus"
    # The first prompt prints no blank rows: the engine already drew the instant header and
    # overwrites it in place when the signal arrives.
    set -l _lead "$P11K_PROMPT_LEAD"
    if test -n "$P11K_INSTANT"
        set _lead ''
        set -e P11K_INSTANT
    end
    if test $COLUMNS != $_p11k_last_cols; or test $LINES != $_p11k_last_rows
        set -g _p11k_last_cols $COLUMNS
        set -g _p11k_last_rows $LINES
        printf 'r\n' >> $P11K_ANNOUNCE
        if test $_st -eq 0
            printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_OK"
        else
            printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_ERR"
        end
        return
    end
    printf 'h\t%s\t%s\t%s\t%s\t%s\n' $_st $PWD (jobs -p | count) "" "$_pipes" >> $P11K_ANNOUNCE
    while not test -f $P11K_ACK
        sleep 0.005
    end
    rm -f $P11K_ACK
    if test $_st -eq 0
        printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_OK"
    else
        printf '\r%s\e]1337;p11k;p\a%s' "$_lead" "$P11K_PREFIX_ERR"
    end
end
"#;

/// pwsh protocol layer (injected via `-NoProfile -NoExit -Command ". <rc>"`).
///
/// pwsh likewise has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in the prompt
/// function, which prints the prompt string only after the engine ack; the engine reads the
/// signal out of that string and backfills the header. resize compares the window size inside
/// prompt and then announces `r`: pwsh does not turn SIGWINCH into a hookable event.
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
        # Announcing only `r` and returning would leave the old prompt on screen: the engine
        # may not be in prompt state right now (just pressed enter), so it would not repaint.
        # Hence fall through to the normal announce.
        [IO.File]::AppendAllText($script:P11kAnnounce, "r`n")
    }
    $P11kJobs = @(Get-Job -ErrorAction SilentlyContinue).Count
    # Send the command number, not the history count: zsh reports HISTCMD, and
    # HistoryInfo.Id is PowerShell's nearest equivalent (session-unique and
    # monotonic). .Count drops when MaximumHistoryCount trims; Id survives that.
    # After Clear-History there is nothing left to read, so it reports 0.
    $P11kHist = if ($h = Get-History -Count 1) { $h.Id } else { 0 }
    [IO.File]::AppendAllText($script:P11kAnnounce, "h`t$P11kCode`t$($PWD.Path)`t$P11kJobs`t$P11kHist`n")
    # Wait for the engine ack: the prompt string is printed first, the header is backfilled after.
    while (-not [IO.File]::Exists($script:P11kAck)) { Start-Sleep -Milliseconds 5 }
    [IO.File]::Delete($script:P11kAck)
    # Prompt is ready, clear the anti-recursion flag (same as zsh's zle-line-init): a manual
    # p11k started later in the session is then not treated as a recursive load.
    if ($null -ne $env:P11K_ENGINE) { $env:P11K_ENGINE = $null }
    # The prefix belongs in the prompt string: PSReadLine draws the buffer right after it.
    # The first prompt prints no blank rows: the engine already drew the instant header and
    # overwrites it in place when the signal arrives.
    $P11kLead = $env:P11K_PROMPT_LEAD
    if ($env:P11K_INSTANT) {
        $P11kLead = ''
        Remove-Item Env:P11K_INSTANT -ErrorAction SilentlyContinue
    }
    # `r` puts the prompt at column 0: the engine draws the instant prefix on the input line
    # before the shell starts, and without it PSReadLine would start the prompt after that.
    if ($P11kCode -eq 0) {
        "$P11kLead`e]1337;p11k;p`a$env:P11K_PREFIX_OK"
    } else {
        "$P11kLead`e]1337;p11k;p`a$env:P11K_PREFIX_ERR"
    }
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

    /// pwsh has no zle: the signal travels inside the prompt string, so the protocol layer must
    /// do all three of announce `h`, wait for the ack and emit the signal plus the prefix (resize
    /// only appends an `r` and must not return early — then nothing backfills the header and the
    /// old prompt stays on screen).
    #[test]
    fn pwsh_template_implements_the_signal_protocol() {
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
            "must wait for the engine ack, otherwise the header backfill and the prompt string get out of order"
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

    /// The prompt skeleton reaches the shell through environment variables, so the row count and
    /// the prefix stay the engine's to decide. No template may carry a literal `__` again: a copy
    /// baked into a template would desync the geometry the moment the theme changes it.
    #[test]
    fn templates_take_the_prompt_skeleton_from_the_environment() {
        for (name, template) in [
            ("ZSHRC_TEMPLATE", ZSHRC_TEMPLATE),
            ("BASHRC_TEMPLATE", BASHRC_TEMPLATE),
            ("FISH_TEMPLATE", FISH_TEMPLATE),
            ("PWSH_TEMPLATE", PWSH_TEMPLATE),
        ] {
            for var in ["P11K_PROMPT_LEAD", "P11K_PREFIX_OK", "P11K_PREFIX_ERR"] {
                assert!(
                    template.contains(var),
                    "{name} must use {var}, which the engine passes in"
                );
            }
            assert!(
                template.contains("1337;p11k;p"),
                "{name} must emit the prompt-ready signal"
            );
            assert!(
                !template.contains("__"),
                "{name} must not carry a prompt skeleton of its own"
            );
        }
    }
}
