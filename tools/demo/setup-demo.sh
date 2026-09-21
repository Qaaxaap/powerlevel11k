#!/bin/bash
# Build the demo environment used to record the README assets.
#
#   REAL_HOME=<dir>  the real home, for the cargo/rustup symlinks (default: $HOME)
#   HOME_DIR=<dir>   the isolated home the recording runs in (default: ./home)
#
# Creates a miniature Rust workspace under $HOME_DIR/dev/p11k whose git state has
# one of everything the vcs segment can show: a branch ahead of and behind its
# upstream, a stash, staged / unstaged / untracked changes. Its test suite fails
# after a few seconds, so `cargo test` also drives the status and
# command_execution_time segments.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
HOME_DIR=${HOME_DIR:-$HERE/home}
REPO=$HOME_DIR/dev/p11k
REAL_HOME=${REAL_HOME:-$HOME}

rm -rf "$HOME_DIR"
mkdir -p "$HOME_DIR/.config"

# cargo is needed to record the execution-time part of the demo; keep the
# toolchain reachable from the isolated home when it is there.
for d in .cargo .rustup; do
    if [ -e "$REAL_HOME/$d" ]; then
        ln -s "$REAL_HOME/$d" "$HOME_DIR/$d"
    fi
done

cat > "$HOME_DIR/.zshrc" <<'RC'
# The engine only needs the shell itself to stay out of the way.
PROMPT_EOL_MARK=''
unsetopt beep
RC

cat > "$HOME_DIR/demo.fish" <<'RC'
set -g fish_greeting ''
RC

mkdir -p "$REPO"
cd "$REPO"
git init -q -b main
git config user.email demo@p11k.invalid
git config user.name "p11k demo"
git config commit.gpgsign false

cat > Cargo.toml <<'TOML'
[workspace]
resolver = "2"
members = ["crates/*"]
TOML

cat > .gitignore <<'IGN'
/target
IGN

mkdir -p crates/p11k-engine/src/render/segments crates/p11k-gitstatus/src

cat > crates/p11k-engine/Cargo.toml <<'TOML'
[package]
name = "p11k-engine"
version = "0.1.0"
edition = "2021"
TOML

cat > crates/p11k-engine/src/lib.rs <<'RS'
//! Stand-in project: the recording looks at the prompt, never at the code.

pub mod render;

/// Fibonacci, recursively — slow enough to be a plausible benchmark target.
pub fn fib(n: u64) -> u64 {
    match n {
        0 | 1 => n,
        _ => fib(n - 1) + fib(n - 2),
    }
}

#[cfg(test)]
mod tests {
    use super::fib;
    use std::time::Duration;

    #[test]
    fn fib_matches_its_table() {
        std::thread::sleep(Duration::from_millis(3200));
        assert_eq!(fib(10), 56);
    }
}
RS

cat > crates/p11k-engine/src/render/mod.rs <<'RS'
pub mod segments;
RS

cat > crates/p11k-engine/src/render/segments/mod.rs <<'RS'
pub fn dir() -> &'static str {
    "dir"
}

pub fn vcs() -> &'static str {
    "vcs"
}
RS

cat > crates/p11k-gitstatus/Cargo.toml <<'TOML'
[package]
name = "p11k-gitstatus"
version = "0.1.0"
edition = "2021"
TOML

cat > crates/p11k-gitstatus/src/lib.rs <<'RS'
pub mod scan;
RS

cat > crates/p11k-gitstatus/src/scan.rs <<'RS'
pub fn fast() -> bool {
    true
}
RS

git add -A
git commit -qm "feat: add the engine skeleton"

# A second commit, so the branch is ahead of its upstream.
printf '\npub const NAME: &str = "demo";\n' >> crates/p11k-engine/src/lib.rs
git add -A
git commit -qm "feat: name the workspace"

# Upstream point: a sibling commit the local branch does not have -> behind 1.
git remote add origin https://github.com/p11k/demo.git
git branch -q upstream-base HEAD~1
git checkout -q upstream-base
printf 'fn unused() {}\n' >> crates/p11k-engine/src/lib.rs
git add -A
git commit -qm "chore: work on upstream"
git update-ref refs/remotes/origin/main HEAD
git checkout -q main
git branch -qD upstream-base
git config branch.main.remote origin
git config branch.main.merge refs/heads/main

# A stash, then the three working-tree states.
printf '\n// scratch\n' >> crates/p11k-engine/src/lib.rs
git stash -q
printf '\n// work in progress\n' >> crates/p11k-engine/src/lib.rs
printf 'pub fn helper() {}\n' > crates/p11k-gitstatus/src/helper.rs
git add crates/p11k-gitstatus/src/helper.rs
printf 'notes\n' > NOTES.md
printf 'scratch\n' > scratch.txt
printf 'log\n' > tmp.log

echo "demo repo at $REPO"
git -C "$REPO" status --short --branch
