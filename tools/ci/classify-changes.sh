#!/bin/sh
# Classify a change list as code or docs for the CI workflow.
#
# Reads paths from stdin, one per line, and prints `code=true` when any path can
# affect the build or the tests. Prints `code=false` when every path is
# documentation, and the workflow then skips the heavy jobs.
#
# Documentation is a closed list: Markdown, docs/images, the license files, the
# contributor list and the issue templates. Anything else counts as code, so a
# new file type runs the full CI until it is added here.
set -eu

code=false
while IFS= read -r path; do
    [ -n "$path" ] || continue
    case $path in
        *.md | docs/images/* | LICENSE | LICENSE.* | COPYING | COPYING.* | \
            .all-contributorsrc | .github/ISSUE_TEMPLATE/*) ;;
        *) code=true ;;
    esac
done
printf 'code=%s\n' "$code"
