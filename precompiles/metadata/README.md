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

By default this regenerates `precompiles-creditcoin3-devnet.json` and `precompiles-creditcoin3-testnet.json` only. `precompiles-creditcoin3-mainnet.json` is opt-in via `--mainnet` (see [Chain Flags](#chain-flags) below) since it should only be refreshed when a change is actually being promoted towards `main`.

This automatically:
- Extracts precompile information from `runtime/src/precompiles.rs` (addresses and precompile types)
- Maps precompile types to ABI filenames and display names
- Reads all Solidity source files from `sol/`
- Reads all ABI files from `abi/`
- Generates/updates the requested `precompiles-creditcoin3-*.json` file(s)
- Formats sources as JSON strings and ABIs as compact JSON strings
- Ensures proper ordering and formatting

**Note**: The script extracts precompile addresses directly from the runtime configuration, ensuring consistency and avoiding manual mapping errors. If you add a new precompile to `runtime/src/precompiles.rs`, make sure to add its mapping in the `get_precompile_info()` function in the script.

#### Chain Flags

The script accepts flags to control which metadata JSON file(s) get (re)generated, instead of that being something to hand-edit in the script itself:

| Flag           | Effect                                                | Default |
|----------------|--------------------------------------------------------|---------|
| `--devnet`     | Generate `precompiles-creditcoin3-devnet.json`          | on      |
| `--no-devnet`  | Skip `precompiles-creditcoin3-devnet.json`              |         |
| `--testnet`    | Generate `precompiles-creditcoin3-testnet.json`         | on      |
| `--no-testnet` | Skip `precompiles-creditcoin3-testnet.json`             |         |
| `--mainnet`    | Generate `precompiles-creditcoin3-mainnet.json`         | off     |
| `--no-mainnet` | Skip `precompiles-creditcoin3-mainnet.json`             |         |
| `--all`        | Shorthand for `--devnet --testnet --mainnet`            |         |

Examples:

```sh
./generate-metadata-json.sh                 # devnet + testnet (default)
./generate-metadata-json.sh --mainnet       # devnet + testnet + mainnet
./generate-metadata-json.sh --all           # same as above
./generate-metadata-json.sh --no-devnet --mainnet  # testnet + mainnet only
```

### 3. Verify the Changes

You can verify that the generated files match what's committed:

```sh
cd ../..
git status precompiles/metadata/precompiles-creditcoin3-devnet.json precompiles/metadata/precompiles-creditcoin3-testnet.json
git diff precompiles/metadata/precompiles-creditcoin3-devnet.json precompiles/metadata/precompiles-creditcoin3-testnet.json
```

(add `precompiles-creditcoin3-mainnet.json` to both commands above if you regenerated it with `--mainnet`.)

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

The CI pipeline (`.github/workflows/sanity.yml`) automatically:
- Generates ABI files from Solidity sources
- Regenerates `devnet`/`testnet` metadata JSON with `generate-metadata-json.sh --devnet --testnet`, and additionally regenerates `mainnet` metadata JSON with `--mainnet` when the PR targets `main` (`github.base_ref == 'main'`)
- Checks for uncommitted changes using `git status` and `git diff` on whichever file(s) were just regenerated
- Fails if the metadata JSON files are out of date (prompting you to commit the changes)

Mainnet metadata is intentionally excluded from this check on PRs targeting `usc-dev`/`usc-testnet`, since it's expected to lag behind until a change is actually promoted to `main` — the same `github.base_ref` signal already used elsewhere in this workflow (e.g. the `Prepare ENV for Devnet/Testnet/Mainnet` steps) drives this instead of a hardcoded/hand-edited output list in the script.

This ensures that metadata JSON files are always kept in sync with the source code. The check uses `git diff` directly on the generated files, avoiding the need for temporary files or complex normalization.
