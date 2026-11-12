//! Terminal output helpers and the generic question skeletons.

use std::io::{self, Write};

use crate::config::Config;
use crate::i18n::t;

use super::Answer;
use super::Control;
use super::input::{Input, key};
use super::preview::preview_lines;

pub(super) fn clear(out: &mut dyn Write) {
    let _ = out.write_all(b"\x1b[2J\x1b[H");
    let _ = out.flush();
}

/// Write one line (raw terminal has OPOST off, so `\n` does not return to column 0; `\r` is added explicitly).
pub(super) fn line(out: &mut dyn Write, s: &str) -> io::Result<()> {
    out.write_all(s.as_bytes())?;
    out.write_all(b"\r\n")?;
    out.flush()
}

/// Pick 1 of n, rendering each option's live prompt preview below it (aligns with p10k
/// wizard's add_prompt: every entry carries a full prompt under the current config).
pub(super) fn ask_choice_preview(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    options: &[&str],
    preview: impl Fn(usize) -> Vec<String>,
) -> Answer<usize> {
    loop {
        clear(out);
        // The title and option labels are msgids, translated here in one place; preview lines
        // are real prompts rendered from the config and do not go through gettext.
        line(out, &t(title))?;
        line(out, "")?;
        for (i, label) in options.iter().enumerate() {
            line(out, &format!("({})  {}", i + 1, t(label)))?;
            for l in preview(i) {
                line(out, &l)?;
            }
            line(out, "")?;
        }
        line(out, &format!("(r)  {}", t("Restart from the beginning.")))?;
        line(out, &format!("(q)  {}", t("Quit")))?;
        match key(input)? {
            b'q' => return Err(Control::Quit),
            b'r' => return Err(Control::Restart),
            k @ b'1'..=b'9' => {
                let i = (k - b'1') as usize;
                if i < options.len() {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
}

/// Previewed-question skeleton: clone the current config → apply the i-th option → render the
/// preview; on selection apply the same edit to the real config (preview and result share one
/// source and cannot drift).
pub(super) fn ask_apply(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    options: &[&str],
    cfg: &mut Config,
    apply: impl Fn(&mut Config, usize),
) -> Answer<()> {
    let i = {
        let base = cfg.clone();
        ask_choice_preview(out, input, title, options, |i| {
            let mut c = base.clone();
            apply(&mut c, i);
            preview_lines(&c)
        })?
    };
    apply(cfg, i);
    Ok(())
}

/// Yes/no question; when `sample` is present, a line with the glyph sample is shown before the
/// options. `title`, `yes` and `no` are msgids.
pub(super) fn ask_yn(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> Answer<bool> {
    ask_yn_title(out, input, &t(title), yes, no, sample)
}

/// Like [`ask_yn`] with an already translated title. Dynamic titles (an overwrite question with a
/// path) go through this variant so gettext sees the text once.
pub(super) fn ask_yn_title(
    out: &mut dyn Write,
    input: &mut dyn Input,
    title: &str,
    yes: &str,
    no: &str,
    sample: Option<&str>,
) -> Answer<bool> {
    loop {
        clear(out);
        line(out, title)?;
        if let Some(s) = sample {
            line(out, "")?;
            // The glyph sample is not text; emit it verbatim.
            line(out, s)?;
        }
        line(out, "")?;
        line(out, &format!("(y)  {}", t(yes)))?;
        line(out, "")?;
        line(out, &format!("(n)  {}", t(no)))?;
        line(out, &format!("(r)  {}", t("Restart from the beginning.")))?;
        line(out, &format!("(q)  {}", t("Quit")))?;
        match key(input)? {
            b'q' => return Err(Control::Quit),
            b'r' => return Err(Control::Restart),
            b'y' => return Ok(true),
            b'n' => return Ok(false),
            _ => {}
        }
    }
}

pub(super) fn preview(out: &mut dyn Write, cfg: &Config) -> io::Result<()> {
    clear(out);
    line(out, &t("Preview (current directory + exit code):"))?;
    line(out, "")?;
    for l in preview_lines(cfg) {
        line(out, &l)?;
    }
    line(out, "")?;
    Ok(())
}
