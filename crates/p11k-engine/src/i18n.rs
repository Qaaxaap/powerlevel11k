//! gettext 薄封装：源码里写英文 msgid，运行时按 locale 取 `po/<lang>.po` 的翻译。
//!
//! 默认就是英文（没有翻译时 gettext 原样返回 msgid），中文来自 `po/zh_CN.po`。
//! 语言由标准环境变量决定（`LANG` / `LC_ALL` / `LANGUAGE`），例如 `LANG=zh_CN.UTF-8`。
//! 翻译目录默认是构建期编译出来的 `$OUT_DIR/locale`，可用 `P11K_LOCALEDIR` 覆盖
//! （装到系统时指到 `/usr/share/locale`）。

use std::sync::Once;

use gettextrs::{bindtextdomain, gettext, setlocale, textdomain, LocaleCategory};

static INIT: Once = Once::new();

/// 绑定翻译目录，进程里调用一次即可（幂等）。
pub fn init() {
    INIT.call_once(|| {
        // 空字符串 = 从环境变量取 locale。
        let _ = setlocale(LocaleCategory::LcAll, "");
        let dir = std::env::var("P11K_LOCALEDIR")
            .unwrap_or_else(|_| env!("P11K_LOCALEDIR").to_string());
        let _ = bindtextdomain("p11k", dir);
        let _ = textdomain("p11k");
    });
}

/// 翻译一条 msgid。找不到翻译时返回原文。
pub fn t(msgid: &str) -> String {
    gettext(msgid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 参与文案的源文件:po 里的每条 msgid 都必须能在其中一处找到原文,
    /// 否则就是改了文案没同步翻译(或拼错了)。
    const SOURCES: &[&str] = &[
        include_str!("main.rs"),
        include_str!("config.rs"),
        include_str!("render.rs"),
        include_str!("presets.rs"),
        include_str!("wizard.rs"),
    ];

    fn po_msgids(po: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for block in po.split("\n\n") {
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("msgid \"") {
                    let id = rest.strip_suffix('"').unwrap_or(rest);
                    // 头部那条 `msgid ""`(元信息)不算。
                    if !id.is_empty() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        ids
    }

    #[test]
    fn every_translated_msgid_is_present_in_the_sources() {
        let po = include_str!("../po/zh_CN.po");
        let ids = po_msgids(po);
        assert!(ids.len() > 50, "po 里的条目太少，像是没解析出来");
        for id in ids {
            assert!(
                SOURCES.iter().any(|src| src.contains(&id)),
                "po 里的 msgid 在源码中找不到:{id:?}"
            );
        }
    }

    #[test]
    fn chinese_locale_gets_translations_and_missing_ones_fall_back() {
        let dir = env!("P11K_LOCALEDIR");
        let _ = bindtextdomain("p11k", dir);
        let _ = textdomain("p11k");
        let got = setlocale(LocaleCategory::LcAll, "zh_CN.UTF-8");
        assert!(
            got.as_deref().is_some_and(|s| !s.is_empty()),
            "系统里没有 zh_CN.UTF-8 locale:{got:?}"
        );

        assert_eq!(gettext("Yes."), "是。");
        assert_eq!(gettext("Prompt Style"), "提示符风格");
        assert_eq!(gettext("Restart from the beginning."), "从头再来。");
        // 没有翻译条目的 msgid 原样返回(英文即默认语言)。
        assert_eq!(gettext("no such msgid"), "no such msgid");

        let _ = setlocale(LocaleCategory::LcAll, "C");
    }
}
