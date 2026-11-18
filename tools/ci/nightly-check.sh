#!/bin/sh
# Decide whether a scheduled run has anything to build: compare HEAD with the head_sha of the
# last successful run of the same workflow.
#
# Usage: nightly-check.sh <workflow-file>
#
# Reads GITHUB_EVENT_NAME, GITHUB_SHA, GITHUB_REF_NAME, GH_TOKEN and GH_REPO, and prints
# `changed=true` or `changed=false` for $GITHUB_OUTPUT. A manual dispatch always builds.
set -eu

workflow=${1:?usage: nightly-check.sh <workflow-file>}

if [ "${GITHUB_EVENT_NAME:-}" != "schedule" ]; then
    echo "changed=true"
    exit 0
fi

branch=${GITHUB_REF_NAME:-main}
last=$(gh api \
    "repos/${GH_REPO}/actions/workflows/${workflow}/runs?branch=${branch}&status=success&per_page=1" \
    --jq '.workflow_runs[0].head_sha // ""')

if [ "$last" = "${GITHUB_SHA:-}" ]; then
    echo "changed=false"
else
    echo "changed=true"
fi
