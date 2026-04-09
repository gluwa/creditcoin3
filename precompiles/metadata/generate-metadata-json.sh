#!/bin/bash

set -euo pipefail

# Script to generate precompiles metadata JSON files for devnet and testnet
# Usage: ./generate-metadata-json.sh
# This script extracts precompile information from runtime/src/precompiles.rs
# to ensure consistency and avoid manual mapping errors.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

sol_directory="sol"
abi_directory="abi"
output_devnet="precompiles-creditcoin3-devnet.json"
output_testnet="precompiles-creditcoin3-testnet.json"
runtime_precompiles_file="../../runtime/src/precompiles.rs"

# Function to convert decimal to hex address (H160 format)
decimal_to_address() {
    local decimal=$1
    # Convert to hex and pad to 40 hex characters (20 bytes = 40 hex chars)
    # Format: 0x + 40 hex characters (uppercase)
    local hex_part
    hex_part=$(printf "%040x" "$decimal" | tr '[:lower:]' '[:upper:]')
    echo "0x${hex_part}"
}

# Function to map precompile type name to ABI filename and display name
get_precompile_info() {
    local precompile_type=$1
    case "$precompile_type" in
        "BlockProverPrecompile")
            echo "block_prover:BlockProver"
            ;;
        "ChainInfoPrecompile")
            echo "chain_info:ChainInfo"
            ;;
        "SignatureVerifierPrecompile")
            echo "signature_verifier:SignatureVerifier"
            ;;
        "SubstrateTransferPrecompile")
            echo "substrate_transfer:SubstrateTransfer"
            ;;
        *)
            echo ""
            ;;
    esac
}

# Parse runtime/precompiles.rs to extract precompile mappings
# Look for lines like: a if a == hash(4050) => Some(BlockProverPrecompile::<Runtime>::execute(handle)),
declare -a precompile_map=()

if [ ! -f "$runtime_precompiles_file" ]; then
    echo "Error: Runtime precompiles file not found at $runtime_precompiles_file"
    exit 1
fi

