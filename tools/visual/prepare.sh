#!/usr/bin/env bash
# Prepare the fixed repository, HOME and per-shell launcher scripts for the bats tests.
#
# The tests run in parallel with `bats --jobs`, so everything shared is created here once and is
# read-only afterwards; each shell only writes its own output directory.
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
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$P11K" ] || die "--p11k is required"
[ -x "$P11K" ] || die "p11k binary is not executable: $P11K"
P11K=$(cd "$(dirname "$P11K")" && pwd)/$(basename "$P11K")
: "${P11K_VISUAL_REFERENCE:?P11K_VISUAL_REFERENCE is not set}"

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# The directory in the prompt is the same on every machine and every shell. The repository is
# clean so vcs renders the branch instead of a change count.
REPO="/tmp/p11k-visual-repo"
rm -rf "$REPO"
mkdir -p "$REPO"
git init -q -b main "$REPO"
printf 'reference\n' > "$REPO/file.txt"
git -C "$REPO" add file.txt
git -C "$REPO" -c user.email=visual@example.invalid -c user.name="visual test" commit -qm init

HOME_DIR="$OUT/home"
rm -rf "$HOME_DIR"
mkdir -p "$HOME_DIR"
: > "$HOME_DIR/empty.ps1"

for shell in zsh bash fish pwsh; do
  run="$OUT/run-$shell.sh"
  cat > "$run" <<EOF
#!/bin/sh
export HOME='$HOME_DIR'
export P11K_USER_PROFILE='$HOME_DIR/empty.ps1'
cd '$REPO'
exec '$P11K' --shell $shell --config '$P11K_VISUAL_REFERENCE'
EOF
  chmod +x "$run"
done

echo "prepare: ready under $OUT" >&2
