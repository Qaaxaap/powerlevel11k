#!/usr/bin/env bash
# Compare the captured frames with the committed reference frames.
#
# Each shell has its own reference frame. The header is the same, but the input line, the echo of
# a typed command and the shell's redraw behaviour differ, so one shared reference would fail for
# every shell that produced it from a different shell. The comparison decodes the PNG images; a
# file hash would also cover the encoder and the metadata, which do not affect the screen.
#
# `odiff --aa` ignores antialiased pixels. The hints and the raster rounding of the GPU and the
# CPU can differ between machines by one color level at glyph edges; those pixels are not a
# rendering defect. `--threshold 0.01` ignores a color difference that is 1% or less; a layout,
# text, color or position difference is larger and fails.
#
# Usage:
#   compare.sh --out DIR [--expected DIR] --frames "zsh-700x450 bash-700x450" [--update]
#
#   --frames LIST   Frame names. The script compares OUT/<frame>/p11k-<frame>-000.png with
#                   EXPECTED/<arch>/<frame>.png.
#   --expected DIR  reference directory in the working tree (default: tools/visual/expected).
#                   It stays a working-tree path on purpose: --update writes the new frames into
#                   the checkout, and the Nix store copy is read-only.
#   --update        Copy each capture over its reference frame. Review every image before
#                   committing it: the update command cannot decide whether a change is intended.

set -euo pipefail

OUT="./render-check"
EXPECTED="./tools/visual/expected"
FRAMES=""
UPDATE="0"

die() {
  echo "compare: $*" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --expected) EXPECTED="$2"; shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --update) UPDATE="1"; shift ;;
    -h|--help) sed -n '2,21p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

: "${P11K_VISUAL_ARCH:?P11K_VISUAL_ARCH is not set}"

[ -d "$OUT" ] || die "capture directory does not exist: $OUT"
EXPECTED_DIR="$EXPECTED/$P11K_VISUAL_ARCH"
mkdir -p "$EXPECTED_DIR"

# Run odiff for one frame. The exit code is the result: 0 means the frame matches, 22 means
# pixel differences, and 21 means the image dimensions differ. Any other code is an error.
# With --parsable-stdout, a pixel difference prints "<different pixels>;<percentage>".
compare_frame() {
  local frame=$1
  local actual="$OUT/$frame/p11k-$frame-000.png"
  local reference="$EXPECTED_DIR/$frame.png"
  if [ ! -f "$actual" ]; then
    echo "compare: missing capture for $frame: $actual" >&2
    return 2
  fi
  if [ ! -f "$reference" ]; then
    echo "compare: missing reference for $frame: $reference" >&2
    echo "compare: run with --update, review the image, then commit it" >&2
    return 2
  fi
  local out rc
  if out=$(odiff --aa --threshold 0.01 --fail-on-layout --parsable-stdout \
    --diff-color '#ff0000' "$actual" "$reference" "$OUT/diff-$frame.png" 2>&1); then
    rc=0
  else
    rc=$?
  fi
  case "$rc" in
    0)
      echo "compare: $frame ok"
      return 0
      ;;
    22)
      local pixels=${out%%;*}
      case "$pixels" in
        ''|*[!0-9]*) pixels=$out ;;
      esac
      echo "compare: $frame differs by $pixels pixels" >&2
      if [ ! -s "$OUT/diff-$frame.png" ]; then
        echo "compare: $frame: the diff image was not written to $OUT/diff-$frame.png" >&2
        return 2
      fi
      return 1
      ;;
    21)
      echo "compare: $frame: the layout differs, the image dimensions do not match" >&2
      return 1
      ;;
    *)
      echo "compare: $frame: the comparison failed (exit $rc): $out" >&2
      return 2
      ;;
  esac
}

if [ "$UPDATE" = "1" ]; then
  for frame in $FRAMES; do
    actual="$OUT/$frame/p11k-$frame-000.png"
    [ -f "$actual" ] || die "--update needs the capture for $frame: $actual"
    cp "$actual" "$EXPECTED_DIR/$frame.png"
    echo "compare: updated $EXPECTED_DIR/$frame.png"
  done
  exit 0
fi

status=0
for frame in $FRAMES; do
  if compare_frame "$frame"; then
    :
  else
    status=1
  fi
done

exit "$status"
