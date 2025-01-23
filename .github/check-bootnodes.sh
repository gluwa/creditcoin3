#!/bin/bash

set -euo pipefail

# Directory containing JSON files
chainspecs_dir="chainspecs"

# Check if chainspecs directory exists
if [ ! -d "$chainspecs_dir" ]; then
    echo "Error: Directory $chainspecs_dir does not exist."
    exit 1
fi

# Flag to track if bootNodes are found
bootnodes_found=0

# Iterate over JSON files in the directory
for file in "$chainspecs_dir"/*.json; do
    echo "Checking $file"
    jq -r < "$file" > /dev/null

    # Read the JSON file
    json=$(cat "$file")

    # Check if the bootNodes field is empty
    if [[ $(echo "$json" | jq -r '.bootNodes | length') -ne 0 ]]; then
        echo "BootNodes field is not empty."
        bootnodes_found=1
    fi
done

# Exit with error if bootNodes are found
if [ $bootnodes_found -eq 1 ]; then
    echo "Error: BootNodes found in at least one chainspec file."
    exit 1
else
    echo "No BootNodes found in any chainspec file."
    exit 0
fi
