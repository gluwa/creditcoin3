#!/usr/bin/env bash

# Usage: Run `bash abi-creator.sh` in the root of the metadata folder.

set -euo pipefail

sol_directory="sol"
abi_directory="abi"

# Drop ABI JSON files whose .sol was removed (otherwise abi/*.json flatten drifts from metadata).
shopt -s nullglob
for j in "$abi_directory"/*.json; do
    base=$(basename "$j" .json)
    if [ ! -f "$sol_directory/${base}.sol" ]; then
        rm -f "$j"
    fi
done

for p in "$sol_directory"/*; do
    file=$(basename "$p")
    file_with_extension="${file%.*}.json"
    out="$abi_directory/$file_with_extension"
    # Extract only the ABI from the combined JSON output
    solc "$sol_directory/$file" --combined-json abi --overwrite | \
        jq '.contracts | to_entries[0].value.abi' > "$out"
    # Fail the job if solc/jq left an empty or invalid file (otherwise jsonlint sees EOF).
    jq -e 'type == "array"' "$out" >/dev/null
done
