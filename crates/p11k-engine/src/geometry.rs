//! The prompt geometry the engine and the inner shell agree on at startup.
//!
//! Two things have to reach the shell before it starts and cannot change afterwards: how many
//! blank rows the header occupies, and the input-line prefix strings. Both travel as environment
//! variables of the inner shell, so a reloaded theme that would change either of them cannot be
//! applied to the running session.

use crate::cli::Shell;
use crate::config::Config;
use crate::i18n::t;
use crate::render::{input_prefix, prefix_text_bash, prefix_text_zsh};
use crate::theme::HeaderInfo;

/// What the shell was handed at startup, and what a reload has to stay compatible with.
pub struct Geometry {
    /// Blank rows the shell prints before the signal; the header occupies the same rows.
    pub header_rows: usize,
    /// `P11K_PROMPT_LEAD`: one newline per header row.
    pub prompt_lead: String,
    /// `P11K_PREFIX_OK`, in the syntax the shell's prompt string needs.
    pub prefix_ok: String,
    /// `P11K_PREFIX_ERR`, likewise.
    pub prefix_err: String,
}

/// A reloaded theme that cannot be applied without restarting the shell.
pub enum ReloadConflict {
    /// The new theme needs a different number of header rows.
    HeaderRows { was: usize, now: usize },
    /// The new theme renders a different input-line prefix.
    Prefix,
}

impl ReloadConflict {
    /// The message a user sees when a reload is refused.
    pub fn message(&self) -> String {
        match self {
            ReloadConflict::HeaderRows { was, now } => format!(
                "{}{was} -> {now}",
                t("the new theme needs a different number of header rows, which the running shell cannot use: restart p11k instead: ")
            ),
            ReloadConflict::Prefix => t(
                "the new theme changes the input-line prefix, which the running shell cannot pick up: restart p11k instead",
            )
            .to_string(),
        }
    }
}

impl Geometry {
    /// Derive the geometry a theme implies for a shell.
    pub fn new(config: &Config, shell: Shell) -> Self {
        let header_rows = config.layout.left.len().max(config.layout.right.len());
        let err_info = HeaderInfo {
            exit_code: Some(1),
            cwd: String::new(),
            exec_seconds: 0.0,
            jobs: 0,
            history: 0,
            pipestatus: Vec::new(),
        };
        let (prefix_ok, prefix_err) = match shell {
            Shell::Zsh => (
                prefix_text_zsh(config, None),
                prefix_text_zsh(config, Some("ERROR")),
            ),
            Shell::Bash => (
                prefix_text_bash(config, None),
                prefix_text_bash(config, Some("ERROR")),
            ),
            Shell::Fish | Shell::Pwsh => (
                input_prefix(config, None).text,
                input_prefix(config, Some(&err_info)).text,
            ),
        };
        Geometry {
            header_rows,
            prompt_lead: "\n".repeat(header_rows),
            prefix_ok,
            prefix_err,
        }
    }

    /// Why `new` cannot replace this geometry in the running session, if it cannot.
    pub fn reload_conflict(&self, new: &Geometry) -> Option<ReloadConflict> {
        if new.header_rows != self.header_rows {
            return Some(ReloadConflict::HeaderRows {
                was: self.header_rows,
                now: new.header_rows,
            });
        }
        if new.prefix_ok != self.prefix_ok || new.prefix_err != self.prefix_err {
            return Some(ReloadConflict::Prefix);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(layout: &str) -> Config {
        Config::parse(layout).expect("test config parses")
    }

    #[test]
    fn header_rows_track_the_tallest_column() {
        let two = Geometry::new(
            &config("layout {\n  left {\n    line { dir }\n    line { time }\n  }\n}"),
            Shell::Zsh,
        );
        assert_eq!(two.header_rows, 2);
        assert_eq!(two.prompt_lead, "\n\n");
    }

    #[test]
    fn a_different_row_count_is_a_reload_conflict() {
        let one = Geometry::new(&config("layout { left { line { dir } } }"), Shell::Zsh);
        let two = Geometry::new(
            &config("layout {\n  left {\n    line { dir }\n    line { time }\n  }\n}"),
            Shell::Zsh,
        );
        match one.reload_conflict(&two) {
            Some(ReloadConflict::HeaderRows { was, now }) => {
                assert_eq!((was, now), (1, 2));
            }
            _ => panic!("a row count change must be reported"),
        }
    }

    #[test]
    fn the_same_theme_has_no_conflict() {
        let cfg = config("layout { left { line { dir } } }");
        let a = Geometry::new(&cfg, Shell::Zsh);
        let b = Geometry::new(&cfg, Shell::Zsh);
        assert!(a.reload_conflict(&b).is_none());
    }

    #[test]
    fn a_different_prompt_char_is_a_reload_conflict() {
        let a = Geometry::new(
            &config("layout { left { line { dir } } }\nsegments { prompt_char char=\"❯\" }"),
            Shell::Zsh,
        );
        let b = Geometry::new(
            &config("layout { left { line { dir } } }\nsegments { prompt_char char=\"λ\" }"),
            Shell::Zsh,
        );
        assert!(matches!(
            a.reload_conflict(&b),
            Some(ReloadConflict::Prefix)
        ));
    }
}
