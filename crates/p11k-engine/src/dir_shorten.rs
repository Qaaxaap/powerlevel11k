//! `dir` 段截断策略（对齐 p10k `POWERLEVEL9K_SHORTEN_STRATEGY` 全量）。
//!
//! 支持的策略：
//! - `truncate_to_unique`（默认）：从当前目录往回，把每个非锚定部件缩短到
//!   "它在目录兄弟中的最短唯一前缀"；缩短后无省略符（对齐 p10k 的默认配置
//!   `SHORTEN_DELIMITER=`）。锚点（home `~`/根、末尾 `length` 级、含 marker
//!   文件的祖先）不缩。
//! - `truncate_middle` / `truncate_from_right`：每级留前 `length`（middle 再留
//!   后 `length`）字符，中间/尾部换成省略符。
//! - `truncate_to_last`：只留末 `length` 级。
//! - `truncate_to_first_and_last`：首 `length` 级 + 末 `length` 级，中间省略。
//! - `truncate_absolute(_chars)`：整条路径按字符数硬截断（保留末尾）。
//! - `truncate_with_folder_marker`：在 marker 文件处折叠。
//!
//! 返回按类别标记的部件，render 据此映射到 state 上色。
//!
//! # 性能（对齐 p10k 的 mtime 缓存）
//!
//! `truncate_to_unique` 每级按 `(绝对目录, 父目录 mtime_ns)` 缓存缩短结果：
//! 目录不变直接复用，不加 `readdir`。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

thread_local! {
    static CACHE: RefCell<HashMap<(PathBuf, i64), String>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// 锚（首 `~`/根/当前目录/marker 祖先），不缩。
    Anchor,
    Shortened,
    /// 普通（未缩但非锚）。
    Normal,
}

/// 折叠后的一个部件。
#[derive(Clone)]
pub struct DirPart {
    pub text: String,
    pub class: Class,
}

/// 截断策略（p10k `POWERLEVEL9K_SHORTEN_STRATEGY`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Strategy {
    /// 每级缩到兄弟中的最短唯一前缀。p11k/p10k 默认。
    #[default]
    TruncateToUnique,
    /// 每级留前 N + 省略符 + 后 N。
    TruncateMiddle,
    /// 每级留前 N + 省略符。
    TruncateFromRight,
    /// 只留末 N 级。
    TruncateToLast,
    /// 首 N 级 + 末 N 级，中间省略。
    TruncateToFirstAndLast,
    /// 整条路径按字符数硬截断（保留末尾）。
    TruncateAbsolute,
    /// 在 marker 文件处折叠。
    TruncateWithFolderMarker,
    /// 空/未知策略（p10k 的默认分支）：只保留末 N 级，前面用省略符。
    FoldToLast,
}

impl Strategy {
    /// p10k 的策略名 → 策略；空/未知走默认分支（`FoldToLast`）。
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

/// 折叠选项（对应 p10k `SHORTEN_*` / `DIR_*` 那几个参数）。
pub struct Opts {
    pub strategy: Strategy,
    /// `SHORTEN_DIR_LENGTH`（保留级数 / 每级字符数）。
    pub length: usize,
    /// `SHORTEN_DELIMITER`；空 = 不留省略符（p10k 的默认配置就是空）。
    pub delimiter: String,
    /// `SHORTEN_FOLDER_MARKER`；空 = 用内置 marker 列表。
    pub marker: String,
    /// `truncate_to_unique` 的折叠预算：`None` = 不折（整行放得下时 p10k 就是
    /// 原样显示），`Some(n)` = 这一行超宽了、需要省出 n 列。p10k 是按行宽动态
    /// 决定折几级的（实测 100 列不折、90 列折一级、80 列折两级）。其它策略与
    /// p10k 一样与宽度无关，忽略本字段。
    pub budget: Option<usize>,
}

/// 内置 marker 文件（p10k 的默认 `markers` 列表）。
const MARKERS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "Cargo.toml",
    "package.json",
    "go.mod",
];

/// 折叠绝对路径 `cwd`，返回部件序列（不含 `~`/`/`；调用方拼装并加前缀）。
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

/// 拆成部件序列 + 基准目录（home 优先，否则根）。
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

/// `truncate_to_unique`：锚点保留，其余缩到唯一前缀。
///
/// `opts.budget` 为 `None` 时整条路径原样（p10k 放得下就不折）；`Some(n)` 时
/// 从前往后逐级折，累计省出的列数够 n 就停——p10k 在窄终端正是这个行为。
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

/// `truncate_middle` / `truncate_from_right`：首尾两个部件保留，中间每级截断。
fn fold_truncate(parts: &[String], len: usize, delim: &str, middle: bool) -> Vec<DirPart> {
    let d = delim.chars().count();
    let n = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            // p10k 只处理第 2 个到倒数第 2 个部件（首尾原样）。
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

/// `truncate_to_last`：只留末 `len` 级，前面的整体丢成一个省略符。
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

/// `truncate_to_first_and_last`：首 `len` + 末 `len`，中间一个省略符。
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

/// `truncate_absolute(_chars)`：整条路径按字符数截断，保留末尾。
fn fold_absolute(parts: &[String], len: usize, delim: &str) -> Vec<DirPart> {
    // 从末尾往回累加部件，直到超过 len；越界的那级只留末尾若干字符。
    let mut acc = 0usize;
    let mut start = 0usize;
    for i in (0..parts.len()).rev() {
        let l = parts[i].chars().count() + 1; // +1 是分隔符
        if acc + l > len {
            start = i;
            break;
        }
        acc += l;
    }
    let mut out: Vec<DirPart> = Vec::new();
    if start > 0 {
        // 越界的那级截掉开头，前面丢弃。
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

/// `truncate_with_folder_marker`：marker 文件之间的间隔折叠成省略符。
fn fold_folder_marker(parts: &[String], base: &Path, delim: &str, opts: &Opts) -> Vec<DirPart> {
    let n = parts.len();
    let mut marks: Vec<usize> = Vec::new();
    for i in (0..n).rev() {
        let abs = join(base, &parts[..=i]);
        if has_marker_in(&abs, opts) {
            marks.push(i);
        }
    }
    marks.push(usize::MAX); // 相当于 p10k 里补的那个 1（最前面没有 marker 时也要能算出省略区间）
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

/// 空策略的默认分支：保留末 `len` 级，前面一个省略符。
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

/// 全保留（策略不需要折叠时）。
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

/// 非折叠部件的类别：最后一个部件是锚（当前目录），其余普通。
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

/// 部件在它目录兄弟中的最短唯一前缀（缓存优先）。
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

/// 该绝对路径前缀是否含 marker 文件的祖先（`opts.marker` 非空时只用它）。
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
            // 测试里给足预算(等价于"行放不下、随便折")。
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
        // 首尾部件原样；中间部件留前 2 + 省略符 + 后 2。
        assert_eq!(
            texts("/alpha/bravocharlie/delta/echo", &o),
            vec!["alpha", "br…ie", "delta", "echo"]
        );
    }

    #[test]
    fn truncate_from_right_keeps_head() {
        let o = opts(Strategy::TruncateFromRight, 3, "…");
        // delta(5) > 3+1 也截；最后一个部件始终原样。
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
