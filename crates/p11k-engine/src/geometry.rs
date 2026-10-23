//! The prompt geometry the engine and the inner shell agree on.
//!
//! The shell prints `header_rows` blank lines before the prompt-ready signal, and the prefix
//! string after it. Both live here so the startup path and the reload path derive them the same
//! way. The row count travels to the shell in the ack on every prompt, so a reload that changes
//! the header height takes effect on the next prompt; the prefix is handed over once at startup.

use crate::cli::Shell;
use crate::config::Config;
use crate::render::{input_prefix, prefix_text_bash, prefix_text_zsh};
use crate::theme::HeaderInfo;

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
            prefix_ok,
            prefix_err,
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
