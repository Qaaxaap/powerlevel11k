//! The per-shell bootstrap rc files: the protocol layer each shell sources, plus the prompt
//! skeleton the engine backfills.

/// Generate the bootstrap .zshrc handed to the shell.
/// Sequence: precmd announces `h` → polls ack → zsh prints the prompt string (header blank
/// rows + zero-width signal + prefix) through zle → the engine backfills the header. The prompt
/// string is printed by zle or its equivalent; the theme is always the engine's job.
pub const ZSHRC_TEMPLATE: &str = include_str!("../templates/zsh.rc");

/// bash has no `p` announce: after the ack bash prints PS1 (the prompt lead + signal + prefix)
/// directly, and the engine reads the signal out of the stream to backfill the header. resize
/// detects size changes via trap WINCH and announces `r`.
pub const BASHRC_TEMPLATE: &str = include_str!("../templates/bash.rc");

/// fish protocol layer (injected via XDG_CONFIG_HOME redirection). fish_prompt + announce.
///
/// fish has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in `fish_prompt`, which prints
/// the prompt string after the ack; the engine reads the signal out of that string and backfills
/// the header. resize uses `--on-signal WINCH`.
pub const FISH_TEMPLATE: &str = include_str!("../templates/fish.rc");

/// pwsh protocol layer (injected via `-NoProfile -NoExit -Command ". <rc>"`).
///
/// pwsh likewise has no precmd/zle/PROMPT_COMMAND: the `h` announce lives in the prompt
/// function, which prints the prompt string only after the engine ack; the engine reads the
/// signal out of that string and backfills the header. resize compares the window size inside
/// prompt and then announces `r`: pwsh does not turn SIGWINCH into a hookable event.
///
/// The body is a standalone function and prompt only forwards: after a user profile
/// overrides prompt, the tail points it back at the engine implementation.
pub const PWSH_TEMPLATE: &str = include_str!("../templates/pwsh.ps1");

#[cfg(test)]
mod tests {
    use super::*;

    /// Each shell sends its own command duration. Fish sends CMD_DURATION. Zsh and bash send
    /// EPOCHREALTIME around preexec and prompt. Pwsh sends history timestamps. The engine falls
    /// back to its Enter timing only when the field is absent.
    #[test]
    fn every_template_sends_a_shell_measured_duration() {
        assert!(
            ZSHRC_TEMPLATE.contains("EPOCHREALTIME"),
            "zsh preexec/precmd timing"
        );
        assert!(
            BASHRC_TEMPLATE.contains("EPOCHREALTIME"),
            "bash DEBUG/prompt timing"
        );
        assert!(FISH_TEMPLATE.contains("CMD_DURATION"), "fish CMD_DURATION");
        assert!(
            PWSH_TEMPLATE.contains("StartExecutionTime"),
            "pwsh history timing"
        );
    }

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
        assert!(
            t.contains(r#""h`t$P11kCode`t$P11kPwdEsc`t$P11kJobs`t$P11kHist`t$P11kExecMs`n""#),
            "must announce the escaped cwd for the header"
        );
        assert!(
            t.contains("Replace('\\', '\\\\')"),
            "must escape a backslash in the cwd, otherwise a tab or a newline splits the announce line"
        );
        assert!(
            t.contains(".Replace(\"`t\", '\\t')") && t.contains(".Replace(\"`n\", '\\n')"),
            "must escape a tab and a newline in the cwd"
        );
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

    /// The prompt shape reaches the shell through the ack, so the row count and the prefix stay
    /// the engine's to decide. No template may carry a literal `__` again: a copy baked into a
    /// template would desync the geometry the moment the theme changes it.
    #[test]
    fn templates_take_the_prompt_shape_from_the_ack() {
        for (name, template) in [
            ("ZSHRC_TEMPLATE", ZSHRC_TEMPLATE),
            ("BASHRC_TEMPLATE", BASHRC_TEMPLATE),
            ("FISH_TEMPLATE", FISH_TEMPLATE),
            ("PWSH_TEMPLATE", PWSH_TEMPLATE),
        ] {
            assert!(
                template.contains("P11K_ACK"),
                "{name} must read the prompt shape from the ack"
            );
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

    /// Every prompt, including the first, prints its own header rows. The instant header and
    /// the input line below it stay above as history. No template steps the cursor back over
    /// them: that step raced with a shell that printed extra prompt rows during the instant
    /// phase, and the capture showed a stale header without a prompt line.
    #[test]
    fn the_first_prompt_keeps_the_instant_header_as_history() {
        assert!(
            !ZSHRC_TEMPLATE.contains(r#"print -rn -- $'\e['${_ack[1]:-0}'A'"#),
            "zsh must not step back over the instant header"
        );
    }

    /// The announce line and the ack are a contract with `protocol.rs`: the field order is
    /// fixed, and so are the ack line numbers. Pin both next to the templates.
    #[test]
    fn templates_share_the_announce_and_ack_contract() {
        let h = "printf 'h\\t%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n'";
        assert!(
            ZSHRC_TEMPLATE.contains("print -r -- \"h\"$'\\t'"),
            "zsh: announce field order"
        );
        for (name, template) in [
            ("BASHRC_TEMPLATE", BASHRC_TEMPLATE),
            ("FISH_TEMPLATE", FISH_TEMPLATE),
        ] {
            assert!(template.contains(h), "{name}: announce field order");
        }
        assert!(
            ZSHRC_TEMPLATE.contains("_ack[1]")
                && ZSHRC_TEMPLATE.contains("_ack[2]")
                && ZSHRC_TEMPLATE.contains("_ack[3]"),
            "zsh reads rows, prefix and fold from ack lines 1-3"
        );
        assert!(
            BASHRC_TEMPLATE.contains("_ack[0]") && BASHRC_TEMPLATE.contains("_ack[1]"),
            "bash reads rows and prefix from ack lines 0-1"
        );
        assert!(
            FISH_TEMPLATE.contains("$_ack[1]") && FISH_TEMPLATE.contains("$_ack[2]"),
            "fish reads rows and prefix from ack lines 1-2"
        );
        assert!(
            PWSH_TEMPLATE.contains("$P11kAckLines[0]")
                && PWSH_TEMPLATE.contains("$P11kAckLines[1]"),
            "pwsh reads rows and prefix from ack lines 0-1"
        );
    }

    /// The templates are shell code. Let each shell's own parser check them: a syntax error
    /// otherwise only shows up when a user starts that shell.
    #[test]
    fn templates_parse_with_their_shell() {
        let dir = std::env::temp_dir().join(format!("p11k-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (shell, name, source) in [
            ("zsh", "zsh.rc", ZSHRC_TEMPLATE),
            ("bash", "bash.rc", BASHRC_TEMPLATE),
            ("fish", "fish.rc", FISH_TEMPLATE),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, source).unwrap();
            let out = std::process::Command::new(shell)
                .arg("-n")
                .arg(&path)
                .output()
                .unwrap_or_else(|e| panic!("{shell} is needed for the template syntax check: {e}"));
            assert!(
                out.status.success(),
                "{shell} rejected {name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
