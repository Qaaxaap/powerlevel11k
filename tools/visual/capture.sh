#!/usr/bin/env bash
# Capture the reference prompt in a real terminal for every shell the engine proxies.
#
# Each run uses the same fixed repository, HOME and theme, so the only variable is the shell.
# The frames land in OUT/<shell>/p11k-<shell>-000.png and compare.sh checks them against the
# committed reference frames.
#
# Environment (set by the Nix wrapper):
#   P11K_VISUAL_REFERENCE   the theme file
#   P11K_VISUAL_KITTY_CONF  the kitty configuration
#   P11K_VISUAL_RECORD_TUI  the record-tui executable
#
# Usage:
#   capture.sh --out DIR --p11k BIN [--shells "zsh bash fish pwsh"] [--size WxH]

set -euo pipefail

# The caller's environment is cleared (--ignore-environment), but git still reads the system and
# user configuration; pin both so the commit and the repository state do not depend on the host.
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null

OUT="./render-check"
P11K=""
SHELLS="zsh bash fish pwsh"
SIZE=""

die() {
  echo "capture: $*" >&2
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --p11k) P11K="$2"; shift 2 ;;
    --shells) SHELLS="$2"; shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$P11K" ] || die "--p11k is required"
[ -x "$P11K" ] || die "p11k binary is not executable: $P11K"
P11K=$(cd "$(dirname "$P11K")" && pwd)/$(basename "$P11K")
: "${P11K_VISUAL_REFERENCE:?P11K_VISUAL_REFERENCE is not set}"
: "${P11K_VISUAL_KITTY_CONF:?P11K_VISUAL_KITTY_CONF is not set}"
: "${P11K_VISUAL_RECORD_TUI:?P11K_VISUAL_RECORD_TUI is not set}"

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

for shell in $SHELLS; do
  run="$OUT/run-$shell.sh"
  cat > "$run" <<EOF
#!/bin/sh
export HOME='$HOME_DIR'
export P11K_USER_PROFILE='$HOME_DIR/empty.ps1'
cd '$REPO'
exec '$P11K' --shell $shell --config '$P11K_VISUAL_REFERENCE'
EOF
  chmod +x "$run"

  mkdir -p "$OUT/$shell"
  size_args=()
  [ -n "$SIZE" ] && size_args=(--size "$SIZE")
  # pwsh starts its runtime and PSReadLine before the prompt exists; the other shells are ready
  # almost immediately. The stable-frame wait is the real gate, this is only the lower bound.
  min_wait="3"
  [ "$shell" = "pwsh" ] && min_wait="10"
  bash "$P11K_VISUAL_RECORD_TUI" \
    --out "$OUT/$shell" \
    "${size_args[@]}" \
    --kitty-conf "$P11K_VISUAL_KITTY_CONF" \
    --min-wait "$min_wait" \
    --title "p11k-$shell" \
    --type "sleep 1" \
    --type-wait 2 \
    -- "$run"
done

echo "capture: frames under $OUT" >&2
