//! Serializing the finished theme and the small config edits the questions share.

use std::io::Write;

use crate::config::{Config, Element};
use crate::i18n::{msgid, t};

use super::Answer;
use super::input::Input;
use super::ui::{ask_yn_title, line};

pub(super) fn remove_segment(cfg: &mut Config, name: &str) {
    for row in cfg
        .layout
        .left
        .iter_mut()
        .chain(cfg.layout.right.iter_mut())
    {
        row.retain(|e| !matches!(e, Element::Seg(s) if s == name));
    }
}

pub(super) fn add_to_right(cfg: &mut Config, name: &str) {
    if cfg
        .layout
        .right
        .iter()
        .flatten()
        .any(|e| matches!(e, Element::Seg(s) if s == name))
    {
        return;
    }
    if cfg.layout.right.is_empty() {
        cfg.layout.right.push(Vec::new());
    }
    cfg.layout.right[0].push(Element::Seg(name.into()));
}

pub(super) fn set_prefix(cfg: &mut Config, name: &str, prefix: &str) {
    if let Some(s) = cfg.segments.get_mut(name) {
        s.prefix = Some(prefix.into());
    }
}

pub(super) fn write_config(out: &mut dyn Write, input: &mut dyn Input, cfg: &Config) -> Answer<()> {
    let path = crate::config::default_config_path();
    let p = path.display();
    if path.exists() {
        // p10k: `p11k config file already exists. Overwrite <path>?`
        let title = format!(
            "{} {p}?",
            t(msgid("p11k config file already exists. Overwrite"))
        );
        if !ask_yn_title(out, input, &title, msgid("Yes."), msgid("No."), None)? {
            return Err(super::Control::Quit);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, cfg.to_kdl())?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "p11k".into());
    line(out, "")?;
    // p10k: `New config: <path>.`
    line(out, &format!("{} {p}.", t("New config:")))?;
    line(out, "")?;
    line(out, &t("Add one of these to your shell rc:"))?;
    line(
        out,
        &format!("  zsh:  [[ -z \"$P11K_ENGINE\" ]] && exec {exe} --shell zsh --config {p}"),
    )?;
    line(
        out,
        &format!("  bash: [[ -z \"$P11K_ENGINE\" ]] && exec {exe} --shell bash --config {p}"),
    )?;
    line(
        out,
        &format!("  fish: if not set -q P11K_ENGINE; exec {exe} --shell fish --config {p}; end"),
    )?;
    // pwsh has no exec, so start the engine, wait for it to exit, then exit the outer pwsh. This line goes into $PROFILE.
    line(
        out,
        &format!(
            "  pwsh: if (-not $env:P11K_ENGINE) {{ & '{exe}' --shell pwsh --config '{p}'; exit }}"
        ),
    )?;
    line(out, "")?;
    line(
        out,
        &t(
            "The engine is the theme: remove any ZSH_THEME/theme setup first, otherwise they will stack.",
        ),
    )?;
    Ok(())
}
