//! `dir` 段截断策略 `truncate_to_unique`(对齐 p10k,见 p10k.zsh P:1896-1992)。
//!
//! 从当前目录往回,把每个**非锚定**部件缩短到"它在自己目录兄弟中的最短唯一前缀";
//! 锚点(home `~`/根、末尾 `shortenlen` 级、含 marker 文件的祖先)不缩;缩短后无省略符
//! (lean 置空 SHORTEN_DELIMITER)。
//!
//! 返回按类别标记的部件([`DirPart`]),render 据此逐部件上色:
//! 缩短=103、锚=39(粗体)、普通=31,`/` 分隔符本色。
//!
//! # 性能(对齐 p10k 的 mtime 缓存)
//!
//! 每级按 `(绝对目录, 父目录 mtime_ns)` 缓存缩短结果:目录不变直接复用,不加 `readdir`。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

thread_local! {
    static CACHE: RefCell<HashMap<(PathBuf, i64), String>> = RefCell::new(HashMap::new());
}

/// 部件类别(决定上色)。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 锚(首 `~`/根/当前目录/marker 祖先),不缩,varies 颜色。
    Anchor,
    /// 被缩短(唯一前缀)。
    Shortened,
    /// 普通(未缩但非锚)。
    Normal,
}

/// 折叠后的一个部件。
#[derive(Clone)]
pub struct DirPart {
    pub text: String,
    pub class: Class,
}

/// 折叠绝对路径 `cwd`,返回部件序列(不含 `~`/`/`;调用方拼装并加前缀)。
pub fn truncate_to_unique(cwd: &Path, shortenlen: usize, home: Option<&Path>) -> Vec<DirPart> {
    let shortenlen = shortenlen.max(1);
    if let Some(home) = home {
        if let Ok(rel) = cwd.strip_prefix(home) {
            let parts: Vec<String> = rel
                .components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            return fold(&parts, shortenlen, home);
        }
    }
    let parts: Vec<String> = cwd
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    fold(&parts, shortenlen, Path::new("/"))
}

/// 折叠一组相对部件(相对 `base`),返回带类别的部件序列。
/// 首部件(若绝对路径是根后第一个、或 home 后的第一个)与末 `shortenlen` 个、
/// marker 祖先为 Anchor;其余缩到唯一前缀 → Shortened。
fn fold(parts: &[String], shortenlen: usize, base: &Path) -> Vec<DirPart> {
    let n = parts.len();
    let anchor_tail = shortenlen.min(n);
    let mut out: Vec<DirPart> = Vec::with_capacity(n);
    for i in 0..n {
        let abs = join(base, &parts[..=i]);
        let is_anchor = i >= n - anchor_tail || has_marker_in(&abs);
        if is_anchor {
            out.push(DirPart { text: parts[i].clone(), class: Class::Anchor });
        } else {
            let text = shorten_component(&abs, &parts[i]);
            let class = if text != parts[i] { Class::Shortened } else { Class::Normal };
            out.push(DirPart { text, class });
        }
    }
    out
}

fn join(base: &Path, parts: &[String]) -> PathBuf {
    let mut p = base.to_path_buf();
    for x in parts {
        p.push(x);
    }
    p
}

/// 部件在它目录兄弟中的最短唯一前缀(缓存优先)。
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
        c.borrow_mut().insert((abs.to_path_buf(), parent_mtime), best.clone());
    });
    best
}

/// 该绝对路径前缀是否含 marker 文件的祖先。
fn has_marker_in(abs: &Path) -> bool {
    const MARKERS: &[&str] = &[".git", ".hg", ".svn", "Cargo.toml", "package.json", "go.mod"];
    MARKERS.iter().any(|m| abs.join(m).exists() || abs.join(m).is_dir())
}

fn list_dir(path: &Path) -> Option<Vec<String>> {
    std::fs::read_dir(path)
        .ok()
        .map(|rd| {
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

    #[test]
    fn home_prefix_is_tilde_and_last_is_anchor() {
        let home = std::env::temp_dir().join("p11k-shorten-home");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Projects").join("p11k")).unwrap();
        std::fs::create_dir_all(home.join("Templates")).unwrap();
        let cwd = home.join("Projects").join("p11k");
        let parts = truncate_to_unique(&cwd, 1, Some(&home));
        assert!(parts.last().map(|p| p.text.as_str()) == Some("p11k"));
        assert!(parts.last().map(|p| p.class) == Some(Class::Anchor));
    }

    #[test]
    fn non_home_produces_parts() {
        let parts = truncate_to_unique(Path::new("/a/b/c"), 1, None);
        assert_eq!(parts.last().map(|p| p.text.as_str()), Some("c"));
    }
}
