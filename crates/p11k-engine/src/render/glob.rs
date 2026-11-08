//! Pattern matching for `dir-classes` and `VCS_DISABLED_WORKDIR_PATTERN`.
//!
//! p10k matches with zsh extended globs. p11k uses a simplified glob: `~` expands to $HOME,
//! `*` / `?` / `[...]` do not cross `/`, `**` crosses directories, and everything else is
//! literal. A pattern ending in `/` also matches all subdirectories. A top-level `|` and every
//! `(...)` group expand to alternatives, mirroring the zsh syntax the themes use.

/// p10k's `DIR_CLASSES` matches `$PWD` with zsh extended globs; p11k uses a simplified glob:
/// `~` expands to $HOME, `*` / `?` / `[…]` do not cross `/`, `**` crosses directories, and
/// everything else is literal. A pattern ending in `/` (`~/work/`) also matches all subdirs.
pub(super) fn glob_match_path(pattern: &str, path: &str, home: &str) -> bool {
    let pat = match pattern.strip_prefix('~') {
        Some(rest) => format!("{home}{rest}"),
        None => pattern.to_string(),
    };
    if pat.ends_with('/') {
        return glob_match(&format!("{pat}**"), path);
    }
    glob_match(&pat, path)
}

/// Simplified glob: `**` crosses `/`, `*` / `?` / `[…]` do not.
pub(super) fn glob_match(pat: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => (0..=t.len()).any(|i| go(&p[2..], &t[i..])),
            Some('*') => (0..=t.len())
                .take_while(|i| *i == 0 || t[i - 1] != '/')
                .any(|i| go(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && t[0] != '/' && go(&p[1..], &t[1..]),
            Some('[') => {
                let Some(close) = p.iter().position(|c| *c == ']') else {
                    return !t.is_empty() && t[0] == '[' && go(&p[1..], &t[1..]);
                };
                if t.is_empty() || t[0] == '/' {
                    return false;
                }
                let set: Vec<char> = p[1..close].to_vec();
                let mut hit = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        if t[0] >= set[i] && t[0] <= set[i + 2] {
                            hit = true;
                        }
                        i += 3;
                    } else {
                        if set[i] == t[0] {
                            hit = true;
                        }
                        i += 1;
                    }
                }
                hit && go(&p[close + 1..], &t[1..])
            }
            Some(c) => !t.is_empty() && t[0] == *c && go(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// Whether a repo whose workdir is `workdir` must be ignored (p10k
/// `VCS_DISABLED_WORKDIR_PATTERN`). p10k tests `[[ $VCS_STATUS_WORKDIR == $~pattern ]]`, so
/// the pattern is a zsh pattern and an unset one disables nothing.
pub(crate) fn workdir_is_disabled(pattern: &str, workdir: &str) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    workdir_matches(pattern, workdir, &home)
}

/// `workdir_is_disabled` with an explicit home directory.
pub(super) fn workdir_matches(pattern: &str, workdir: &str, home: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    pattern_alternatives(pattern)
        .iter()
        .any(|p| glob_match_path(p, workdir, home))
}

/// Rewrites a zsh pattern as the equivalent set of globs, expanding top-level `|` and every
/// `(...)` group: `~(|/foo)|/bar/baz/*` becomes `~`, `~/foo` and `/bar/baz/*`.
fn pattern_alternatives(pat: &str) -> Vec<String> {
    split_top_level(pat, '|')
        .into_iter()
        .flat_map(|branch| expand_groups(&branch))
        .collect()
}

/// Splits on `delim` at the top level, ignoring `|` inside `(...)` and `[...]`.
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut bracket = false;
    for c in s.chars() {
        match c {
            '[' if !bracket => bracket = true,
            ']' if bracket => bracket = false,
            '(' if !bracket => depth += 1,
            ')' if !bracket && depth > 0 => depth -= 1,
            _ => {}
        }
        if c == delim && depth == 0 && !bracket {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Expands the first top-level `(...)` group, then any group that follows it. An unbalanced
/// `(` is left alone, like a literal.
fn expand_groups(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0usize;
    let mut bracket = false;
    let mut open = 0usize;
    for (i, c) in chars.iter().enumerate() {
        match *c {
            '[' if !bracket => bracket = true,
            ']' if bracket => bracket = false,
            '(' if !bracket => {
                if depth == 0 {
                    open = i;
                }
                depth += 1;
            }
            ')' if !bracket && depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let prefix: String = chars[..open].iter().collect();
                    let inner: String = chars[open + 1..i].iter().collect();
                    let suffix: String = chars[i + 1..].iter().collect();
                    return pattern_alternatives(&inner)
                        .into_iter()
                        .flat_map(|alt| expand_groups(&format!("{prefix}{alt}{suffix}")))
                        .collect();
                }
            }
            _ => {}
        }
    }
    vec![s.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_workdir_pattern_follows_p10k() {
        // p10k's VCS_DISABLED_WORKDIR_PATTERN: the pattern is a zsh pattern matched against
        // the repo's workdir, `~` is $HOME, and an unset pattern disables nothing.
        assert!(!workdir_matches("", "/home/u/repo", "/home/u"));
        assert!(workdir_matches("~", "/home/u", "/home/u"));
        // `~` alone is not a prefix match: $HOME/.git stays visible.
        assert!(!workdir_matches("~", "/home/u/.git", "/home/u"));
        assert!(!workdir_matches("~", "/tmp/repo", "/home/u"));
        // Alternation, and a group with an empty branch (p10k's own README example).
        assert_eq!(
            pattern_alternatives("~(|/foo)|/bar/baz/*"),
            ["~", "~/foo", "/bar/baz/*"]
        );
        assert!(workdir_matches("~(|/foo)", "/home/u/foo", "/home/u"));
        assert!(!workdir_matches("~(|/foo)", "/home/u/bar", "/home/u"));
        assert!(workdir_matches("/tmp|/var", "/var", "/home/u"));
        // A pattern ending in `/` covers the subdirectories; `|` inside `[...]` is literal.
        assert!(workdir_matches("~/work/", "/home/u/work/a/b", "/home/u"));
        assert_eq!(pattern_alternatives("[a|b]"), ["[a|b]"]);
        assert_eq!(pattern_alternatives("a(|b)c"), ["ac", "abc"]);
    }

}
