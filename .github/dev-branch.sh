#!/bin/bash

# Print the name of the development branch on origin.
#
# usc-dev is being renamed to dev. GitHub redirects the old name for web URLs but not
# for git refs, so anything that fetches or compares against the branch has to ask
# origin which name it has. Delete this once the usc-dev spellings are gone.

set -euo pipefail

# --exit-code returns 2 when the ref is absent. Anything else means origin could not be
# asked, and guessing would only move the failure somewhere harder to read.
status=0
git ls-remote --exit-code --heads origin refs/heads/dev >/dev/null || status=$?

case "$status" in
    0) echo "dev" ;;
    2) echo "usc-dev" ;;
    *)
        echo "FAIL: could not ask origin for its branches (git ls-remote exit $status)" >&2
        exit 1
        ;;
esac
