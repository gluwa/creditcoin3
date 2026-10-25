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
- Extracts precompile information from `runtime/src/precompiles.rs` (addresses and precompile types)
- Maps precompile types to ABI filenames and display names
- Reads all Solidity source files from `sol/`
- Reads all ABI files from `abi/`
- Generates/updates `precompiles-creditcoin3-devnet.json` and `precompiles-creditcoin3-testnet.json`
- Formats sources as JSON strings and ABIs as compact JSON strings
- Ensures proper ordering and formatting

**Note**: The script extracts precompile addresses directly from the runtime configuration, ensuring consistency and avoiding manual mapping errors. If you add a new precompile to `runtime/src/precompiles.rs`, make sure to add its mapping in the `get_precompile_info()` function in the script.

### 3. Verify the Changes

You can verify that the generated files match what's committed:

```sh
cd ../..
git status precompiles/metadata/precompiles-creditcoin3-devnet.json precompiles/metadata/precompiles-creditcoin3-testnet.json
git diff precompiles/metadata/precompiles-creditcoin3-devnet.json precompiles/metadata/precompiles-creditcoin3-testnet.json
```

If there are no changes, the files are up to date. If there are changes, commit them.

**Note**: The CI pipeline automatically runs `generate-metadata-json.sh` and checks for uncommitted changes using `git status` and `git diff`, ensuring the metadata JSON files stay in sync with the source code.

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
- Generates metadata JSON files using `generate-metadata-json.sh`
- Checks for uncommitted changes using `git status` and `git diff`
- Fails if the metadata JSON files are out of date (prompting you to commit the changes)

This ensures that metadata JSON files are always kept in sync with the source code. The check uses `git diff` directly on the generated files, avoiding the need for temporary files or complex normalization.
