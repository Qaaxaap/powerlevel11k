//! `dir` segment truncation strategies (the full p10k `POWERLEVEL9K_SHORTEN_STRATEGY` set).
//!
//! Supported strategies:
//! - `truncate_to_unique` (default): walking back from the current directory, shorten every
//!   non-anchor component to "its shortest unique prefix among directory siblings"; no ellipsis
//!   is left after shortening (aligns with p10k's default `SHORTEN_DELIMITER=`). Anchors
//!   (home `~`/root, the trailing `length` levels, ancestors containing a marker file) are not shortened.
//! - `truncate_middle` / `truncate_from_right`: keep the first `length` characters per level
//!   (middle also keeps the last `length`), replacing the middle/tail with the ellipsis.
//! - `truncate_to_last`: keep only the last `length` levels.
//! - `truncate_to_first_and_last`: first `length` levels + last `length` levels, eliding the middle.
//! - `truncate_absolute(_chars)`: hard-truncate the whole path by character count (keeping the tail).
//! - `truncate_with_folder_marker`: fold at marker files.
//!
//! Returns parts tagged by class; render maps those to states for coloring.
//!
//! # Performance (aligns with p10k's mtime cache)
//!
//! `truncate_to_unique` caches each level's shortened result under
//! `(absolute directory, parent mtime_ns)`: an unchanged directory is reused directly,
//! with no `readdir`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

thread_local! {
    static CACHE: RefCell<HashMap<(PathBuf, i64), String>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// Anchor (leading `~`/root/current directory/marker ancestor), not shortened.
    Anchor,
    Shortened,
    /// Normal (not shortened, but not an anchor).
    Normal,
}

/// One folded part.
#[derive(Clone)]
pub struct DirPart {
    pub text: String,
    pub class: Class,
}

/// Truncation strategy (p10k `POWERLEVEL9K_SHORTEN_STRATEGY`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Strategy {
    /// Shorten each level to its shortest unique prefix among siblings. The p11k/p10k default.
    #[default]
    TruncateToUnique,
    /// Keep the first N + ellipsis + last N per level.
    TruncateMiddle,
    /// Keep the first N + ellipsis per level.
    TruncateFromRight,
    /// Keep only the last N levels.
    TruncateToLast,
    /// First N levels + last N levels, eliding the middle.
    TruncateToFirstAndLast,
    /// Hard-truncate the whole path by character count (keeping the tail).
    TruncateAbsolute,
    /// Fold at marker files.
    TruncateWithFolderMarker,
    /// Empty/unknown strategy (p10k's default branch): keep only the last N levels, prefixed by the ellipsis.
    FoldToLast,
}

impl Strategy {
    /// p10k strategy name → strategy; empty/unknown takes the default branch (`FoldToLast`).
    pub fn parse(s: &str) -> Strategy {
        match s {
            "truncate_to_unique" => Strategy::TruncateToUnique,
            "truncate_middle" => Strategy::TruncateMiddle,
            "truncate_from_right" => Strategy::TruncateFromRight,
            "truncate_to_last" => Strategy::TruncateToLast,
            "truncate_to_first_and_last" => Strategy::TruncateToFirstAndLast,
            "truncate_absolute" | "truncate_absolute_chars" => Strategy::TruncateAbsolute,
            "truncate_with_folder_marker" => Strategy::TruncateWithFolderMarker,
            _ => Strategy::FoldToLast,
        }
    }
}

/// Folding options (the p10k `SHORTEN_*` / `DIR_*` parameters).
pub struct Opts {
    pub strategy: Strategy,
    /// `SHORTEN_DIR_LENGTH` (levels kept / characters per level).
    pub length: usize,
    /// `SHORTEN_DELIMITER`; empty = leave no ellipsis.
    pub delimiter: String,
    /// `SHORTEN_FOLDER_MARKER`; empty = use the built-in marker list.
    pub marker: String,
    /// Folding budget for `truncate_to_unique`: `None` = no folding,
    /// `Some(n)` = this line is too wide and n columns must be saved.
    /// Other strategies ignore this field, being width-independent like p10k.
    pub budget: Option<usize>,
}

/// Built-in marker files (p10k's default `markers` list).
const MARKERS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "Cargo.toml",
    "package.json",
    "go.mod",
];

