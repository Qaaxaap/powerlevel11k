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

/// Filesystem access for the shortening strategies. The production implementation is
/// [`RealFs`]; tests inject an in-memory one, so the strategies do not read the host.
pub(super) trait Fs {
    /// Names in the directory, or None when it cannot be read.
    fn read_dir(&self, path: &Path) -> Option<Vec<String>>;
    /// Directory mtime in nanoseconds since the epoch, used as the cache key.
    fn mtime_ns(&self, path: &Path) -> Option<i64>;
    /// Whether the path exists.
    fn exists(&self, path: &Path) -> bool;
    /// Whether the path is a directory.
    fn is_dir(&self, path: &Path) -> bool;
}

/// The process filesystem.
pub(super) struct RealFs;

impl Fs for RealFs {
    fn read_dir(&self, path: &Path) -> Option<Vec<String>> {
        std::fs::read_dir(path).ok().map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect()
        })
    }

    fn mtime_ns(&self, path: &Path) -> Option<i64> {
        let md = std::fs::metadata(path).ok()?;
        let t = md.modified().ok()?;
        Some(t.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i64)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// The input of one shortening run: the parts, their base directory, the options and the
/// filesystem the strategies may read.
struct FoldCtx<'a> {
    parts: &'a [String],
    base: &'a Path,
    opts: &'a Opts,
    fs: &'a dyn Fs,
}

impl FoldCtx<'_> {
    fn len(&self) -> usize {
        self.opts.length.max(1)
    }

    fn delim(&self) -> &str {
        &self.opts.delimiter
    }

    fn n(&self) -> usize {
        self.parts.len()
    }

    fn abs(&self, up_to: usize) -> PathBuf {
        join(self.base, &self.parts[..=up_to])
    }
}

/// Fold the absolute path `cwd` and return the part sequence (without `~`/`/`; the caller
/// assembles it and adds prefixes).
pub fn shorten(cwd: &Path, home: Option<&Path>, opts: &Opts) -> Vec<DirPart> {
    shorten_with_fs(cwd, home, opts, &RealFs)
}

/// [`shorten`] with an injected filesystem.
fn shorten_with_fs(cwd: &Path, home: Option<&Path>, opts: &Opts, fs: &dyn Fs) -> Vec<DirPart> {
    let (parts, base) = split_parts(cwd, home);
    if parts.is_empty() {
        return Vec::new();
    }
    let ctx = FoldCtx {
        parts: &parts,
        base: &base,
        opts,
        fs,
    };
    strategy(opts.strategy)(&ctx)
}

