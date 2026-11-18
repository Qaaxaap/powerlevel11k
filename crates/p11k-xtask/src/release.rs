//! Check a release tag against the manifest and the changelog.
//!
//! The checks used to be inline shell in the release workflow. They live here so `just
//! release-check v1.2.3` runs the same code before the tag is pushed.

use std::fs;

use anyhow::{Context, Result, bail};

use crate::workspace;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The tag, with or without the leading `v`
    #[arg(value_name = "TAG")]
    tag: String,
}

pub fn run(args: Args) -> Result<()> {
    let version = workspace::version()?;
    let tag = args.tag.strip_prefix('v').unwrap_or(&args.tag);
    if tag != version {
        bail!(
            "tag {} does not match the workspace version {version}",
            args.tag
        );
    }

    let path = workspace::root().join("CHANGELOG.md");
    let changelog =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    if !has_section(&changelog, &version) {
        bail!(
            "CHANGELOG.md has no `## {version}` section; run `just changelog v{version}` and commit the result"
        );
    }
    println!("release {version} matches Cargo.toml and CHANGELOG.md");
    Ok(())
}

/// Whether the changelog has a version section for `version`.
///
/// git-cliff writes `## <version> — <date>`, and keeps an `## Unreleased` heading while the
/// version is unreleased.
fn has_section(changelog: &str, version: &str) -> bool {
    changelog.lines().any(|line| {
        line.strip_prefix("## ").is_some_and(|rest| {
            rest == version
                || rest
                    .strip_prefix(version)
                    .is_some_and(|tail| tail.starts_with([' ', '\u{2014}', '\u{2013}']))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str =
        "# Changelog\n\n## Unreleased\n\n- something\n\n## 1.0.0-alpha.1 — 2026-11-16\n\n- first\n";

    #[test]
    fn finds_the_version_section() {
        assert!(has_section(CHANGELOG, "1.0.0-alpha.1"));
    }

    #[test]
    fn does_not_confuse_a_prefix() {
        assert!(!has_section(CHANGELOG, "1.0.0"));
        assert!(!has_section(CHANGELOG, "1.0.0-alpha"));
        assert!(!has_section(CHANGELOG, "2.0.0"));
    }
}
