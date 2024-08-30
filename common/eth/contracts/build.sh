#!/bin/bash

# Define the input and output file names
input_file="artifact.json"
output_file="prover.json"

solc --via-ir --pretty-json --combined-json abi,bin sol/Prover.sol > $input_file

# Extract the JSON struct under "Prover.sol:QueryVerifierContract" and save it to contract.json
jq '.contracts["sol/Prover.sol:CreditcoinPublicProver"]' "$input_file" > "$output_file"

# Print a message indicating the extraction was successful
echo "JSON struct extracted and saved to $output_file."

# Remove artifact.json
rm $input_file

# Print a message indicating the removal was successful
echo "artifact.json removed."