/// The strategy table: one function per strategy. The dispatcher only picks the row.
fn strategy(s: Strategy) -> fn(&FoldCtx<'_>) -> Vec<DirPart> {
    match s {
        Strategy::TruncateToUnique => fold_unique,
        Strategy::TruncateMiddle => fold_middle,
        Strategy::TruncateFromRight => fold_from_right,
        Strategy::TruncateToLast => fold_to_last,
        Strategy::TruncateToFirstAndLast => fold_first_last,
        Strategy::TruncateAbsolute => fold_absolute,
        Strategy::TruncateWithFolderMarker => fold_folder_marker,
        Strategy::FoldToLast => fold_keep_last,
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

/// One output slot: a kept path part or the ellipsis. The strategies choose slots; the
/// materialisation below turns them into rendered parts in one place.
enum Slot {
    /// The ellipsis, classed as shortened.
    Ellipsis,
    /// A kept part. `text` replaces the original when the strategy edits the part.
    Part {
        index: usize,
        class: Class,
        text: Option<String>,
    },
}

impl Slot {
    fn part(index: usize, class: Class) -> Slot {
        Slot::Part {
            index,
            class,
            text: None,
        }
    }

    fn edited(index: usize, class: Class, text: String) -> Slot {
        Slot::Part {
            index,
            class,
            text: Some(text),
        }
    }
}

/// Turn slots into parts. The ellipsis takes the configured delimiter.
fn materialize(parts: &[String], delim: &str, slots: Vec<Slot>) -> Vec<DirPart> {
    slots
        .into_iter()
        .map(|slot| match slot {
            Slot::Ellipsis => DirPart {
                text: delim.to_string(),
                class: Class::Shortened,
            },
            Slot::Part { index, class, text } => DirPart {
                text: text.unwrap_or_else(|| parts[index].clone()),
                class,
            },
        })
        .collect()
}

/// Slots for a kept range of parts.
fn range_slots(parts_len: usize, range: std::ops::Range<usize>) -> Vec<Slot> {
    range
        .map(|i| Slot::part(i, class_for(i, parts_len)))
        .collect()
}

/// `truncate_to_unique`: anchors are kept, the rest are shortened to unique prefixes.
///
/// With `opts.budget` as `None` the whole path is left as is; with `Some(n)` levels are folded
/// front to back until the accumulated saved columns reach n.
fn fold_unique(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let n = ctx.n();
    let shortenlen = ctx.len();
    let anchor_tail = shortenlen.min(n);
    let delim_len = ctx.delim().chars().count();
    let mut saved = 0usize;
    let mut slots: Vec<Slot> = Vec::with_capacity(n);
    for (i, part) in ctx.parts.iter().enumerate() {
        let abs = ctx.abs(i);
        let is_anchor = i >= n - anchor_tail || has_marker_in(ctx, &abs);
        if is_anchor {
            slots.push(Slot::part(i, Class::Anchor));
            continue;
        }
        let text = match ctx.opts.budget {
            None => part.clone(),
            Some(budget) => {
                let short = shorten_component(ctx, &abs, part);
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
        slots.push(Slot::edited(i, class, text));
    }
    materialize(ctx.parts, "", slots)
}

/// `truncate_middle`: the first and last parts are kept, interior levels are truncated.
fn fold_middle(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    fold_truncate(ctx, true)
}

/// `truncate_from_right`: the first part is kept, interior levels keep their head.
fn fold_from_right(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    fold_truncate(ctx, false)
}

fn fold_truncate(ctx: &FoldCtx<'_>, middle: bool) -> Vec<DirPart> {
    let len = ctx.len();
    let delim = ctx.delim();
    let d = delim.chars().count();
    let n = ctx.n();
    let mut slots = Vec::with_capacity(n);
    for (i, part) in ctx.parts.iter().enumerate() {
        // p10k handles only the 2nd through 2nd-to-last parts (first and last as is).
        let interior = i > 0 && i + 1 < n;
        let chars: Vec<char> = part.chars().collect();
        let suf = if middle { len } else { 0 };
        if interior && chars.len() > len + suf + d {
            let head: String = chars[..len].iter().collect();
            let tail: String = chars[chars.len() - suf..].iter().collect();
            slots.push(Slot::edited(
                i,
                Class::Shortened,
                format!("{head}{delim}{tail}"),
            ));
        } else {
            slots.push(Slot::part(i, class_for(i, n)));
        }
    }
    materialize(ctx.parts, "", slots)
}

/// `truncate_to_last`: keep only the last `len` levels, dropping the rest into one ellipsis.
fn fold_to_last(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let len = ctx.len();
    let delim = ctx.delim();
    let n = ctx.n();
    if n <= len {
        return plain(ctx.parts);
    }
    let mut slots = Vec::new();
    if !delim.is_empty() {
        slots.push(Slot::Ellipsis);
    }
    slots.extend(range_slots(n, n - len..n));
    materialize(ctx.parts, delim, slots)
}

/// `truncate_to_first_and_last`: first `len` + last `len`, with one ellipsis in between.
fn fold_first_last(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let len = ctx.len();
    let delim = ctx.delim();
    let n = ctx.n();
    if n <= len * 2 {
        return plain(ctx.parts);
    }
    let mut slots = Vec::with_capacity(len * 2 + 1);
    for i in 0..len {
        slots.push(Slot::part(i, Class::Anchor));
    }
    if !delim.is_empty() {
        slots.push(Slot::Ellipsis);
    }
    for i in n - len..n {
        slots.push(Slot::part(i, Class::Anchor));
    }
    materialize(ctx.parts, delim, slots)
}

/// `truncate_absolute(_chars)`: truncate the whole path by character count, keeping the tail.
fn fold_absolute(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let len = ctx.len();
    let delim = ctx.delim();
    let n = ctx.n();
    // Accumulate parts back from the end until len is exceeded; the overflowing level keeps
    // only its tail characters.
    let mut acc = 0usize;
    let mut start = 0usize;
    for i in (0..n).rev() {
        let l = ctx.parts[i].chars().count() + 1; // +1 for the separator
        if acc + l > len {
            start = i;
            break;
        }
        acc += l;
    }
    let mut slots = Vec::with_capacity(n - start);
    if start > 0 {
        let part = &ctx.parts[start];
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
        slots.push(Slot::edited(start, Class::Shortened, text));
        slots.extend(range_slots(n, start + 1..n));
    } else {
        slots.extend(range_slots(n, start..n));
    }
    materialize(ctx.parts, "", slots)
}

/// `truncate_with_folder_marker`: gaps between marker files fold into the ellipsis.
fn fold_folder_marker(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let delim = ctx.delim();
    let n = ctx.n();
    let mut marks: Vec<usize> = Vec::new();
    for i in (0..n).rev() {
        let abs = ctx.abs(i);
        if has_marker_in(ctx, &abs) {
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
    let mut slots = Vec::with_capacity(n);
    let mut elided = false;
    for (i, _) in ctx.parts.iter().enumerate() {
        if hidden[i] {
            if !elided && !delim.is_empty() {
                slots.push(Slot::Ellipsis);
                elided = true;
            }
            continue;
        }
        slots.push(Slot::part(i, class_for(i, n)));
    }
    materialize(ctx.parts, delim, slots)
}

/// Default branch for an empty strategy: keep the last `len` levels, prefixed by one ellipsis.
fn fold_keep_last(ctx: &FoldCtx<'_>) -> Vec<DirPart> {
    let len = ctx.len();
    let delim = ctx.delim();
    let n = ctx.n();
    if n <= len {
        return plain(ctx.parts);
    }
    let mut slots = Vec::with_capacity(len + 1);
    if !delim.is_empty() {
        slots.push(Slot::Ellipsis);
    }
    slots.extend(range_slots(n, n - len..n));
    materialize(ctx.parts, delim, slots)
}

/// Keep everything (when the strategy needs no folding).
fn plain(parts: &[String]) -> Vec<DirPart> {
    materialize(parts, "", range_slots(parts.len(), 0..parts.len()))
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
fn shorten_component(ctx: &FoldCtx<'_>, abs: &Path, name: &str) -> String {
    let parent = abs.parent().unwrap_or_else(|| Path::new("/"));
    let parent_mtime = ctx.fs.mtime_ns(parent).unwrap_or(-1);
    if let Some(v) = CACHE.with(|c| c.borrow().get(&(abs.to_path_buf(), parent_mtime)).cloned()) {
        return v;
    }
    let Some(sibs) = ctx.fs.read_dir(parent) else {
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

/// Whether this absolute path prefix has an ancestor containing a marker file (only
/// `opts.marker` when it is non-empty).
fn has_marker_in(ctx: &FoldCtx<'_>, abs: &Path) -> bool {
    if !ctx.opts.marker.is_empty() {
        return ctx.fs.exists(&abs.join(&ctx.opts.marker));
    }
    MARKERS
        .iter()
        .any(|m| ctx.fs.exists(&abs.join(m)) || ctx.fs.is_dir(&abs.join(m)))
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

    /// An empty delimiter must not leave an empty part: the slot stays out of the output.
    #[test]
    fn empty_delimiter_leaves_no_empty_part() {
        let o = opts(Strategy::FoldToLast, 1, "");
        assert_eq!(texts("/a/b/c", &o), vec!["c"]);
        let o = opts(Strategy::TruncateToFirstAndLast, 1, "");
        assert_eq!(texts("/a/b/c/d/e", &o), vec!["a", "e"]);
    }

    /// The overflowing level is kept once, in its shortened form, not repeated.
    #[test]
    fn absolute_keeps_the_overflowing_level_once() {
        let o = opts(Strategy::TruncateAbsolute, 4, "\u{2026}");
        assert_eq!(texts("/alpha/bravo/charlie", &o), vec!["\u{2026}rlie"]);
    }

    /// An in-memory filesystem: the shortening strategies do not touch the host in tests.
    struct FakeFs {
        dirs: std::collections::BTreeMap<PathBuf, Vec<String>>,
        mtime: i64,
    }

    impl FakeFs {
        fn new(entries: &[(&str, &[&str])]) -> FakeFs {
            let dirs = entries
                .iter()
                .map(|(dir, names)| {
                    (
                        PathBuf::from(dir),
                        names.iter().map(|s| s.to_string()).collect(),
                    )
                })
                .collect();
            FakeFs { dirs, mtime: 1 }
        }
    }

    impl Fs for FakeFs {
        fn read_dir(&self, path: &Path) -> Option<Vec<String>> {
            self.dirs.get(path).cloned()
        }

        fn mtime_ns(&self, _path: &Path) -> Option<i64> {
            Some(self.mtime)
        }

        fn exists(&self, path: &Path) -> bool {
            self.dirs.contains_key(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains_key(path)
        }
    }

    #[test]
    fn home_prefix_keeps_last_as_anchor() {
        // The engine reads the tree through the injected filesystem, not the host.
        let fs = FakeFs::new(&[
            ("/home/u", &["Projects", "Templates"]),
            ("/home/u/Projects", &["p11k"]),
            ("/home/u/Templates", &[]),
            ("/home/u/Projects/p11k", &[]),
        ]);
        let home = Path::new("/home/u");
        let cwd = home.join("Projects").join("p11k");
        let o = opts(Strategy::TruncateToUnique, 1, "");
        let parts = shorten_with_fs(&cwd, Some(home), &o, &fs);
        assert_eq!(parts.last().map(|p| p.text.as_str()), Some("p11k"));
        assert_eq!(parts.last().map(|p| p.class), Some(Class::Anchor));
        // The non-anchor level shortens to the shortest unique prefix among its siblings.
        assert_eq!(parts.first().map(|p| p.text.as_str()), Some("P"));
        assert_eq!(parts.first().map(|p| p.class), Some(Class::Shortened));
    }

    /// The sibling list comes from the injected filesystem, so the result is deterministic.
    #[test]
    fn unique_prefix_reads_the_injected_siblings() {
        let o = opts(Strategy::TruncateToUnique, 1, "");
        let home = Path::new("/x");
        let fs = FakeFs::new(&[
            ("/x", &["Projects", "Photos"]),
            ("/x/Projects", &["sub"]),
            ("/x/Projects/sub", &[]),
        ]);
        let parts = shorten_with_fs(&home.join("Projects").join("sub"), Some(home), &o, &fs);
        // With two P-siblings the prefix must extend past "P".
        assert_eq!(parts.first().map(|p| p.text.as_str()), Some("Pr"));
        // A separate root keeps the prefix cache of the first case out of this one.
        let home = Path::new("/y");
        let fs = FakeFs::new(&[
            ("/y", &["Projects", "Zeta"]),
            ("/y/Projects", &["sub"]),
            ("/y/Projects/sub", &[]),
        ]);
        let parts = shorten_with_fs(&home.join("Projects").join("sub"), Some(home), &o, &fs);
        assert_eq!(parts.first().map(|p| p.text.as_str()), Some("P"));
    }
}
