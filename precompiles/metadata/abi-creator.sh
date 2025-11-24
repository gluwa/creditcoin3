#!/bin/bash

# Usage: Run `bash abi-creator.sh` in the root of the metadata folder.

sol_directory="sol"
abi_directory="abi"

for p in "$sol_directory"/*; do
    file=$(basename "$p")
    file_with_extension="${file%.*}.json"
    # Extract only the ABI from the combined JSON output
    solc "$sol_directory/$file" --combined-json abi --overwrite | \
        jq '.contracts | to_entries[0].value.abi' > "$abi_directory/$file_with_extension"
done
