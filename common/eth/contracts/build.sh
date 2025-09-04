#!/bin/bash
set -euo pipefail

input_file="artifact.json"
output_file="prover.json"

base_path="."
# Add all plausible node_modules locations relative to contracts/
include_paths=(
  "./node_modules"          # common/eth/node_modules
)

# Compose flags
include_flags=()
for p in "${include_paths[@]}"; do
  include_flags+=( --include-path "$p" )
done

allow_paths=$(IFS=, ; echo "${include_paths[*]}")

solc --pretty-json --combined-json abi,bin \
  --no-cbor-metadata \
  --base-path "$base_path" \
  "${include_flags[@]}" \
  --allow-paths "$allow_paths" \
  sol/Prover.sol > "$input_file"

jq '.contracts["sol/Prover.sol:CreditcoinPublicProver"]' "$input_file" > "$output_file"
echo "JSON struct extracted and saved to $output_file."
rm "$input_file"
echo "artifact.json removed."
