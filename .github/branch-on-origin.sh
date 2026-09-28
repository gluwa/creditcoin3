#!/bin/bash

# Usage: branch-on-origin.sh <name> <old-name>
#
# Print <name> if origin has that branch, otherwise <old-name>.
#
# Branches are being renamed (usc-dev -> dev, writeability-off-usc-dev -> asc-dev).
# GitHub redirects a renamed branch for web URLs but not for git refs, so anything
# that fetches or compares against one has to ask origin which name it has. Delete
# this once the old spellings are gone.

set -euo pipefail

NAME="${1:?usage: $0 <name> <old-name>}"
OLD_NAME="${2:?usage: $0 <name> <old-name>}"

# --exit-code returns 2 when the ref is absent. Anything else means origin could not be
# asked, and guessing would only move the failure somewhere harder to read.
status=0
git ls-remote --exit-code --heads origin "refs/heads/$NAME" >/dev/null || status=$?

case "$status" in
    0) echo "$NAME" ;;
    2) echo "$OLD_NAME" ;;
    *)
        echo "FAIL: could not ask origin for its branches (git ls-remote exit $status)" >&2
        exit 1
        ;;
esac
