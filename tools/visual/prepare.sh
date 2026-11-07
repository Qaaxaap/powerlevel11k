#!/usr/bin/env bash
# Prepare the fixed repositories, HOME and per-shell launcher scripts for the bats tests.
#
# The tests run in parallel with `bats --jobs`, so everything shared is created here once and is
# read-only afterwards; each shell only writes its own output directory.
#
# Two scenes:
#   reference  clean repository on main
#   dirty      repository with a tag, one stash, one staged, one unstaged and one untracked change
#
# Environment (set by the Nix wrapper):
#   P11K_VISUAL_REFERENCE  the theme file
#
# Usage:
#   prepare.sh --out DIR --p11k BIN

set -euo pipefail

# The caller's environment is cleared (--ignore-environment), but git still reads the system and
# user configuration; pin both so the commit and the repository state do not depend on the host.
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME="visual test"
export GIT_AUTHOR_EMAIL="visual@example.invalid"
export GIT_COMMITTER_NAME="visual test"
export GIT_COMMITTER_EMAIL="visual@example.invalid"

OUT="./render-check"
P11K=""

die() {
  echo "prepare: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --p11k) P11K="$2"; shift 2 ;;
    -h|--help) sed -n '2,17p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$P11K" ] || die "--p11k is required"
[ -x "$P11K" ] || die "p11k binary is not executable: $P11K"
P11K=$(cd "$(dirname "$P11K")" && pwd)/$(basename "$P11K")
: "${P11K_VISUAL_REFERENCE:?P11K_VISUAL_REFERENCE is not set}"

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# ---- reference scene: clean repository on main ----
REPO="/tmp/p11k-visual-repo"
rm -rf "$REPO"
mkdir -p "$REPO"
git init -q -b main "$REPO"
printf 'reference\n' > "$REPO/file.txt"
git -C "$REPO" add file.txt
git -C "$REPO" commit -qm init

# ---- dirty scene: one of every vcs state the header draws ----
DIRTY_REPO="/tmp/p11k-visual-repo-dirty"
rm -rf "$DIRTY_REPO"
mkdir -p "$DIRTY_REPO"
git init -q -b main "$DIRTY_REPO"
printf 'dirty\n' > "$DIRTY_REPO/file.txt"
git -C "$DIRTY_REPO" add file.txt
git -C "$DIRTY_REPO" commit -qm init
git -C "$DIRTY_REPO" tag v1
# Stash one change, then leave one change of each kind behind.
printf 'stashed\n' > "$DIRTY_REPO/file.txt"
git -C "$DIRTY_REPO" stash push -q -m "visual stash"
printf 'unstaged\n' > "$DIRTY_REPO/file.txt"
printf 'staged\n' > "$DIRTY_REPO/staged.txt"
git -C "$DIRTY_REPO" add staged.txt
printf 'untracked\n' > "$DIRTY_REPO/untracked.txt"

HOME_DIR="$OUT/home"
rm -rf "$HOME_DIR"
mkdir -p "$HOME_DIR"
: > "$HOME_DIR/empty.ps1"

for scene in reference dirty; do
  case "$scene" in
    reference) dir="$REPO" ;;
    dirty) dir="$DIRTY_REPO" ;;
  esac
  for shell in zsh bash fish pwsh; do
    run="$OUT/run-$shell-$scene.sh"
    cat > "$run" <<EOF
#!/bin/sh
export HOME='$HOME_DIR'
export P11K_USER_PROFILE='$HOME_DIR/empty.ps1'
cd '$dir'
exec '$P11K' --shell $shell --config '$P11K_VISUAL_REFERENCE'
EOF
    chmod +x "$run"
  done
done

echo "prepare: ready under $OUT" >&2
