#!/usr/bin/env bash

# Regenerate the AttestCoinTreasuryVault test artifact from the Solidity source that ships in the
# @gluwa/atc-treasury-and-gov npm package.
#
# The vault lives in the atc-treasury-and-gov repo, not here. That package distributes Solidity
# source only -- no ABI and no bytecode -- so there is nothing to require() directly, and deploying
# the vault from a test means compiling it. Generating the artifact rather than hand-maintaining a
# copy is the point: the JSON cannot silently drift from the contract that repo ships, because it
# is rebuilt from that contract on every test run.
#
# Usage:
#   bash scripts/sync-vault-artifact.sh           # regenerate in place
#   bash scripts/sync-vault-artifact.sh --check   # report drift, touch nothing, exit non-zero
#
# The package is a devDependency of cli/package.json, so a normal `yarn install` puts the contract
# source in place and this script compiles from it. To move to a new vault release, bump that
# version and re-run without --check to rewrite the artifact.

set -euo pipefail

CLI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT="$CLI_DIR/src/test/blockchain-tests/artifacts/AttestCoinTreasuryVault.json"
CONTRACT_PATH="contracts/treasury/AttestCoinTreasuryVault.sol"

# Mirrors atc-treasury-and-gov's hardhat.config.ts so this artifact is the contract that repo
# tests and ships, not a local variant of it. `paris` is its pin, inherited from the asc-contracts
# build the vault was split out of -- not a statement about Creditcoin, which runs Osaka (see
# EVM_CONFIG in runtime/src/lib.rs). Paris bytecode executes correctly on Osaka; the reverse would
# not hold, so tracking the pin is the safe direction. If that repo raises it, raise it here in
# the same change.
#
# `--metadata-hash none` is about reproducibility, not size. solc hashes the resolved import
# paths into the trailing metadata, so an @openzeppelin/contracts that npm hoists to the top
# level and one that yarn nests under the vault package produce different bytecode from identical
# source -- which would make the drift check below fail on install layout rather than on real
# change. Dropping the hash makes the output a function of source + compiler + settings only.
# Nothing here is verified on Sourcify/Etherscan, so the hash buys us nothing to offset that.
SOLC_FLAGS=(--optimize --optimize-runs 200 --via-ir --evm-version paris --metadata-hash none)

mode="write"
if [ "${1:-}" = "--check" ]; then
    mode="check"
elif [ $# -gt 0 ]; then
    echo "usage: $(basename "$0") [--check]" >&2
    exit 64
fi

# Skip silently rather than fail when the package is absent: see NOT YET ACTIVE above. Once the
# dependency is declared, a missing install is a real error and yarn install will have failed
# first anyway.
if ! pkg_dir=$(node -p \
    "require('path').dirname(require.resolve('@gluwa/atc-treasury-and-gov/package.json'))" \
    2>/dev/null); then
    echo "@gluwa/atc-treasury-and-gov is not installed; keeping the committed artifact as-is."
    exit 0
fi

# solc needs a concrete directory for the @openzeppelin/contracts remapping, and the package
# manager may hoist that dependency to the top level or nest it under the vault package. Ask node
# where it actually landed instead of guessing at a path.
oz_dir=$(node -p \
    "require('path').dirname(require.resolve('@openzeppelin/contracts/package.json', { paths: ['$pkg_dir'] }))")

if ! command -v solc >/dev/null 2>&1; then
    if [ -n "${CI:-}" ]; then
        echo "ERROR: solc is not on PATH. CI installs it via .github/install-solidity-compiler.sh." >&2
        exit 1
    fi
    echo "WARNING: solc is not on PATH; keeping the committed artifact as-is." >&2
    echo "         Install it to verify the artifact against @gluwa/atc-treasury-and-gov locally." >&2
    exit 0
fi

pkg_version=$(node -p "require('@gluwa/atc-treasury-and-gov/package.json').version")
solc_version=$(solc --version | sed -n 's/^Version: //p')

combined=$(solc "${SOLC_FLAGS[@]}" \
    --combined-json abi,bin \
    --allow-paths "$pkg_dir,$oz_dir" \
    "@openzeppelin/contracts/=$oz_dir/" \
    "$pkg_dir/$CONTRACT_PATH")

generated=$(jq \
    --arg comment "GENERATED FILE -- do not edit by hand. Compiled from $CONTRACT_PATH (solc $solc_version, ${SOLC_FLAGS[*]}). Regenerate with cli/scripts/sync-vault-artifact.sh; edit the contract in the atc-treasury-and-gov repo." \
    --arg source "@gluwa/atc-treasury-and-gov@$pkg_version" \
    '.contracts
     | to_entries
     | map(select(.key | endswith(":AttestCoinTreasuryVault")))
     | if length != 1 then
           error("expected exactly one AttestCoinTreasuryVault in solc output, got \(length)")
       else .[0].value end
     | {
         _comment: $comment,
         contractName: "AttestCoinTreasuryVault",
         sourcePackage: $source,
         abi: .abi,
         bytecode: ("0x" + .bin),
       }' <<<"$combined")

# A build that produced no bytecode would deploy as an empty contract and fail confusingly later.
jq -e '.bytecode | length > 2' >/dev/null <<<"$generated"

if [ "$mode" = "check" ]; then
    # Compare the compiled output and its provenance, not `_comment`: that line carries the local
    # solc build string, which differs between platforms (Linux vs Darwin) and would report drift
    # that isn't there.
    #
    # Report which half moved rather than diffing it: the bytecode is a single 6KB line, and a
    # raw diff of it buries the finding in noise without telling anyone anything actionable.
    drift=()
    if ! diff -q <(jq -S .abi "$ARTIFACT") <(jq -S .abi <<<"$generated") >/dev/null; then
        drift+=("abi")
    fi
    if [ "$(jq -r .bytecode "$ARTIFACT")" != "$(jq -r .bytecode <<<"$generated")" ]; then
        drift+=("bytecode")
    fi
    # Caught separately from the bytecode so a dependency bump that happens to compile to the same
    # code still forces a rebuild, and the recorded provenance never names the wrong release.
    if [ "$(jq -r .sourcePackage "$ARTIFACT")" != "$(jq -r .sourcePackage <<<"$generated")" ]; then
        drift+=("sourcePackage")
    fi

    if [ ${#drift[@]} -eq 0 ]; then
        echo "AttestCoinTreasuryVault.json is up to date with @gluwa/atc-treasury-and-gov@$pkg_version."
        exit 0
    fi

    echo "ERROR: AttestCoinTreasuryVault.json is stale against @gluwa/atc-treasury-and-gov@$pkg_version." >&2
    echo "       Differs in: ${drift[*]}" >&2
    echo "       The committed artifact is not what that package's contract compiles to. Either" >&2
    echo "       it was hand-edited, or the dependency moved without the artifact being rebuilt." >&2
    echo "       Fix with: cd cli && bash scripts/sync-vault-artifact.sh" >&2
    exit 1
fi

printf '%s\n' "$generated" >"$ARTIFACT"
echo "Wrote $(basename "$ARTIFACT") from @gluwa/atc-treasury-and-gov@$pkg_version (solc $solc_version)."
