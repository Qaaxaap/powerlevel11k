//! `dir` 段截断策略 `truncate_to_unique`(对齐 p10k,见 p10k.zsh P:1896-1992)。
//!
//! 从当前目录往回,把每个**非锚定**部件缩短到"它在自己目录兄弟中的最短唯一前缀";
//! 锚点(home `~`/根、末尾 `shortenlen` 级、含 marker 文件的祖先)不缩;缩短后无省略符
//! (lean 置空 SHORTEN_DELIMITER)。
//!
//! # 性能(对齐 p10k 的 mtime 缓存)
//!
//! 每级按 `(绝对目录, 父目录 mtime_ns)` 缓存缩短结果:目录不变(父 mtime 不变)直接复用,
//! 不加 `readdir`;只有该目录增删兄弟(父 mtime 变)才重算这一级。锚点级根本不 `readdir`。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

thread_local! {
    static CACHE: RefCell<HashMap<(PathBuf, i64), String>> = RefCell::new(HashMap::new());
}

/// 折叠绝对路径 `cwd`。`home` 提供时,若 cwd 在 home 下,首部用 `~` 且 home 不缩。
pub fn truncate_to_unique(cwd: &Path, shortenlen: usize, home: Option<&Path>) -> String {
    let shortenlen = shortenlen.max(1);
    if let Some(home) = home {
        if let Ok(rel) = cwd.strip_prefix(home) {
            if rel.as_os_str().is_empty() {
                return "~".to_string();
            }
            let parts: Vec<String> = rel
                .components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let folded = fold(&parts, shortenlen, home);
            return format!("~/{folded}");
        }
    }
    let parts: Vec<String> = cwd
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        return "/".to_string();
    }
    format!("/{}", fold(&parts, shortenlen, Path::new("/")))
}

/// 折叠一组相对路径部件(相对 `base`),返回相对串(不含前导 `/` 或 `~`)。
fn fold(parts: &[String], shortenlen: usize, base: &Path) -> String {
    let n = parts.len();
    let anchor_tail = shortenlen.min(n);
    let mut out: Vec<String> = Vec::with_capacity(n);
    for i in 0..n {
        let abs = join(base, &parts[..=i]);
        let is_anchor = i >= n - anchor_tail || has_marker_in(&abs);
        if is_anchor {
            out.push(parts[i].clone());
        } else {
            out.push(shorten_component(&abs, &parts[i]));
        }
    }
    out.join("/")
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
    use std::path::PathBuf;

    #[test]
    fn home_prefix_is_tilde() {
        let home = std::env::temp_dir().join("p11k-shorten-home");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Projects").join("p11k")).unwrap();
        std::fs::create_dir_all(home.join("Templates")).unwrap();
        let cwd = home.join("Projects").join("p11k");
        let s = truncate_to_unique(&cwd, 1, Some(&home));
        assert!(s.starts_with("~/"), "home 前缀应为 ~,got {s}");
        assert!(s.ends_with("p11k"), "当前目录应保留,got {s}");
    }

    #[test]
    fn non_home_absolute_has_leading_slash() {
        let s = truncate_to_unique(Path::new("/a/b/c"), 1, None);
        assert!(s.starts_with('/'));
        assert!(s.ends_with("/c"));
    }
}
