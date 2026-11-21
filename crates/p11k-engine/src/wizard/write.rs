//! Serializing the finished theme and the small config edits the questions share.

use std::io::Write;

use crate::config::{Config, Element};
use crate::i18n::{t, t_args};

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
    if let Some(input) = &mut cfg.layout.input {
        input
            .left
            .retain(|e| !matches!(e, Element::Seg(s) if s == name));
        input
            .right
            .retain(|e| !matches!(e, Element::Seg(s) if s == name));
    }
}

pub(super) fn add_to_right(cfg: &mut Config, name: &str) {
    let in_header = cfg
        .layout
        .right
        .iter()
        .flatten()
        .any(|e| matches!(e, Element::Seg(s) if s == name));
    let in_input = cfg.layout.input.as_ref().is_some_and(|input| {
        input
            .right
            .iter()
            .any(|e| matches!(e, Element::Seg(s) if s == name))
    });
    if in_header || in_input {
        return;
    }
    if cfg.layout.right.is_empty() {
        // No header row: the row the user types on is the only row, so the segment joins its
        // right column.
        if let Some(input) = &mut cfg.layout.input {
            input.right.push(Element::Seg(name.into()));
            return;
        }
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
    let p = path.display().to_string();
    if path.exists() {
        // p10k: `p11k config file already exists. Overwrite <path>?`
        let title = t_args("wizard-overwrite", &[("path", &p)]);
        if !ask_yn_title(out, input, &title, "wizard-yes", "wizard-no", None)? {
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
    line(out, &t_args("wizard-new-config", &[("path", &p)]))?;
    line(out, "")?;
    line(out, &t("wizard-add-to-rc"))?;
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
    line(out, &t("wizard-engine-is-theme"))?;
    Ok(())
}
