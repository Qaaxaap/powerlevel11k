//! The prompt geometry the engine and the inner shell agree on.
//!
//! The shell prints `header_rows` blank lines before the prompt-ready signal, and the prefix
//! string after it. Both live here so the startup path and the reload path derive them the same
//! way. The row count travels to the shell in the ack on every prompt, so a reload that changes
//! the header height takes effect on the next prompt; the prefix is handed over once at startup.

use crate::cli::Shell;
use crate::config::Config;
use crate::render::{prefix_text, prefix_text_bash, prefix_text_zsh};

/// What determines the shape of the prompt area.
pub struct Geometry {
    /// Blank rows the shell prints before the signal; the header occupies the same rows.
    pub header_rows: usize,
    /// `P11K_PREFIX_OK`, in the syntax the shell's prompt string needs.
    pub prefix_ok: String,
    /// `P11K_PREFIX_ERR`, likewise.
    pub prefix_err: String,
}

impl Geometry {
    /// Derive the geometry a theme implies for a shell.
    pub fn new(config: &Config, shell: Shell) -> Self {
        Geometry {
            header_rows: config.layout.left.len().max(config.layout.right.len()),
            prefix_ok: Self::prefix_for(config, shell, None),
            prefix_err: Self::prefix_for(config, shell, Some("ERROR")),
        }
    }

    /// The prefix for one prompt state, in the syntax that shell's prompt string needs.
    ///
    /// The state is the engine's to decide: it is the only side that sees the exit code, the
    /// pipeline statuses and the vi mode, and a shell that had to pick for itself could not tell
    /// `OK_PIPE` from `OK` or follow a mode change.
    pub fn prefix_for(config: &Config, shell: Shell, state: Option<&str>) -> String {
        match shell {
            Shell::Zsh => prefix_text_zsh(config, state),
            Shell::Bash => prefix_text_bash(config, state),
            Shell::Fish | Shell::Pwsh => prefix_text(config, state),
        }
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
    }

    #[test]
    fn a_single_line_layout_is_one_row() {
        let one = Geometry::new(&config("layout { left { line { dir } } }"), Shell::Zsh);
        assert_eq!(one.header_rows, 1);
    }

    #[test]
    fn the_prefix_follows_the_prompt_char() {
        let a = Geometry::new(
            &config("layout { left { line { dir } } }\nsegments { prompt_char char=\"❯\" }"),
            Shell::Zsh,
        );
        let b = Geometry::new(
            &config("layout { left { line { dir } } }\nsegments { prompt_char char=\"λ\" }"),
            Shell::Zsh,
        );
        assert_ne!(a.prefix_ok, b.prefix_ok);
    }
}
