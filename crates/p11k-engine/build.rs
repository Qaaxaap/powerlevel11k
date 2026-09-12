//! Build time: compile `po/*.po` into `.mo` with `msgfmt` and inject `P11K_LOCALEDIR`.
//!
//! At runtime `i18n::init` uses it to call `bindtextdomain`; when installed on the system
//! the same-named environment variable can point at `/usr/share/locale` and the like.
//! A packager that installs the catalogs elsewhere sets `P11K_LOCALEDIR` at build time to
//! the final prefix, so the binary does not carry a build directory that is about to vanish;
//! the catalogs still land in `$OUT_DIR` for running straight out of `target/`.
//! Without `msgfmt` (or without po files) it only warns, never errors: the program still
//! runs, it just does no translation (gettext returns the msgid when it finds none).

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let po_dir = manifest.join("po");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mo_dir = out_dir.join("locale");
    let embedded = match std::env::var("P11K_LOCALEDIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => mo_dir.clone(),
    };

    println!("cargo:rerun-if-changed=po");
    println!("cargo:rerun-if-env-changed=P11K_LOCALEDIR");
    println!("cargo:rustc-env=P11K_LOCALEDIR={}", embedded.display());

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
        let target_dir = mo_dir.join(lang).join("LC_MESSAGES");
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            println!("cargo:warning=cannot create {}: {e}", target_dir.display());
            continue;
        }
        let mo = target_dir.join("p11k.mo");
        println!("cargo:rerun-if-changed={}", path.display());
        match Command::new(&msgfmt).arg("-o").arg(&mo).arg(&path).output() {
            Ok(out) if out.status.success() => {}
            Ok(out) => println!(
                "cargo:warning=msgfmt failed for {}: {}",
                lang,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => {
                println!("cargo:warning=cannot run {msgfmt} ({e}); building without translations")
            }
        }
    }
}
