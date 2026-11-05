#!/usr/bin/env bash
# Run the visual render check.
#
# `just render-check` builds the debug binary first, in the default devShell, so cargo uses the
# normal source and target caches. This script runs in the `visual` devShell entered with
# `--ignore-environment`: the shell provides the compositor, terminal, image tools, fonts, bats
# and the four shells, and the caller's environment is not visible.
#
# The four shells are four bats test files; `bats --jobs 4` captures them in parallel and reports
# one TAP result per test.
#
# Usage: check.sh [--update]

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT="$REPO_ROOT/.cache/render-check"
P11K="$REPO_ROOT/target/debug/p11k"
UPDATE=0
if [ "${1:-}" = "--update" ]; then
  UPDATE=1
fi

bash "$REPO_ROOT/tools/visual/prepare.sh" --out "$OUT" --p11k "$P11K"

# `bats --jobs` runs through GNU parallel, which needs HOME; the visual devShell is entered with
# --ignore-environment, so nothing provides it.
export HOME="${HOME:-/tmp}"

export P11K_VISUAL_OUT="$OUT"
export P11K_VISUAL_EXPECTED="$REPO_ROOT/tools/visual/expected"
export P11K_VISUAL_UPDATE="$UPDATE"

bats --jobs 4 --print-output-on-failure "$REPO_ROOT/tools/visual/tests"
