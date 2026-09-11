#!/usr/bin/env bash
# One-shot wrapper: redeploy the Sepolia (chain key 8) RelayerContractLite on usc-devnet.
# Exists because the full command wraps in a terminal. Devnet-only keys.
#   ASC_CONTRACTS_DIR  compiled asc-contracts checkout (main c83b3372+); required
#   QUOTER_EOA         dedicated devnet quoter (default: the usc-dev test quoter)
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck disable=SC1090
set -a; . ~/.usc-dev-deployer.env; set +a
: "${ASC_CONTRACTS_DIR:?set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (npx hardhat compile)}"
[ -d "$ASC_CONTRACTS_DIR/artifacts/contracts" ] || { echo "ASC_CONTRACTS_DIR=$ASC_CONTRACTS_DIR has no artifacts/contracts — compile it first" >&2; exit 1; }
export QUOTER_EOA="${QUOTER_EOA:-0x7CC15788445190C8EaADa403b010196f96A0f28d}"
exec npx tsx scripts/redeploy-lite-devnet.mts
