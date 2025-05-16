#!/bin/bash

# Check if there are broken symlinks in this repository!
set -euo pipefail

BROKEN_SYMLINKS=$(find . -xtype l)

if [ -z "$BROKEN_SYMLINKS" ]; then
    echo "PASS: No broken symlinks found"
    exit 0
fi

echo "FAIL: Broken symlinks found"
echo "$BROKEN_SYMLINKS"
echo "---------- END ----------"
echo
echo

exit 1