/// Fold the absolute path `cwd` and return the part sequence (without `~`/`/`; the caller assembles it and adds prefixes).
pub fn shorten(cwd: &Path, home: Option<&Path>, opts: &Opts) -> Vec<DirPart> {
    let len = opts.length.max(1);
    let (parts, base) = split_parts(cwd, home);
    if parts.is_empty() {
        return Vec::new();
    }
    match opts.strategy {
        Strategy::TruncateToUnique => fold_unique(&parts, len, &base, opts),
        Strategy::TruncateMiddle => fold_truncate(&parts, len, &opts.delimiter, true),
        Strategy::TruncateFromRight => fold_truncate(&parts, len, &opts.delimiter, false),
        Strategy::TruncateToLast => fold_to_last(&parts, len, &opts.delimiter),
        Strategy::TruncateToFirstAndLast => fold_first_last(&parts, len, &opts.delimiter),
        Strategy::TruncateAbsolute => fold_absolute(&parts, len, &opts.delimiter),
        Strategy::TruncateWithFolderMarker => {
            fold_folder_marker(&parts, &base, &opts.delimiter, opts)
        }
        Strategy::FoldToLast => fold_keep_last(&parts, len, &opts.delimiter),
    }
}

/// Split into a part sequence + base directory (home when available, otherwise root).
fn split_parts(cwd: &Path, home: Option<&Path>) -> (Vec<String>, PathBuf) {
    if let Some(home) = home
        && let Ok(rel) = cwd.strip_prefix(home)
    {
        let parts = to_parts(rel);
        return (parts, home.to_path_buf());
    }
    (to_parts(cwd), PathBuf::from("/"))
}

fn to_parts(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

/// `truncate_to_unique`: anchors are kept, the rest are shortened to unique prefixes.
///
/// With `opts.budget` as `None` the whole path is left as is; with `Some(n)` levels are folded
/// front to back until the accumulated saved columns reach n.
fn fold_unique(parts: &[String], shortenlen: usize, base: &Path, opts: &Opts) -> Vec<DirPart> {
    let n = parts.len();
    let anchor_tail = shortenlen.min(n);
    let delim_len = opts.delimiter.chars().count();
    let mut saved = 0usize;
    let mut out: Vec<DirPart> = Vec::with_capacity(n);
    for (i, part) in parts.iter().enumerate() {
        let abs = join(base, &parts[..=i]);
        let is_anchor = i >= n - anchor_tail || has_marker_in(&abs, opts);
        if is_anchor {
            out.push(DirPart {
                text: part.clone(),
                class: Class::Anchor,
            });
            continue;
        }
        let text = match opts.budget {
            None => part.clone(),
            Some(budget) => {
                let short = shorten_component(&abs, part);
                let gain = part
                    .chars()
                    .count()
                    .saturating_sub(short.chars().count() + delim_len);
                if gain > 0 && saved < budget {
                    saved += gain;
                    short
                } else {
                    part.clone()
                }
            }
        };
        let class = if text != *part {
            Class::Shortened
        } else {
            Class::Normal
        };
        out.push(DirPart { text, class });
    }
    out
}

/// `truncate_middle` / `truncate_from_right`: the first and last parts are kept, interior levels are truncated.
fn fold_truncate(parts: &[String], len: usize, delim: &str, middle: bool) -> Vec<DirPart> {
    let d = delim.chars().count();
    let n = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            // p10k handles only the 2nd through 2nd-to-last parts (first and last as is).
            let interior = i > 0 && i + 1 < n;
            let chars: Vec<char> = part.chars().collect();
            let suf = if middle { len } else { 0 };
            if interior && chars.len() > len + suf + d {
                let head: String = chars[..len].iter().collect();
                let tail: String = chars[chars.len() - suf..].iter().collect();
                DirPart {
                    text: format!("{head}{delim}{tail}"),
                    class: Class::Shortened,
                }
            } else {
                DirPart {
                    text: part.clone(),
                    class: class_for(i, n),
                }
            }
        })
        .collect()
}

/// `truncate_to_last`: keep only the last `len` levels, dropping the rest into a single ellipsis.
fn fold_to_last(parts: &[String], len: usize, delim: &str) -> Vec<DirPart> {
    let n = parts.len();
    if n <= len {
        return plain(parts);
    }
    let mut out = Vec::new();
    if !delim.is_empty() {
        out.push(DirPart {
            text: delim.to_string(),
            class: Class::Shortened,
        });
    }
    for (i, part) in parts.iter().enumerate().skip(n - len) {
        out.push(DirPart {
            text: part.clone(),
            class: class_for(i, n),
        });
    }
    out
}

