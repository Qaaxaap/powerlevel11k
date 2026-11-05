#!/usr/bin/env bash
# Run the visual render check.
#
# `just render-check` builds the debug binary first, in the default devShell, so cargo uses the
# normal source and target caches. This script runs in the `visual` devShell entered with
# `--ignore-environment`: the shell provides the compositor, terminal, image tools, fonts and the
# four shells, and the caller's environment is not visible.
#
# Usage: check.sh [--update]

set -euo pipefail

OUT=".cache/render-check"
UPDATE=()
if [ "${1:-}" = "--update" ]; then
  UPDATE=(--update)
fi

bash tools/visual/capture.sh --p11k target/debug/p11k --out "$OUT"
bash tools/visual/compare.sh --out "$OUT" "${UPDATE[@]}"
