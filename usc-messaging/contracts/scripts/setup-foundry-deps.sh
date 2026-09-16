#!/usr/bin/env bash
# Prepares this Foundry project to build/test, without vendoring anything into git:
#   - fetches forge-std at the commit pinned in foundry.lock (never committed, like node_modules)
#   - regenerates remappings.txt (machine-specific paths, gitignored) from $ASC_CONTRACTS_DIR
#
# The @gluwa/usc-contracts npm pin in package.json (0.1.2) predates MessageReceiverBase,
# ASCBridgeTypes, IASCProofVerifier, BlockProverTypes, EvmV1Decoder, CompatibleERC20, and the
# current Inbox/Outbox — BridgeVault/BridgeHub need all of these, so building against it requires
# a real checkout of gluwa/asc-contracts (private repo; CI checks it out at a pinned SHA and sets
# ASC_CONTRACTS_DIR — see .github/workflows/write-ability-e2e.yml and bridge-e2e.yml).
#
# asc-contracts moves fast (mid-redesign as of 2026-09: a USC->ASC rename plus a new
# ChainRegistry/OutboxDiscovery/bridge-intent framework landed within days of each other). Point
# ASC_CONTRACTS_DIR at a checkout pinned to the SAME commit write-ability-e2e.yml checks out, not
# an arbitrary local clone's HEAD/main — otherwise this builds fine locally against APIs CI (and
# any real deploy) does not actually have yet.
#
# Usage: ASC_CONTRACTS_DIR=/path/to/asc-contracts-checkout ./scripts/setup-foundry-deps.sh
set -euo pipefail

: "${ASC_CONTRACTS_DIR:?Set ASC_CONTRACTS_DIR to a checkout of gluwa/asc-contracts, npm installed there for its vendored @openzeppelin/contracts}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
contracts_dir="$(dirname "$script_dir")"
cd "$contracts_dir"

forge_std_rev="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["lib/forge-std"]["tag"]["rev"])' foundry.lock)"

if [ ! -d lib/forge-std ]; then
  echo "Fetching forge-std @ ${forge_std_rev} (pinned in foundry.lock)..."
  git clone -q https://github.com/foundry-rs/forge-std lib/forge-std
  git -C lib/forge-std checkout -q "$forge_std_rev"
  rm -rf lib/forge-std/.git
else
  echo "lib/forge-std already present, skipping fetch."
fi

cat > remappings.txt <<EOF
@gluwa/usc-contracts/=${ASC_CONTRACTS_DIR}/contracts/
@openzeppelin/contracts/=${ASC_CONTRACTS_DIR}/node_modules/@openzeppelin/contracts/
EOF

echo "Wrote remappings.txt for ASC_CONTRACTS_DIR=${ASC_CONTRACTS_DIR}"