/// `truncate_to_first_and_last`: first `len` + last `len`, with one ellipsis in between.
fn fold_first_last(parts: &[String], len: usize, delim: &str) -> Vec<DirPart> {
    let n = parts.len();
    if n <= len * 2 {
        return plain(parts);
    }
    let mut out: Vec<DirPart> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i < len {
            out.push(DirPart {
                text: part.clone(),
                class: Class::Anchor,
            });
        } else if i == len {
            out.push(DirPart {
                text: delim.to_string(),
                class: Class::Shortened,
            });
        } else if i >= n - len {
            out.push(DirPart {
                text: part.clone(),
                class: Class::Anchor,
            });
        }
    }
    out
}

/// `truncate_absolute(_chars)`: truncate the whole path by character count, keeping the tail.
fn fold_absolute(parts: &[String], len: usize, delim: &str) -> Vec<DirPart> {
    // Accumulate parts back from the end until len is exceeded; the overflowing level keeps only its tail characters.
    let mut acc = 0usize;
    let mut start = 0usize;
    for i in (0..parts.len()).rev() {
        let l = parts[i].chars().count() + 1; // +1 for the separator
        if acc + l > len {
            start = i;
            break;
        }
        acc += l;
    }
    let mut out: Vec<DirPart> = Vec::new();
    if start > 0 {
        // The overflowing level loses its head and everything before it is dropped.
        let part = &parts[start];
        let keep = len.saturating_sub(acc);
        let chars: Vec<char> = part.chars().collect();
        let text = if keep > 0 && keep < chars.len() {
            format!(
                "{}{}",
                delim,
                chars[chars.len() - keep..].iter().collect::<String>()
            )
        } else {
            part.clone()
        };
        out.push(DirPart {
            text,
            class: Class::Shortened,
        });
    }
    for (i, part) in parts.iter().enumerate().skip(start) {
        out.push(DirPart {
            text: part.clone(),
            class: class_for(i, parts.len()),
        });
    }
    out
}

/// `truncate_with_folder_marker`: gaps between marker files fold into the ellipsis.
fn fold_folder_marker(parts: &[String], base: &Path, delim: &str, opts: &Opts) -> Vec<DirPart> {
    let n = parts.len();
    let mut marks: Vec<usize> = Vec::new();
    for i in (0..n).rev() {
        let abs = join(base, &parts[..=i]);
        if has_marker_in(&abs, opts) {
            marks.push(i);
        }
    }
    marks.push(usize::MAX); // stands for the 1 p10k appends (so the elided span is computable even without a marker at the front)
    let mut hidden: Vec<bool> = vec![false; n];
    for w in marks.windows(2) {
        let (hi, lo) = (w[0], w[1]);
        let gap = if lo == usize::MAX { hi + 1 } else { hi - lo };
        if gap > 2 {
            let from = if lo == usize::MAX { 0 } else { lo + 1 };
            for h in hidden.iter_mut().take(hi).skip(from) {
                *h = true;
            }
        }
    }
    let mut out: Vec<DirPart> = Vec::new();
    let mut elided = false;
    for (i, part) in parts.iter().enumerate() {
        if hidden[i] {
            if !elided && !delim.is_empty() {
                out.push(DirPart {
                    text: delim.to_string(),
                    class: Class::Shortened,
                });
                elided = true;
            }
            continue;
        }
        out.push(DirPart {
            text: part.clone(),
            class: class_for(i, n),
        });
    }
    out
}

/// Default branch for an empty strategy: keep the last `len` levels, prefixed by one ellipsis.
fn fold_keep_last(parts: &[String], len: usize, delim: &str) -> Vec<DirPart> {
    let n = parts.len();
    if n <= len {
        return plain(parts);
    }
    let mut out = vec![DirPart {
        text: delim.to_string(),
        class: Class::Shortened,
    }];
    for (i, part) in parts.iter().enumerate().skip(n - len) {
        out.push(DirPart {
            text: part.clone(),
            class: class_for(i, n),
        });
    }
    out
}

/// Keep everything (when the strategy needs no folding).
fn plain(parts: &[String]) -> Vec<DirPart> {
    let n = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| DirPart {
            text: p.clone(),
            class: class_for(i, n),
        })
        .collect()
}

/// Class for a non-folded part: the last part is the anchor (current directory), the rest are normal.
fn class_for(i: usize, n: usize) -> Class {
    if i + 1 == n {
        Class::Anchor
    } else {
        Class::Normal
    }
}

fn join(base: &Path, parts: &[String]) -> PathBuf {
    let mut p = base.to_path_buf();
    for x in parts {
        p.push(x);
    }
    p
}

