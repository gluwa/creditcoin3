#!/usr/bin/env bash

# This script:
# - Checks that JSON files have valid syntax

set -e

for JSON_FILE in $(find ./ -type f -name "*.json" | grep -v node_modules/ | grep -v target/ | sort); do
    echo "INFO: examining $JSON_FILE"
    jq -r < "$JSON_FILE" >/dev/null
done
