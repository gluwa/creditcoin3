# Steps for Updating Metadata After Precompile Changes

## Automated Workflow (Recommended)

After making changes to a precompile's Solidity source code:

### 1. Generate ABI Files

```sh
cd precompiles/metadata
./abi-creator.sh
```

This generates compact JSON array files in the `abi/` directory for each Solidity contract.

### 2. Generate Metadata JSON Files

```sh
./generate-metadata-json.sh
```

This automatically:
- Reads all Solidity source files from `sol/`
- Reads all ABI files from `abi/`
- Generates/updates `precompiles-creditcoin3-devnet.json` and `precompiles-creditcoin3-testnet.json`
- Formats sources as JSON strings and ABIs as compact JSON strings
- Ensures proper ordering and formatting

### 3. Verify the Changes

```sh
cd ../..
.github/check-solidity-source-vs-metadata.sh
```

This validates that:
- Sources in the JSON files match the on-disk Solidity files
- ABIs in the JSON files match the generated ABI files
- Addresses are correct

## Manual Workflow (If Needed)

If you need to manually update the metadata JSON files:

1. **Generate ABI**: Run `./abi-creator.sh` to create ABI files
2. **Convert ABI to JSON string**: `cat abi/YourContract.json | jq -Rs '.'`
3. **Convert source to JSON string**: `cat sol/YourContract.sol | jq -Rs '.'`
4. **Update the JSON files**: Copy the results into the appropriate fields in `precompiles-creditcoin3-devnet.json` and `precompiles-creditcoin3-testnet.json`

**Note**: The automated workflow (`generate-metadata-json.sh`) handles all of this automatically and is recommended to avoid formatting issues.

## CI Checks

The CI pipeline automatically:
- Generates ABI files from Solidity sources
- Generates metadata JSON files
- Verifies that committed JSON files match the generated ones
- Fails if the metadata JSON files are out of date

This ensures that metadata JSON files are always kept in sync with the source code.