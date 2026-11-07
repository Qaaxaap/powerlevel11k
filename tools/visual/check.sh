#!/usr/bin/env bash
# Run the visual render check.
#
# `just render-check` builds the debug binary first, in the default devShell, so cargo uses the
# normal source and target caches. This script runs in the `visual` devShell entered with
# `--ignore-environment`: the shell provides the compositor, terminal, image tools, fonts, bats
# and the four shells, and the caller's environment is not visible.
#
# Each shell is a bats file. The file has one test per scene and size. `bats --jobs N`
# captures the frames in parallel and reports one TAP result per frame.
#
# Usage: check.sh [--update] [-j N]
#
#   --update   Capture each frame into tools/visual/expected-pending/<arch>/. The script
#              does not compare the frames. Review the images, then run `just render-accept`
#              to make them the committed references.
#   -j/--jobs N  Bats parallelism (default 4).

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT="$REPO_ROOT/.cache/render-check"
P11K="$REPO_ROOT/target/debug/p11k"
UPDATE=0
JOBS=4

usage() {
  sed -n '2,24p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p' >&2
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --update) UPDATE=1; shift ;;
    -j|--jobs) JOBS="${2:-}"; shift 2 ;;
    -j*) JOBS="${1#-j}"; shift ;;
    --jobs=*) JOBS="${1#--jobs=}"; shift ;;
    -h|--help) usage 0 ;;
    *) echo "check: unknown argument: $1" >&2; usage 2 ;;
  esac
done

case "$JOBS" in
  ''|*[!0-9]*) echo "check: --jobs needs a positive integer, got '$JOBS'" >&2; exit 2 ;;
esac
[ "$JOBS" -gt 0 ] || { echo "check: --jobs needs a positive integer, got '$JOBS'" >&2; exit 2; }

bash "$REPO_ROOT/tools/visual/prepare.sh" --out "$OUT" --p11k "$P11K"

# `bats --jobs` runs through GNU parallel, which needs HOME; the visual devShell is entered with
# --ignore-environment, so nothing provides it.
export HOME="${HOME:-/tmp}"

export P11K_VISUAL_OUT="$OUT"
if [ "$UPDATE" = "1" ]; then
  PENDING="$REPO_ROOT/tools/visual/expected-pending"
  rm -rf "$PENDING"
  mkdir -p "$PENDING"
  export P11K_VISUAL_EXPECTED="$PENDING"
else
  export P11K_VISUAL_EXPECTED="$REPO_ROOT/tools/visual/expected"
fi
export P11K_VISUAL_UPDATE="$UPDATE"

bats --jobs "$JOBS" --print-output-on-failure "$REPO_ROOT/tools/visual/tests"

if [ "$UPDATE" = "1" ]; then
  echo "check: pending frames are under $PENDING/<arch>/" >&2
  echo "check: review every image and compare it with the intended layout, then run 'just render-accept'" >&2
fi
