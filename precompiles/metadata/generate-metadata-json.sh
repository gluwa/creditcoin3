#!/bin/bash

set -euo pipefail

# Script to generate precompiles metadata JSON files for devnet and testnet
# Usage: ./generate-metadata-json.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

sol_directory="sol"
abi_directory="abi"
output_devnet="precompiles-creditcoin3-devnet.json"
output_testnet="precompiles-creditcoin3-testnet.json"

# Map ABI file names to precompile names and addresses
# Format: "abi_filename:precompile_name:address"
declare -a precompile_map=(
    "INativeQueryVerifier:QueryVerifierContract:0x0000000000000000000000000000000000000FD2"
    "chain_info:ChainInfo:0x0000000000000000000000000000000000000fD3"
    "signature_verifier:SignatureVerifier:0x00000000000000000000000000000000000013B9"
    "substrate_transfer:SubstrateTransfer:0x0000000000000000000000000000000000000Fd1"
)

# Function to generate JSON for a single precompile
generate_precompile_json() {
    local abi_file="$1"
    local precompile_name="$2"
    local address="$3"
    
    # Get source file name (remove .json extension and find corresponding .sol file)
    local abi_basename=$(basename "$abi_file" .json)
    local source_file=""
    
    # Find the corresponding source file
    case "$abi_basename" in
        "INativeQueryVerifier")
            source_file="${sol_directory}/INativeQueryVerifier.sol"
            ;;
        "chain_info")
            source_file="${sol_directory}/chain_info.sol"
            ;;
        "signature_verifier")
            source_file="${sol_directory}/signature_verifier.sol"
            ;;
        "substrate_transfer")
            source_file="${sol_directory}/substrate_transfer.sol"
            ;;
        *)
            echo "Warning: Unknown ABI file $abi_file, skipping..."
            return 1
            ;;
    esac
    
    if [ ! -f "$source_file" ]; then
        echo "Error: Source file $source_file not found for $precompile_name"
        return 1
    fi
    
    if [ ! -f "$abi_file" ]; then
        echo "Error: ABI file $abi_file not found for $precompile_name"
        return 1
    fi
    
    # Read source and convert to JSON string (single line, escaped)
    # Strip trailing newline from source file before converting to JSON string
    local source_content=$(cat "$source_file" | perl -pe 'chomp if eof' | jq -Rs '.')
    
    # Read ABI and convert to JSON string (compact, single line, escaped)
    local abi_content=$(cat "$abi_file" | jq -Rs '.')
    
    # Generate JSON entry
    jq -n \
        --arg address "$address" \
        --arg name "$precompile_name" \
        --argjson source "$source_content" \
        --argjson abi "$abi_content" \
        '{
            address: $address,
            name: $name,
            bytecode: "0xfe",
            compiler: "Not Installed",
            source: $source,
            abi: $abi
        }'
}

# Generate entries for all precompiles
entries=()
for mapping in "${precompile_map[@]}"; do
    IFS=':' read -r abi_filename precompile_name address <<< "$mapping"
    abi_file="${abi_directory}/${abi_filename}.json"
    
    if [ -f "$abi_file" ]; then
        entry=$(generate_precompile_json "$abi_file" "$precompile_name" "$address")
        entries+=("$entry")
    else
        echo "Warning: ABI file $abi_file not found, skipping $precompile_name"
    fi
done

# Combine all entries into a JSON array
if [ ${#entries[@]} -eq 0 ]; then
    echo "Error: No precompile entries generated"
    exit 1
fi

# Create JSON array from entries
json_array="["
for i in "${!entries[@]}"; do
    if [ $i -gt 0 ]; then
        json_array+=","
    fi
    json_array+="${entries[$i]}"
done
json_array+="]"

# Format and write to output files
echo "$json_array" | jq '.' > "$output_devnet"
echo "$json_array" | jq '.' > "$output_testnet"

echo "Generated $output_devnet and $output_testnet successfully"

