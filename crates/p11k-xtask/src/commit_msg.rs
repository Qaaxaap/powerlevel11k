//! Check Conventional Commit subjects.
//!
//! The rules live here rather than in the workflow so the local hook and the CI job run the
//! same implementation. See the "Commit conventions" section of `CONTRIBUTING.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use xshell::{Shell, cmd};

use crate::workspace;

/// The commit types the repository accepts.
const TYPES: &[&str] = &[
    "feat", "fix", "chore", "docs", "test", "ci", "refactor", "perf", "build", "deps", "style",
    "revert", "i18n", "license", "bench",
];

/// Subject prefixes that carry no Conventional Commit and are left alone.
const SKIPPED: &[&str] = &["Merge ", "Revert ", "revert:", "fixup!", "squash!"];

/// The subject limit `CONTRIBUTING.md` states.
const SUBJECT_LIMIT: usize = 72;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A file holding one commit message, as the `commit-msg` hook receives it
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// A git range such as `origin/main..HEAD`; every non-merge commit is checked
    #[arg(long, value_name = "RANGE", conflicts_with = "file")]
    range: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    match (args.file, args.range) {
        (Some(file), None) => check_file(&file),
        (None, Some(range)) => check_range(&range),
        (None, None) => bail!("pass a message file or --range"),
        (Some(_), Some(_)) => unreachable!("clap rejects this combination"),
    }
}

/// Check one message file. The hook passes the path it received from git.
fn check_file(path: &Path) -> Result<()> {
    let message =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let Some(subject) = subject(&message) else {
        return Ok(());
    };
    if let Err(reason) = check(subject) {
        eprintln!("{reason}");
        process::exit(1);
    }
    Ok(())
}

/// Check every non-merge commit in `range`.
fn check_range(range: &str) -> Result<()> {
    let root = workspace::root();
    let sh = Shell::new()?;

    // A force push can remove the left side of the range. Check HEAD alone in that case rather
    // than failing on a revision that the clone no longer has.
    let left = range.split("..").next().unwrap_or_default();
    let shas = if left.is_empty() || is_null_sha(left) || !exists(&sh, &root, left)? {
        cmd!(sh, "git -C {root} rev-list --no-merges -n 1 HEAD").read()?
    } else {
        cmd!(sh, "git -C {root} rev-list --no-merges {range}").read()?
    };

    let mut failed = false;
    for sha in shas.split_whitespace() {
        let message = cmd!(sh, "git -C {root} log -1 --format=%B {sha}")
            .read()
            .with_context(|| format!("cannot read commit {sha}"))?;
        let Some(subject) = subject(&message) else {
            continue;
        };
        if let Err(reason) = check(subject) {
            eprintln!("{reason}");
            eprintln!("  commit {sha}");
            failed = true;
        }
    }
    if failed {
        process::exit(1);
    }
    Ok(())
}

/// Whether `rev` resolves to a commit in the repository.
fn exists(sh: &Shell, root: &Path, rev: &str) -> Result<bool> {
    let spec = format!("{rev}^{{commit}}");
    let output = sh
        .cmd("git")
        .arg("-C")
        .arg(root)
        .arg("cat-file")
        .arg("-e")
        .arg(&spec)
        .ignore_status()
        .output()?;
    Ok(output.status.success())
}

/// Whether `rev` is the all-zero placeholder of a new branch.
fn is_null_sha(rev: &str) -> bool {
    !rev.is_empty() && rev.chars().all(|c| c == '0')
}

/// The subject: the first line that is neither blank nor a comment.
fn subject(message: &str) -> Option<&str> {
    message
        .lines()
        .find(|line| !line.trim().is_empty() && !line.starts_with('#'))
}

/// Check one subject and describe the first problem.
fn check(subject: &str) -> Result<(), String> {
    if SKIPPED.iter().any(|prefix| subject.starts_with(prefix)) {
        return Ok(());
    }
    let Some((head, description)) = subject.split_once(": ") else {
        return Err(format!(
            "subject is not a Conventional Commit (expected `type(scope): description`): {subject}"
        ));
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) => (kind, Some(scope)),
            None => return Err(format!("unclosed scope parenthesis: {subject}")),
        },
        None => (head, None),
    };
    if !TYPES.contains(&kind) {
        return Err(format!(
            "unknown type `{kind}` in: {subject}\n  accepted: {}",
            TYPES.join(", ")
        ));
    }
    if let Some(scope) = scope {
        check_scope(scope).map_err(|reason| format!("{reason} in: {subject}"))?;
    }
    if description.trim().is_empty() {
        return Err(format!("empty description: {subject}"));
    }
    let length = subject.chars().count();
    if length > SUBJECT_LIMIT {
        return Err(format!(
            "subject is {length} characters, the limit is {SUBJECT_LIMIT}: {subject}"
        ));
    }
    Ok(())
}

/// A scope names a crate or a module below it.
fn check_scope(scope: &str) -> Result<(), String> {
    if scope.is_empty() {
        return Err("empty scope".to_owned());
    }
    let allowed =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
    if !scope.chars().all(allowed) {
        return Err(format!(
            "scope `{scope}` may only use lowercase letters, digits, dot, underscore and hyphen"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(subject: &str) -> Option<String> {
        check(subject).err()
    }

    #[test]
    fn accepts_the_conventional_subjects() {
        for subject in [
            "feat(engine): add a wizard",
            "fix: keep the prompt when the pty closes",
            "deps: vendor libgit2",
            "ci: split lint from test",
            "docs(glossary): define head and tail",
            "refactor(engine)!: rename the state module",
        ] {
            assert_eq!(violation(subject), None, "{subject}");
        }
    }

    #[test]
    fn skips_merge_revert_and_fixup() {
        for subject in [
            "Merge branch 'main' into ci/xtask",
            "Revert \"feat: x\"",
            "revert: feat: x",
            "fixup! feat: x",
            "squash! feat: x",
        ] {
            assert_eq!(violation(subject), None, "{subject}");
        }
    }

    #[test]
    fn rejects_broken_subjects() {
        let cases = [
            ("feat add a wizard", "not a Conventional Commit"),
            ("frobnicate: x", "unknown type"),
            ("fix(): x", "empty scope"),
            ("fix(P11K): x", "lowercase letters"),
            ("fix(engine: x", "unclosed scope"),
            ("feat:  ", "empty description"),
        ];
        for (subject, expected) in cases {
            let reason = violation(subject).unwrap_or_else(|| panic!("{subject} should fail"));
            assert!(reason.contains(expected), "{subject} -> {reason}");
        }
    }

    #[test]
    fn rejects_a_long_subject() {
        let subject = format!("fix: {}", "x".repeat(SUBJECT_LIMIT));
        let reason = violation(&subject).unwrap();
        assert!(reason.contains("the limit is 72"), "{reason}");
    }

    #[test]
    fn counts_characters_not_bytes() {
        let subject = format!("fix: {}", "提".repeat(SUBJECT_LIMIT - 5));
        assert_eq!(violation(&subject), None);
    }

    #[test]
    fn subject_skips_blanks_and_comments() {
        assert_eq!(subject("\n# a comment\n\nfix: x\n"), Some("fix: x"));
        assert_eq!(subject("\n# only a comment\n"), None);
    }

    #[test]
    fn detects_null_sha() {
        assert!(is_null_sha("0000000000000000000000000000000000000000"));
        assert!(!is_null_sha(""));
        assert!(!is_null_sha("abc123"));
    }
}
