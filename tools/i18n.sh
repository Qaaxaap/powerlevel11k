#!/usr/bin/env bash
# Helper for the p11k i18n workflow.
#
#   tools/i18n.sh extract   # xgettext scans the sources into crates/p11k-engine/po/p11k.pot
#   tools/i18n.sh update    # msgmerge merges new pot entries into each .po, keeping translations
#
# Run both commands after a user-facing text change. Then add the msgstr for each new entry.
# Guard tests in `cargo test -p p11k-engine` check this. The source scan must match the pot, and
# every pot msgid must have an entry in each .po. A missing entry fails the tests.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
src="$root/crates/p11k-engine/src"
po="$root/crates/p11k-engine/po"
pot="$po/p11k.pot"

# The sources use two markers. t("...") translates directly. msgid("...") only marks the string,
# and a caller translates it later. i18n.rs does not count: xgettext matches keywords by suffix,
# so it picks up `gettext("...")` as a `t` call too, and that argument is a variable, not a
# message.
keywords=(--keyword=t:1 --keyword=msgid:1)

sources=()
while IFS= read -r -d '' f; do
  [[ $(basename "$f") == i18n.rs ]] && continue
  sources+=("${f#"$root"/}")
done < <(find "$src" -name '*.rs' -print0 | sort -z)

# Reproducible: POT-Creation-Date comes from the last commit time, so extract does not rewrite a
# line on every run.
if [[ -z ${SOURCE_DATE_EPOCH:-} ]] && git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
  export SOURCE_DATE_EPOCH
  SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct)
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

case ${1:-extract} in
extract)
  # Run from the repository root, so the pot records relative paths.
  cd "$root"
  # xgettext only understands C/C++ lexing. Rust lifetimes (`'a`) and raw strings (`r#"..."#`)
  # make it print a series of unterminated warnings. Those regions hold no messages: if they did,
  # the guard test would report a source/pot mismatch. So this filters the noise out and passes
  # the rest of stderr through unchanged.
  err=$(mktemp)
  trap 'rm -f "$err"' EXIT
  xgettext \
    --language=C++ \
    "${keywords[@]}" \
    --from-code=UTF-8 \
    --package-name=p11k \
    --package-version="${version:-0.0.0}" \
    --msgid-bugs-address="https://github.com/Qaaxaap/powerlevel11k/issues" \
    --sort-by-file \
    --output="$pot" \
    "${sources[@]}" 2>"$err"
  grep -vE 'warning: unterminated (character constant|string literal)' "$err" >&2 || true
  echo "wrote $pot"
  ;;
update)
  [[ -f $pot ]] || { echo "run '$0 extract' first" >&2; exit 2; }
  for f in "$po"/*.po; do
    msgmerge --quiet --update --backup=none "$f" "$pot"
    echo "merged $pot -> $f"
  done
  ;;
*)
  echo "usage: $0 [extract|update]" >&2
  exit 2
  ;;
esac