/// Shortest unique prefix of the part among its directory siblings (cache first).
fn shorten_component(abs: &Path, name: &str) -> String {
    let parent = abs.parent().unwrap_or_else(|| Path::new("/"));
    let parent_mtime = file_mtime(parent).unwrap_or(-1);
    if let Some(v) = CACHE.with(|c| c.borrow().get(&(abs.to_path_buf(), parent_mtime)).cloned()) {
        return v;
    }
    let Some(sibs) = list_dir(parent) else {
        return name.to_string();
    };
    let mut best = name.to_string();
    let nchars = name.chars().count();
    for j in 1..=nchars {
        let prefix: String = name.chars().take(j).collect();
        if sibs.iter().filter(|s| s.starts_with(&prefix)).count() <= 1 {
            best = prefix;
            break;
        }
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .insert((abs.to_path_buf(), parent_mtime), best.clone());
    });
    best
}

/// Whether this absolute path prefix has an ancestor containing a marker file (only `opts.marker` when it is non-empty).
fn has_marker_in(abs: &Path, opts: &Opts) -> bool {
    if !opts.marker.is_empty() {
        return abs.join(&opts.marker).exists();
    }
    MARKERS
        .iter()
        .any(|m| abs.join(m).exists() || abs.join(m).is_dir())
}

fn list_dir(path: &Path) -> Option<Vec<String>> {
    std::fs::read_dir(path).ok().map(|rd| {
        rd.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect()
    })
}

fn file_mtime(path: &Path) -> Option<i64> {
    let md = std::fs::metadata(path).ok()?;
    let t = md.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(strategy: Strategy, length: usize, delimiter: &str) -> Opts {
        Opts {
            strategy,
            length,
            delimiter: delimiter.into(),
            marker: String::new(),
            // Give the test ample budget (equivalent to "the line does not fit, fold freely").
            budget: Some(usize::MAX),
        }
    }

    fn texts(cwd: &str, o: &Opts) -> Vec<String> {
        shorten(Path::new(cwd), None, o)
            .into_iter()
            .map(|p| p.text)
            .collect()
    }

    #[test]
    fn truncate_middle_keeps_head_and_tail() {
        let o = opts(Strategy::TruncateMiddle, 2, "…");
        // First and last parts as is; interior parts keep the first 2 + ellipsis + last 2.
        assert_eq!(
            texts("/alpha/bravocharlie/delta/echo", &o),
            vec!["alpha", "br…ie", "delta", "echo"]
        );
    }

    #[test]
    fn truncate_from_right_keeps_head() {
        let o = opts(Strategy::TruncateFromRight, 3, "…");
        // delta (5) > 3+1 is truncated too; the last part is always left as is.
        assert_eq!(
            texts("/alpha/bravocharlie/delta/echo", &o),
            vec!["alpha", "bra…", "del…", "echo"]
        );
    }

    #[test]
    fn truncate_to_last_keeps_tail_levels() {
        let o = opts(Strategy::TruncateToLast, 2, "…");
        assert_eq!(
            texts("/alpha/bravo/charlie/delta", &o),
            vec!["…", "charlie", "delta"]
        );
    }

    #[test]
    fn truncate_to_first_and_last_collapses_middle() {
        let o = opts(Strategy::TruncateToFirstAndLast, 1, "…");
        assert_eq!(
            texts("/alpha/bravo/charlie/delta/echo", &o),
            vec!["alpha", "…", "echo"]
        );
    }

    #[test]
    fn fold_to_last_is_the_default_branch() {
        let o = opts(Strategy::FoldToLast, 2, "…");
        assert_eq!(texts("/a/b/c/d", &o), vec!["…", "c", "d"]);
    }

    #[test]
    fn short_paths_are_left_alone() {
        for s in [
            Strategy::TruncateMiddle,
            Strategy::TruncateFromRight,
            Strategy::TruncateToLast,
            Strategy::TruncateToFirstAndLast,
        ] {
            let o = opts(s, 3, "…");
            assert_eq!(texts("/a/b", &o), vec!["a", "b"], "{s:?}");
        }
    }

    #[test]
    fn home_prefix_keeps_last_as_anchor() {
        let home = std::env::temp_dir().join("p11k-shorten-home2");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Projects").join("p11k")).unwrap();
        std::fs::create_dir_all(home.join("Templates")).unwrap();
        let cwd = home.join("Projects").join("p11k");
        let o = opts(Strategy::TruncateToUnique, 1, "");
        let parts = shorten(&cwd, Some(&home), &o);
        assert_eq!(parts.last().map(|p| p.text.as_str()), Some("p11k"));
        assert_eq!(parts.last().map(|p| p.class), Some(Class::Anchor));
    }
}
