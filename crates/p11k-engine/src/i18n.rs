//! Thin gettext wrapper: sources carry English msgids, translated at runtime from
//! `po/<lang>.po` for the current locale.
//!
//! English is the default (gettext returns the msgid unchanged when no translation
//! exists); Chinese comes from `po/zh_CN.po`. The language is chosen by the standard
//! environment variables (`LANG` / `LC_ALL` / `LANGUAGE`), e.g. `LANG=zh_CN.UTF-8`.
//! The catalog directory defaults to the build-time `$OUT_DIR/locale` and can be
//! overridden with `P11K_LOCALEDIR` (`/usr/share/locale` for a system install).

use std::path::{Path, PathBuf};
use std::sync::Once;

use gettextrs::{LocaleCategory, bindtextdomain, gettext, setlocale, textdomain};

static INIT: Once = Once::new();

/// Bind the catalog directory. Call once per process (idempotent).
pub fn init() {
    INIT.call_once(|| {
        // An empty string means: take the locale from the environment.
        let _ = setlocale(LocaleCategory::LcAll, "");
        let _ = bindtextdomain("p11k", catalog_dir());
        let _ = textdomain("p11k");
    });
}

/// Where the catalogs are. `P11K_LOCALEDIR` wins, then the directory baked in at build time.
///
/// A release binary is built for `/usr/share/locale`, which exists on the target machine
/// whether or not p11k was installed there — so an empty directory has to fall through to
/// the catalogs shipped beside the executable (`<prefix>/share/locale`), which is what an
/// unpacked tarball looks like. Finding nothing just means no translation.
fn catalog_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("P11K_LOCALEDIR") {
        return PathBuf::from(dir);
    }
    let compiled = PathBuf::from(env!("P11K_LOCALEDIR"));
    if has_catalog(&compiled) {
        return compiled;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(beside) = exe.parent().and_then(Path::parent)
    {
        let beside = beside.join("share/locale");
        if has_catalog(&beside) {
            return beside;
        }
    }
    compiled
}

/// Whether `dir` holds a `p11k.mo` for at least one language.
fn has_catalog(dir: &Path) -> bool {
    let Ok(langs) = std::fs::read_dir(dir) else {
        return false;
    };
    langs
        .flatten()
        .any(|lang| lang.path().join("LC_MESSAGES/p11k.mo").is_file())
}

/// Translate one msgid. Returns the original text when no translation exists.
pub fn t(s: &str) -> String {
    gettext(s)
}

/// Mark a string as a msgid without translating it.
///
/// For call sites that take the text now and translate it later in one place (the
/// wizard's question titles and options: the call site passes only the literal,
/// and `ask_choice_preview`/`ask_yn` translate). With this, `xgettext
/// --keyword=msgid` picks those literals up into `po/p11k.pot`.
pub const fn msgid(s: &'static str) -> &'static str {
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_detection_needs_the_mo_file() {
        // The fallback must not accept a directory that merely exists: `/usr/share/locale`
        // is present on most systems and holds no p11k catalog.
        let root = std::env::temp_dir().join(format!("p11k-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!has_catalog(&root), "a missing directory has no catalog");
        std::fs::create_dir_all(root.join("zh_CN/LC_MESSAGES")).unwrap();
        assert!(!has_catalog(&root), "an empty language directory has none");
        std::fs::write(root.join("zh_CN/LC_MESSAGES/p11k.mo"), b"").unwrap();
        assert!(has_catalog(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Source files carrying text (the same set tools/i18n.sh scans; i18n.rs itself
    /// does not count, it holds only wrappers, no text).
    const SOURCES: &[(&str, &str)] = &[
        ("main.rs", include_str!("main.rs")),
        ("config.rs", include_str!("config.rs")),
        ("render.rs", include_str!("render.rs")),
        ("presets.rs", include_str!("presets.rs")),
        ("wizard.rs", include_str!("wizard.rs")),
    ];

    /// Parse .po/.pot: pull out every msgid (including continuation chunks), skipping the empty header msgid.
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
                // Continuation chunk of the previous msgid (msgstr continuations never
                // reach here: the `msgstr` line itself starts with `msgstr` and clears
                // cur first).
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

    /// Scan the sources for `t("…")` and `msgid("…")`.
    ///
    /// Strip `//` comments line by line and join the rest into one character stream
    /// before searching for the markers: rustfmt wraps long text, so a newline and
    /// indentation may follow `t(`/`msgid(` — only the string literal right after
    /// matters. A marker must be preceded by a separator so that `format!(` and
    /// `gettext(` are not mistaken for one.
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
        assert_eq!(gettext("Restart from the beginning."), "重新开始。");
        // A msgid with no catalog entry is returned unchanged (English is the default language).
        let unknown = String::from("no such msgid");
        assert_eq!(gettext(&unknown), unknown);

        let _ = setlocale(LocaleCategory::LcAll, "C");
    }
}
