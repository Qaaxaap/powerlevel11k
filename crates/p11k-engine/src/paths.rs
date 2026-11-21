//! Filesystem locations the engine writes to.

use std::path::PathBuf;

/// `$XDG_STATE_HOME/p11k`, or `$HOME/.local/state/p11k` when the variable is unset or not
/// absolute. Falls back to a per-uid directory under the temporary directory when neither is
/// usable, so the engine still starts with an empty environment.
pub fn state_home() -> PathBuf {
    state_home_from(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::temp_dir(),
    )
}

/// The choice [`state_home`] makes, with the environment passed in so a test can drive it.
///
/// The per-uid name keeps two users from sharing one state directory. It does not make the
/// directory private: another local user can still pre-create the predictable path, and the
/// files carry the default umask.
fn state_home_from(xdg: Option<PathBuf>, home: Option<PathBuf>, temp: PathBuf) -> PathBuf {
    let base = xdg
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/state")))
        .unwrap_or_else(|| {
            // SAFETY: `getuid` takes no arguments and cannot fail.
            let uid = unsafe { libc::getuid() };
            temp.join(format!("p11k-{uid}"))
        });
    base.join("p11k")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_xdg_state_home_wins() {
        assert_eq!(
            state_home_from(
                Some(PathBuf::from("/var/state")),
                Some(PathBuf::from("/home/user")),
                PathBuf::from("/tmp"),
            ),
            PathBuf::from("/var/state/p11k")
        );
    }

    #[test]
    fn a_relative_xdg_state_home_falls_back_to_home() {
        assert_eq!(
            state_home_from(
                Some(PathBuf::from("relative")),
                Some(PathBuf::from("/home/user")),
                PathBuf::from("/tmp"),
            ),
            PathBuf::from("/home/user/.local/state/p11k")
        );
    }

    #[test]
    fn an_empty_environment_uses_a_per_user_temp_directory() {
        let got = state_home_from(None, None, PathBuf::from("/tmp"));
        // SAFETY: `getuid` takes no arguments and cannot fail.
        let uid = unsafe { libc::getuid() };
        assert_eq!(got, PathBuf::from(format!("/tmp/p11k-{uid}/p11k")));
    }
}
