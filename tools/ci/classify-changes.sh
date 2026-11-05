#!/bin/sh
# Classify a change list as code or docs for the CI workflow.
#
# Reads paths from stdin, one per line, and prints `code=true` when any path can
# affect the build or the tests. Prints `code=false` when every path is
# documentation, and the workflow then skips the heavy jobs.
#
# Documentation is a closed list: the root documents, docs/, the license files, the
# contributor list and the issue and pull request templates. A Markdown file elsewhere,
# for example a crate README, counts as code. Anything else counts as code, so a new file
# type runs the full CI until it is added here.
set -eu

code=false
while IFS= read -r path; do
    [ -n "$path" ] || continue
    case $path in
        README.md | README.zh.md | CHANGELOG.md | CONTRIBUTING.md | CONTRIBUTING.zh.md | \
            CODE_OF_CONDUCT.md | SECURITY.md | AGENTS.md | docs/* | \
            .github/ISSUE_TEMPLATE/* | .github/pull_request_template.md | \
            LICENSE | LICENSE.* | COPYING | COPYING.* | .all-contributorsrc) ;;
        *) code=true ;;
    esac
done
printf 'code=%s\n' "$code"
