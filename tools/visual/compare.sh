#!/usr/bin/env bash
# Compare the captured frames with the committed reference frames.
#
# Each shell has its own reference frame. The header is the same, but the input line, the echo of
# a typed command and the shell's redraw behaviour differ, so one shared reference would fail for
# every shell that produced it from a different shell. The comparison decodes the PNG images; a
# file hash would also cover the encoder and the metadata, which do not affect the screen.
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
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed -n '/^#/s/^# \{0,1\}//p'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

: "${P11K_VISUAL_ARCH:?P11K_VISUAL_ARCH is not set}"

[ -d "$OUT" ] || die "capture directory does not exist: $OUT"
EXPECTED_DIR="$EXPECTED/$P11K_VISUAL_ARCH"
mkdir -p "$EXPECTED_DIR"

# `magick compare -metric AE` exits 0 when the images are equal and 1 when they differ; the AE
# metric on stderr is the number of differing pixels. Any other outcome (unreadable file,
# mismatched sizes) is a comparison error, not a rendering difference.
ae_metric() {
  local a=$1 b=$2
  if metric=$(magick compare -metric AE "$a" "$b" null: 2>&1); then
    rc=0
  else
    rc=$?
  fi
  # AE prints "<absolute> (<normalized>)". The absolute part can carry a fractional part when
  # the images have an alpha channel; the integer part is the differing pixel count.
  raw="$metric"
  metric=${raw%% *}
  case "$metric" in
    ''|*[!0-9.]*)
      die "comparison failed for $a and $b: $raw"
      ;;
  esac
  metric=${metric%%.*}
  case "$metric" in
    ''|*[!0-9]*)
      die "comparison failed for $a and $b: $raw"
      ;;
  esac
  if [ "$rc" != "0" ] && [ "$rc" != "1" ]; then
    die "comparison failed for $a and $b (exit $rc, metric $raw)"
  fi
  if [ "$rc" = "0" ] && [ "$metric" != "0" ]; then
    die "inconsistent result for $a and $b: exit 0 with $raw differing pixels"
  fi
  if [ "$rc" = "1" ] && [ "$metric" = "0" ]; then
    die "inconsistent result for $a and $b: exit 1 with $raw differing pixels"
  fi
  printf '%s' "$metric"
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
  actual="$OUT/$frame/p11k-$frame-000.png"
  reference="$EXPECTED_DIR/$frame.png"
  if [ ! -f "$actual" ]; then
    echo "compare: missing capture for $frame: $actual" >&2
    status=1
    continue
  fi
  if [ ! -f "$reference" ]; then
    echo "compare: missing reference for $frame: $reference" >&2
    echo "compare: run with --update, review the image, then commit it" >&2
    status=1
    continue
  fi
  metric=$(ae_metric "$actual" "$reference")
  if [ "$metric" = "0" ]; then
    echo "compare: $frame ok"
    continue
  fi
  echo "compare: $frame differs by $metric pixels" >&2
  diff="$OUT/diff-$frame.png"
  if magick compare "$actual" "$reference" "$diff" 2>"$OUT/diff-$frame.err"; then
    echo "compare: $frame: the diff pass returned success after a non-zero AE metric" >&2
    status=1
    continue
  else
    diff_rc=$?
  fi
  if [ "$diff_rc" != "1" ]; then
    cat "$OUT/diff-$frame.err" >&2
    echo "compare: $frame: could not write the diff image (exit $diff_rc)" >&2
    status=1
    continue
  fi
  if [ ! -s "$diff" ]; then
    cat "$OUT/diff-$frame.err" >&2
    echo "compare: $frame: the diff image was not written to $diff" >&2
    status=1
    continue
  fi
  rm -f "$OUT/diff-$frame.err"
  status=1
done

exit "$status"
