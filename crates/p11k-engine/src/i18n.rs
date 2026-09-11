//! gettext 薄封装：源码里写英文 msgid，运行时按 locale 取 `po/<lang>.po` 的翻译。
//!
//! 默认就是英文（没有翻译时 gettext 原样返回 msgid），中文来自 `po/zh_CN.po`。
//! 语言由标准环境变量决定（`LANG` / `LC_ALL` / `LANGUAGE`），例如 `LANG=zh_CN.UTF-8`。
//! 翻译目录默认是构建期编译出来的 `$OUT_DIR/locale`，可用 `P11K_LOCALEDIR` 覆盖
//! （装到系统时指到 `/usr/share/locale`）。

use std::sync::Once;

use gettextrs::{LocaleCategory, bindtextdomain, gettext, setlocale, textdomain};

static INIT: Once = Once::new();

/// 绑定翻译目录，进程里调用一次即可（幂等）。
pub fn init() {
    INIT.call_once(|| {
        // 空字符串 = 从环境变量取 locale。
        let _ = setlocale(LocaleCategory::LcAll, "");
        let dir =
            std::env::var("P11K_LOCALEDIR").unwrap_or_else(|_| env!("P11K_LOCALEDIR").to_string());
        let _ = bindtextdomain("p11k", dir);
        let _ = textdomain("p11k");
    });
}

/// 翻译一条 msgid。找不到翻译时返回原文。
pub fn t(s: &str) -> String {
    gettext(s)
}

/// 标记一个字符串是 msgid，不翻译。
///
/// 给「先收下文案、稍后在别处统一翻译」的地方用（wizard 的问题标题与选项：
/// 调用点只传字面量，翻译发生在 `ask_choice_preview`/`ask_yn` 里）。有了它，
/// `xgettext --keyword=msgid` 就能把那些字面量一并提取进 `po/p11k.pot`。
pub const fn msgid(s: &'static str) -> &'static str {
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// 装了文案的源文件（与 tools/i18n.sh 扫的是同一批；i18n.rs 自己不算，
    /// 它只有封装，没有文案）。
    const SOURCES: &[(&str, &str)] = &[
        ("main.rs", include_str!("main.rs")),
        ("config.rs", include_str!("config.rs")),
        ("render.rs", include_str!("render.rs")),
        ("presets.rs", include_str!("presets.rs")),
        ("wizard.rs", include_str!("wizard.rs")),
    ];

    /// 解析 .po/.pot:每条 msgid 取出来（含续行分片），头部那条空 msgid 跳过。
    fn catalog_msgids(text: &str) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        let mut cur: Option<String> = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("msgid ") {
                if let Some(s) = cur.take() {
                    ids.insert(s);
                }
                cur = Some(unescape(rest.trim_matches('"')));
            } else if let Some(rest) = line.strip_prefix('"') {
                // 上一条 msgid 的续行分片（msgstr 的续行不会走到这里：
                // msgstr 行本身以 `msgstr` 开头，会先清掉 cur）。
                if let Some(s) = cur.as_mut() {
                    s.push_str(&unescape(rest.trim_matches('"')));
                }
            } else {
                if let Some(s) = cur.take() {
                    ids.insert(s);
                }
            }
        }
        if let Some(s) = cur {
            ids.insert(s);
        }
        ids.remove("");
        ids
    }

    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        out
    }

    /// 扫源码里的 `t("…")` 与 `msgid("…")`。
    ///
    /// 逐行去掉 `//` 注释后拼成一段字符流再找标记：rustfmt 会把长文案折行，
    /// 所以 `t(`/`msgid(` 后面允许换行与缩进，只看紧跟的是不是字符串字面量。
    /// 标记前面必须是分隔符，以免把 `format!(`、`gettext(` 之类误判为标记。
    fn source_msgids(name: &str, src: &str) -> BTreeSet<String> {
        let code: String = src
            .lines()
            .map(|l| match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut ids = BTreeSet::new();
        for marker in ["t(", "msgid("] {
            let mut from = 0;
            while let Some(pos) = code[from..].find(marker) {
                let at = from + pos;
                let prev = code[..at].chars().next_back();
                let after = &code[at + marker.len()..];
                let indented = after.len() - after.trim_start().len();
                let is_call = !prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
                if is_call && let Some(rest) = after.trim_start().strip_prefix('"') {
                    let end = rest.find('"').unwrap_or_else(|| {
                        panic!("{name}: unterminated string literal: {after:.40}")
                    });
                    assert!(
                        !rest[..end].ends_with('\\'),
                        "{name}: escaped quote, scanner cannot handle it: {after:.40}"
                    );
                    ids.insert(unescape(&rest[..end]));
                }
                from = at + marker.len() + indented;
            }
        }
        ids
    }

    fn all_source_msgids() -> BTreeSet<String> {
        SOURCES
            .iter()
            .flat_map(|(name, src)| source_msgids(name, src))
            .collect()
    }

    #[test]
    fn source_msgids_match_the_pot() {
        let src = all_source_msgids();
        let pot = catalog_msgids(include_str!("../po/p11k.pot"));
        assert!(
            src.len() > 60,
            "too few msgids scanned from sources: {src:?}"
        );
        let missing: Vec<_> = src.difference(&pot).collect();
        let stale: Vec<_> = pot.difference(&src).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "sources and po/p11k.pot are out of sync, run tools/i18n.sh extract\n  only in sources: {missing:?}\n  only in pot: {stale:?}"
        );
    }

    #[test]
    fn catalogs_cover_every_msgid() {
        let src = all_source_msgids();
        let pot = catalog_msgids(include_str!("../po/p11k.pot"));
        let po = catalog_msgids(include_str!("../po/zh_CN.po"));

        let untranslated: Vec<_> = pot.difference(&po).collect();
        assert!(
            untranslated.is_empty(),
            "zh_CN.po is missing entries (run tools/i18n.sh update, then fill in msgstr): {untranslated:?}"
        );
        let stale: Vec<_> = po.difference(&src).collect();
        assert!(
            stale.is_empty(),
            "zh_CN.po has entries that no longer exist in the sources: {stale:?}"
        );
    }

    #[test]
    fn chinese_locale_gets_translations_and_missing_ones_fall_back() {
        let dir = env!("P11K_LOCALEDIR");
        let _ = bindtextdomain("p11k", dir);
        let _ = textdomain("p11k");
        if !std::path::Path::new(dir)
            .join("zh_CN/LC_MESSAGES/p11k.mo")
            .exists()
        {
            eprintln!("skipping: no compiled zh_CN catalog (msgfmt missing?)");
            return;
        }
        let got = setlocale(LocaleCategory::LcAll, "zh_CN.UTF-8");
        if !got.as_deref().is_some_and(|s| !s.is_empty()) {
            eprintln!("skipping: system has no zh_CN.UTF-8 locale");
            return;
        }

        assert_eq!(gettext("Yes."), "是。");
        assert_eq!(gettext("Prompt Style"), "提示符风格");
        assert_eq!(gettext("Restart from the beginning."), "从头再来。");
        // 没有翻译条目的 msgid 原样返回(英文即默认语言)。
        let unknown = String::from("no such msgid");
        assert_eq!(gettext(&unknown), unknown);

        let _ = setlocale(LocaleCategory::LcAll, "C");
    }
}
