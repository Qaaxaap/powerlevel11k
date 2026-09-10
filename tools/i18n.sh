#!/usr/bin/env bash
# p11k i18n 辅助脚本。
#
#   tools/i18n.sh extract   # xgettext 扫源码 → crates/p11k-engine/po/p11k.pot
#   tools/i18n.sh update    # msgmerge 把 pot 的新条目并进各语言 .po（保留已有翻译）
#
# 文案改动后两步都跑一遍，然后补上新条目的 msgstr。
# `cargo test -p p11k-engine` 里有守卫测试盯着：源码扫描结果必须和 pot 一致、
# pot 的每条 msgid 必须都在各 .po 里有条目，漏了会测试失败。

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
src="$root/crates/p11k-engine/src"
po="$root/crates/p11k-engine/po"
pot="$po/p11k.pot"

# 源码里两种标记: t("…") 直接翻译, msgid("…") 只标记(稍后由调用方翻译)。
# i18n.rs 自己不算：xgettext 的 keyword 是后缀匹配，`gettext("…")` 也会被当成
# `t` 调用，而那里的参数是变量、不是文案。
keywords=(--keyword=t:1 --keyword=msgid:1)

sources=()
for f in "$src"/*.rs; do
  [[ $(basename "$f") == i18n.rs ]] && continue
  sources+=("crates/p11k-engine/src/$(basename "$f")")
done

# 可复现：POT-Creation-Date 取最后一次提交时间，避免每次 extract 都改一行。
if [[ -z ${SOURCE_DATE_EPOCH:-} ]] && git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
  export SOURCE_DATE_EPOCH
  SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct)
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

case ${1:-extract} in
extract)
  # 在仓库根跑，pot 里记的是相对路径。
  cd "$root"
  # xgettext 只认 C/C++ 词法，Rust 的生命期 `'a` 与 raw string `r#"…"#` 会让它
  # 报一串 unterminated 警告。这些区段里没有文案（有的话守卫测试会报源码与 pot
  # 不一致），所以这里只把噪声滤掉，其余 stderr 照原样转出来。
  err=$(mktemp)
  trap 'rm -f "$err"' EXIT
  xgettext \
    --language=C++ \
    "${keywords[@]}" \
    --from-code=UTF-8 \
    --package-name=p11k \
    --package-version="${version:-0.0.0}" \
    --msgid-bugs-address="https://github.com/powerlevel11k/p11k/issues" \
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
