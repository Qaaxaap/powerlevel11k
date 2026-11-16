//! Fluent-backed localisation.
//!
//! English is the source language (`locales/en.ftl`); `locales/zh-CN.ftl` carries the Chinese
//! translation. Both are embedded at build time. `P11K_LANG` overrides the language. Otherwise
//! `LANGUAGE` wins over the locale categories, like gettext: it is a colon-separated priority
//! list, and the first supported language in it wins. The locale categories are checked in the
//! order `LC_ALL`, `LC_MESSAGES`, `LANG`; the first one that carries a language tag decides, and
//! an unsupported language falls back to English.
//!
//! `P11K_LOCALEDIR` can point at a directory of `.ftl` files to override the embedded
//! resources. The directory must carry `en.ftl` as the fallback language, or the external
//! catalogue is rejected with a warning and the embedded one stays in use. Tests and
//! translators use it; a release binary needs no data files.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use fluent_bundle::FluentValue;
use fluent_templates::{ArcLoader, LanguageIdentifier, Loader, static_loader};

static_loader! {
    static EMBEDDED = {
        locales: "./locales",
        fallback_language: "en",
        // Fluent inserts Unicode isolating marks around variables by default. They are
        // invisible, but they count as characters in a terminal: a copied prompt or a width
        // calculation would carry them. The prompt is plain text, so turn them off.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

/// English, the source language and the fallback for every missing translation.
static EN: LanguageIdentifier = fluent_templates::langid!("en");

/// The shipped translation besides English.
static ZH_CN: LanguageIdentifier = fluent_templates::langid!("zh-CN");

static LANGUAGE: OnceLock<LanguageIdentifier> = OnceLock::new();

/// A runtime resource directory from `P11K_LOCALEDIR`, when it has usable files.
static EXTERNAL: OnceLock<Option<ArcLoader>> = OnceLock::new();

/// Resolve the language and load `P11K_LOCALEDIR` if it is set. Call once per process.
pub fn init() {
    let _ = LANGUAGE.set(detect_language());
    let _ = EXTERNAL.set(load_external());
}

/// The language selected at startup (English until [`init`] runs).
pub fn language() -> LanguageIdentifier {
    LANGUAGE.get().cloned().unwrap_or_else(|| EN.clone())
}

/// Translate one message. A message id is the key in the `.ftl` file.
pub fn t(id: &str) -> String {
    lookup(id, None)
}

/// Translate one message with variables, e.g. `t_args("cli-unknown-shell", &[("shell", "x")])`.
pub fn t_args(id: &str, args: &[(&str, &str)]) -> String {
    let mut values: HashMap<Cow<'static, str>, FluentValue<'_>> = HashMap::new();
    for (key, value) in args {
        values.insert(
            Cow::Owned((*key).to_owned()),
            FluentValue::String(Cow::Borrowed(*value)),
        );
    }
    lookup(id, Some(&values))
}

fn lookup(id: &str, args: Option<&HashMap<Cow<'static, str>, FluentValue<'_>>>) -> String {
    let language = language();
    if let Some(loader) = EXTERNAL.get().and_then(|loader| loader.as_ref()) {
        return loader.lookup_complete(&language, id, args);
    }
    EMBEDDED.lookup_complete(&language, id, args)
}

/// Whether `dir` is a usable external catalogue. The fallback language has to be present:
/// without `en.ftl` a language that has no translation would render raw message ids.
fn external_catalogue_ready(dir: &Path) -> bool {
    dir.join("en.ftl").is_file()
}

fn load_external() -> Option<ArcLoader> {
    let dir = PathBuf::from(std::env::var_os("P11K_LOCALEDIR")?);
    if !external_catalogue_ready(&dir) {
        eprintln!(
            "p11k: P11K_LOCALEDIR {} has no en.ftl; using the embedded catalogue",
            dir.display()
        );
        return None;
    }
    match ArcLoader::builder(&dir, EN.clone())
        .customize(|bundle| bundle.set_use_isolating(false))
        .build()
    {
        Ok(loader) => Some(loader),
        Err(error) => {
            eprintln!(
                "p11k: cannot load the external catalogue from {}: {error}; using the embedded catalogue",
                dir.display()
            );
            None
        }
    }
}

/// Pick the language from the environment.
fn detect_language() -> LanguageIdentifier {
    detect_language_with(|key| std::env::var_os(key).map(|v| v.to_string_lossy().into_owned()))
}

/// [`detect_language`] with an injectable environment (tests).
fn detect_language_with(var: impl Fn(&str) -> Option<String>) -> LanguageIdentifier {
    // P11K_LANG is an explicit override: an unsupported tag falls back to English without
    // consulting the locale.
    if let Some(raw) = var("P11K_LANG")
        && let Some(language) = parse_language(&raw)
    {
        return if supported(&language) {
            language
        } else {
            EN.clone()
        };
    }
    // LANGUAGE is gettext's priority list and wins over the locale categories. A list without
    // a supported language falls through to the locale.
    if let Some(raw) = var("LANGUAGE")
        && let Some(language) = first_supported(&raw)
    {
        return language;
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Some(raw) = var(key) else {
            continue;
        };
        if raw.trim().is_empty() {
            continue;
        }
        let Some(language) = parse_language(&raw) else {
            continue;
        };
        return if supported(&language) {
            language
        } else {
            EN.clone()
        };
    }
    EN.clone()
}

/// Whether the shipped catalogue carries a translation for `language`.
fn supported(language: &LanguageIdentifier) -> bool {
    *language == EN || *language == ZH_CN
}

/// The first supported language in a gettext priority list, e.g. `zh_CN:en`.
fn first_supported(raw: &str) -> Option<LanguageIdentifier> {
    raw.split(':')
        .filter_map(parse_language_tag)
        .find(supported)
}

/// Parse a locale value. `LANGUAGE` and `P11K_LANG` are colon-separated priority lists, the
/// other variables hold one value. `C`/`POSIX`, an empty value and anything that does not parse
/// mean English.
fn parse_language(raw: &str) -> Option<LanguageIdentifier> {
    raw.split(':').find_map(parse_language_tag)
}

fn parse_language_tag(raw: &str) -> Option<LanguageIdentifier> {
    // Locale values carry an encoding and a modifier: zh_CN.UTF-8, sr_RS@latin.
    let tag = raw.split(['.', '@']).next()?.trim();
    if tag.is_empty() {
        return None;
    }
    let tag = tag.replace('_', "-");
    if tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
        return Some(EN.clone());
    }
    tag.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// Parse a simple `id = value` fluent file. All messages in this project are one-liners
    /// (with `#` comments), so a small scanner is enough and keeps the test independent of the
    /// Fluent parser used at build time.
    fn messages(text: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((id, value)) = line.split_once(" = ") else {
                panic!("not an `id = value` line: {line:?}");
            };
            assert!(
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "invalid message id: {id:?}"
            );
            out.insert(id.to_owned(), value.to_owned());
        }
        out
    }

    fn english() -> BTreeMap<String, String> {
        messages(include_str!("../locales/en.ftl"))
    }

    fn chinese() -> BTreeMap<String, String> {
        messages(include_str!("../locales/zh-CN.ftl"))
    }

    #[test]
    fn the_catalogues_define_the_same_messages() {
        let en = english();
        let zh = chinese();
        let missing: Vec<_> = en.keys().filter(|id| !zh.contains_key(*id)).collect();
        let stale: Vec<_> = zh.keys().filter(|id| !en.contains_key(*id)).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "locales/en.ftl and locales/zh-CN.ftl disagree\n  missing in zh-CN: {missing:?}\n  stale in zh-CN: {stale:?}"
        );
        assert!(en.len() > 70, "too few messages in en.ftl: {}", en.len());
        for (id, value) in &zh {
            assert!(!value.is_empty(), "empty translation for {id:?}");
        }
    }

    /// Collect the source files that call `t`/`t_args`; the i18n module itself only implements
    /// the lookup and holds the tests.
    fn sources() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk_sources(&root, &root, &mut files);
        files.sort();
        files
    }

    fn walk_sources(
        root: &std::path::Path,
        dir: &std::path::Path,
        files: &mut Vec<(String, String)>,
    ) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_sources(root, &path, files);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs")
                || path.file_name().is_some_and(|name| name == "i18n.rs")
            {
                continue;
            }
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            files.push((name, text));
        }
    }

    /// Every message id the sources reference: literal arguments of `t(...)`/`t_args(...)`
    /// (including a literal that starts on the line after the call), plus the bare
    /// `wizard-*`/`preset-*` ids the wizard passes to its question helpers and
    /// `PresetKind::title` returns.
    fn referenced_ids() -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        for (name, src) in sources() {
            let code = src
                .lines()
                .map(|line| match line.find("//") {
                    Some(at) => &line[..at],
                    None => line,
                })
                .collect::<Vec<_>>()
                .join("\n");
            for marker in ["t(", "t_args("] {
                let mut from = 0;
                while let Some(pos) = code[from..].find(marker) {
                    let at = from + pos;
                    let before = code[..at].chars().next_back();
                    from = at + marker.len();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(rest) = code[from..].trim_start().strip_prefix('"') else {
                        continue;
                    };
                    let end = rest
                        .find('"')
                        .unwrap_or_else(|| panic!("{name}: unterminated message id near {rest:?}"));
                    ids.insert(rest[..end].to_owned());
                }
            }
            // The question helpers take ids directly, and `PresetKind::title` returns one.
            for prefix in ["\"wizard-", "\"preset-"] {
                let mut from = 0;
                while let Some(pos) = code[from..].find(prefix) {
                    let start = from + pos + 1;
                    let rest = &code[start..];
                    let end = rest
                        .find('"')
                        .unwrap_or_else(|| panic!("{name}: unterminated message id near {rest:?}"));
                    ids.insert(rest[..end].to_owned());
                    from = start + end;
                }
            }
        }
        ids
    }

    #[test]
    fn the_sources_only_reference_defined_messages() {
        let en = english();
        let referenced = referenced_ids();
        assert!(
            referenced.len() > 60,
            "too few message ids scanned from sources: {referenced:?}"
        );
        let missing: Vec<_> = referenced
            .iter()
            .filter(|id| !en.contains_key(*id))
            .collect();
        let stale: Vec<_> = en.keys().filter(|id| !referenced.contains(*id)).collect();
        assert!(
            missing.is_empty(),
            "sources reference message ids that locales/en.ftl does not define: {missing:?}"
        );
        assert!(
            stale.is_empty(),
            "locales/en.ftl has messages that nothing references: {stale:?}"
        );
    }

    #[test]
    fn the_chinese_catalogue_translates_every_message() {
        // Fluent falls back to English for a missing message, so the payload matters: every
        // entry must be translated, even when the translation keeps the English word.
        let zh = chinese();
        let en = english();
        for id in [
            "wizard-yes",
            "wizard-no",
            "wizard-style-title",
            "wizard-restart",
            "wizard-quit",
            "engine-inner-shell-failed",
            "main-recursive-launch",
        ] {
            assert!(zh.contains_key(id), "missing zh translation for {id}");
            assert_ne!(zh[id], en[id], "untranslated zh entry for {id}");
        }
    }

    fn chinese_loader() -> ArcLoader {
        ArcLoader::builder(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales"),
            EN.clone(),
        )
        .customize(|bundle| bundle.set_use_isolating(false))
        .build()
        .expect("the locales directory parses")
    }

    #[test]
    fn chinese_messages_load_and_english_falls_back() {
        let loader = chinese_loader();
        let zh: LanguageIdentifier = fluent_templates::langid!("zh-CN");
        assert_eq!(loader.lookup(&zh, "wizard-yes"), "是");
        assert_eq!(
            loader.lookup(&zh, "main-recursive-launch"),
            "检测到递归启动（在 p11k 会话内再次启动 p11k）"
        );
        // A language with no bundle falls back to English.
        let fr: LanguageIdentifier = fluent_templates::langid!("fr");
        assert_eq!(loader.lookup(&fr, "wizard-yes"), "Yes.");
    }

    /// A directory without the fallback language would render raw message ids for languages
    /// that have no translation, so the external catalogue requires `en.ftl`.
    #[test]
    fn an_external_catalogue_needs_the_fallback_language() {
        let dir = std::env::temp_dir().join(format!("p11k-locale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the locale dir");
        assert!(!external_catalogue_ready(&dir));
        std::fs::write(dir.join("zh-CN.ftl"), "wizard-yes = 是\n").expect("write zh-CN");
        assert!(!external_catalogue_ready(&dir));
        std::fs::write(dir.join("en.ftl"), "wizard-yes = Yes\n").expect("write en");
        assert!(external_catalogue_ready(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn variables_are_formatted_without_isolating_marks() {
        let loader = chinese_loader();
        let zh: LanguageIdentifier = fluent_templates::langid!("zh-CN");
        let mut args = HashMap::new();
        args.insert(
            Cow::Borrowed("detail"),
            FluentValue::String(Cow::Borrowed("no such file")),
        );
        let text = loader.lookup_with_args(&zh, "cli-cannot-read-session-file", &args);
        assert_eq!(text, "无法读取会话文件：no such file");
        assert!(
            !text.contains('\u{2068}'),
            "isolating mark leaked: {text:?}"
        );
    }

    #[test]
    fn locale_values_parse_into_language_tags() {
        assert_eq!(
            parse_language_tag("zh_CN.UTF-8").map(|l| l.to_string()),
            Some("zh-CN".into())
        );
        assert_eq!(
            parse_language_tag("zh-Hans").map(|l| l.to_string()),
            Some("zh-Hans".into())
        );
        assert_eq!(
            parse_language_tag("sr_RS@latin").map(|l| l.to_string()),
            Some("sr-RS".into())
        );
        assert_eq!(
            parse_language_tag("C").map(|l| l.to_string()),
            Some("en".into())
        );
        assert_eq!(
            parse_language_tag("POSIX").map(|l| l.to_string()),
            Some("en".into())
        );
        assert_eq!(
            parse_language("fr_FR:de_DE").map(|l| l.to_string()),
            Some("fr-FR".into())
        );
        assert!(parse_language_tag("").is_none());
    }

    /// An environment backed by a fixed map.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    /// gettext precedence: `P11K_LANG` overrides, then `LANGUAGE`, then the locale categories.
    /// `LANGUAGE` is a priority list and skips entries the catalogue does not carry.
    #[test]
    fn language_precedence() {
        let zh = ZH_CN.clone();
        let en = EN.clone();
        // LANGUAGE wins over the locale categories.
        assert_eq!(
            detect_language_with(env(&[("LANGUAGE", "zh_CN"), ("LANG", "en_US.UTF-8")])),
            zh
        );
        // The list is walked: unsupported entries are skipped.
        assert_eq!(
            detect_language_with(env(&[("LANGUAGE", "fr_FR:zh_CN"), ("LANG", "en_US.UTF-8")])),
            zh
        );
        // A list without a supported language falls through to the locale.
        assert_eq!(
            detect_language_with(env(&[("LANGUAGE", "fr_FR"), ("LANG", "zh_CN.UTF-8")])),
            zh
        );
        // P11K_LANG is an explicit override and does not fall through when unsupported.
        assert_eq!(
            detect_language_with(env(&[
                ("P11K_LANG", "fr_FR"),
                ("LANGUAGE", "zh_CN"),
                ("LANG", "zh_CN.UTF-8"),
            ])),
            en
        );
        assert_eq!(
            detect_language_with(env(&[("P11K_LANG", "zh_CN"), ("LC_ALL", "C")])),
            zh
        );
        // An empty P11K_LANG is ignored.
        assert_eq!(
            detect_language_with(env(&[("P11K_LANG", ""), ("LANG", "zh_CN.UTF-8")])),
            zh
        );
        // LC_ALL overrides LC_MESSAGES and LANG; LC_MESSAGES overrides LANG.
        assert_eq!(
            detect_language_with(env(&[("LC_ALL", "C"), ("LANG", "zh_CN.UTF-8")])),
            en
        );
        assert_eq!(
            detect_language_with(env(&[("LC_MESSAGES", "zh_CN"), ("LANG", "en_US.UTF-8")])),
            zh
        );
        // Empty values are skipped and nothing set is English.
        assert_eq!(
            detect_language_with(env(&[("LC_ALL", ""), ("LANG", "zh_CN.UTF-8")])),
            zh
        );
        assert_eq!(detect_language_with(env(&[])), en);
    }
}
