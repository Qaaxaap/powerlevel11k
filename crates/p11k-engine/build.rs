//! 构建期把 `po/*.po` 用 `msgfmt` 编成 `.mo`，并注入 `P11K_LOCALEDIR`。
//!
//! 运行时由 `i18n::init` 用它调 `bindtextdomain`；装到系统时可以用同名环境变量
//! 指到 `/usr/share/locale` 之类的目录。没有 `msgfmt`（或没有 po 文件）时只警告
//! 不报错：程序照常跑，只是不翻译（gettext 找不到翻译就返回 msgid 原文）。

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let po_dir = manifest.join("po");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let locale_dir = out_dir.join("locale");

    println!("cargo:rerun-if-changed=po");
    println!(
        "cargo:rustc-env=P11K_LOCALEDIR={}",
        locale_dir.display()
    );

    let Ok(entries) = std::fs::read_dir(&po_dir) else {
        println!("cargo:warning=no po/ directory, building without translations");
        return;
    };
    let msgfmt = std::env::var("MSGFMT").unwrap_or_else(|_| "msgfmt".into());
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "po") {
            continue;
        }
        let Some(lang) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let target_dir = locale_dir.join(lang).join("LC_MESSAGES");
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            println!("cargo:warning=cannot create {}: {e}", target_dir.display());
            continue;
        }
        let mo = target_dir.join("p11k.mo");
        println!("cargo:rerun-if-changed={}", path.display());
        match Command::new(&msgfmt)
            .arg("-o")
            .arg(&mo)
            .arg(&path)
            .output()
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => println!(
                "cargo:warning=msgfmt failed for {}: {}",
                lang,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => println!(
                "cargo:warning=cannot run {msgfmt} ({e}); building without translations"
            ),
        }
    }
}
