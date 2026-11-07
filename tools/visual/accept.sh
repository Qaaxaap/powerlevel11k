#!/usr/bin/env bash
# Copy the reviewed pending reference frames into the committed expected set.
#
# `just render-update` writes captures to tools/visual/expected-pending/<arch>/. It never
# touches tools/visual/expected/. Review each pending image and compare it with the intended
# layout before you run this script. The frames are the reference. A wrong image accepts a wrong
# rendering without a warning.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
PENDING="$REPO_ROOT/tools/visual/expected-pending"
EXPECTED="$REPO_ROOT/tools/visual/expected"

[ -d "$PENDING" ] || {
  echo "accept: no pending frames; run 'just render-update' first" >&2
  exit 2
}

found=0
for dir in "$PENDING"/*/; do
  [ -d "$dir" ] || continue
  arch=$(basename "$dir")
  mkdir -p "$EXPECTED/$arch"
  for image in "$dir"*.png; do
    [ -f "$image" ] || continue
    cp "$image" "$EXPECTED/$arch/$(basename "$image")"
    found=1
  done
done

[ "$found" = "1" ] || {
  echo "accept: no pending PNG files under $PENDING" >&2
  exit 2
}

echo "accept: copied the pending frames into $EXPECTED"
rm -rf "$PENDING"