# Extract precompile mappings from the Rust file
while IFS= read -r line; do
    # Match lines like: a if a == hash(4050) => Some(BlockProverPrecompile::<Runtime>::execute(handle)),
    # Extract hash number and precompile type name
    if [[ $line =~ hash\(([0-9]+)\)\ =\>\ Some\(([A-Za-z]+Precompile):: ]]; then
        hash_number="${BASH_REMATCH[1]}"
        precompile_type="${BASH_REMATCH[2]}"

        # Get ABI filename and display name
        precompile_info=$(get_precompile_info "$precompile_type")

        if [ -n "$precompile_info" ]; then
            IFS=':' read -r abi_filename display_name <<< "$precompile_info"
            address=$(decimal_to_address "$hash_number")
            precompile_map+=("${abi_filename}:${display_name}:${address}")
        fi
    fi
done < <(grep -E "hash\([0-9]+\)\s*=>\s*Some\([A-Za-z]+Precompile::" "$runtime_precompiles_file")

if [ ${#precompile_map[@]} -eq 0 ]; then
    echo "Error: No precompiles found in $runtime_precompiles_file"
    exit 1
fi

echo "Found ${#precompile_map[@]} precompiles from runtime configuration:"
for mapping in "${precompile_map[@]}"; do
    echo "  - $mapping"
done

# Sort precompile_map alphabetically by ABI filename to match cat sol/*.sol order
sorted_precompile_map=()
while IFS= read -r line; do
    sorted_precompile_map+=("$line")
done < <(printf '%s\n' "${precompile_map[@]}" | sort -t':' -k1)
precompile_map=("${sorted_precompile_map[@]}")

# Function to generate JSON for a single precompile
generate_precompile_json() {
    local abi_file="$1"
    local precompile_name="$2"
    local address="$3"

    # Get source file name (remove .json extension and use same basename for .sol file)
    # basename with second argument removes that suffix: basename "file.json" .json -> "file"
    local abi_basename
    abi_basename=$(basename "$abi_file" .json)
    local source_file="${sol_directory}/${abi_basename}.sol"

    if [ ! -f "$source_file" ]; then
        echo "Error: Source file $source_file not found for $precompile_name (ABI file: $abi_file)" >&2
        exit 1
    fi

    if [ ! -f "$abi_file" ]; then
        echo "Error: ABI file $abi_file not found for $precompile_name" >&2
        exit 1
    fi

    # Check if ABI file is empty (non-empty check also verifies file exists)
    if [ ! -s "$abi_file" ]; then
        echo "Error: ABI file $abi_file is empty for $precompile_name" >&2
        exit 1
    fi

    # Read source and convert to JSON string (single line, escaped)
    # Strip trailing newline from source file before converting to JSON string
    local source_content
    source_content=$(cat "$source_file" | perl -pe 'chomp if eof' | jq -Rs '.')

    # Read ABI and convert to JSON string (compact, single line, escaped)
    # ABI files are JSON arrays, so parse first, then convert to JSON string
    # Root cause: jq errors might output to stdout, which would be invalid JSON for --argjson
    local abi_content
    local jq_exit_code

    # Parse JSON - separate stdout and stderr properly
    # Use a temp file for stderr to avoid mixing with stdout
    local stderr_file
    stderr_file=$(mktemp)
    abi_content=$(cat "$abi_file" | jq -c '.' 2>"$stderr_file")
    jq_exit_code=$?
    local jq_stderr
    jq_stderr=$(cat "$stderr_file")
    rm -f "$stderr_file"

    if [ $jq_exit_code -ne 0 ]; then
        echo "Error: Failed to parse ABI file $abi_file as JSON (exit code: $jq_exit_code)" >&2
        if [ -n "$jq_stderr" ]; then
            echo "jq stderr: ${jq_stderr:0:200}" >&2
        fi
        if [ -n "$abi_content" ]; then
            echo "jq stdout: ${abi_content:0:200}" >&2
        fi
        exit 1
    fi

    # Check if output is empty (after trimming whitespace)
    # Use echo -n to avoid adding newline, then trim
    local abi_content_trimmed
    abi_content_trimmed=$(echo -n "$abi_content" | tr -d '[:space:]')

    if [ -z "$abi_content_trimmed" ]; then
        echo "Error: ABI file $abi_file produced empty output after parsing" >&2
        echo "Debug: File size is $(wc -c < "$abi_file") bytes" >&2
        echo "Debug: First 100 chars: $(head -c 100 < "$abi_file")" >&2
        echo "Debug: jq exit code was: $jq_exit_code" >&2
        exit 1
    fi

    # Validate that the content is actually valid JSON before using with --argjson
    if ! echo -n "$abi_content" | jq -e . >/dev/null 2>&1; then
        echo "Error: ABI file $abi_file contains invalid JSON: ${abi_content:0:100}" >&2
        exit 1
    fi

    # Generate JSON entry
    # Note: source is a JSON string (from jq -Rs), abi is a JSON value (from jq -c)
    # Store source as parsed JSON, abi as JSON string (using tojson to ensure proper formatting)
    jq -n \
        --arg address "$address" \
        --arg name "$precompile_name" \
        --arg source "$source_content" \
        --argjson abi_json "$abi_content" \
        '{
            address: $address,
            name: $name,
            bytecode: "0xfe",
            compiler: "Not Installed",
            source: ($source | fromjson),
            abi: ($abi_json | tojson)
        }'
}

# Generate entries for all precompiles (already sorted by ABI filename)
entries=()
for mapping in "${precompile_map[@]}"; do
    IFS=':' read -r abi_filename precompile_name address <<< "$mapping"
    abi_file="${abi_directory}/${abi_filename}.json"

    # Generate entry - function will exit on error, so no need to check return code
    entry_output=$(generate_precompile_json "$abi_file" "$precompile_name" "$address" 2>&1)
    entries+=("$entry_output")
done

# Combine all entries into a JSON array
if [ ${#entries[@]} -eq 0 ]; then
    echo "Error: No precompile entries generated"
    exit 1
fi

# Create JSON array from entries
json_array="["
for i in "${!entries[@]}"; do
    if [ "$i" -gt 0 ]; then
        json_array+=","
    fi
    json_array+="${entries[$i]}"
done
json_array+="]"

# Format and write to output files
echo "$json_array" | jq '.' > "$output_devnet"
echo "$json_array" | jq '.' > "$output_testnet"

echo "Generated $output_devnet and $output_testnet successfully"
