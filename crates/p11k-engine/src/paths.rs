//! Filesystem locations the engine writes to.

use std::path::PathBuf;

/// `$XDG_STATE_HOME/p11k`, or `$HOME/.local/state/p11k` when the variable is unset or not
/// absolute. Falls back to the temporary directory when neither is usable, so the engine still
/// starts with an empty environment.
pub fn state_home() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|h| h.join(".local/state"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("p11k")
}
