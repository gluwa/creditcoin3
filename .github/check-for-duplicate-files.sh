#!/bin/bash

# Check if there are duplicate files in the repository!

ALL_FILES=$(
    find ./ -not -empty -type f |
    grep -v ".git/" |
    grep -v "./target" |
    grep -v node_modules |
    grep -v cli/dist |
    grep -v cli/src/lib/interfaces |
    # these are usually the same b/c that's what we were told to do
    grep -v precompiles/metadata/precompiles-creditcoin3 |
    xargs md5sum
)

set -euo pipefail

DUPLICATE_CHECK_SUMS=$(echo "$ALL_FILES" | cut -f1 -d' ' | sort | uniq -d)
if [ -z "$DUPLICATE_CHECK_SUMS" ]; then
    echo "PASS: No duplicate files found"
    exit 0
fi

DUPLICATE_COUNT=$(echo "$DUPLICATE_CHECK_SUMS" | wc -l)
echo "----- DUPLICATE CHECK SUMS -----"
echo "$DUPLICATE_CHECK_SUMS"
echo "---------- END ----------"
echo
echo

for CHECK_SUM in $DUPLICATE_CHECK_SUMS; do
    echo "ERROR: duplicate files detected:"
    echo "$ALL_FILES" | grep "$CHECK_SUM"
    echo
done

echo "FAIL: $DUPLICATE_COUNT duplicate files found in git!"
echo "TODO: Convert those into symlinks"
exit "$DUPLICATE_COUNT"
