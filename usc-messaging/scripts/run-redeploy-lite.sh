#!/bin/zsh
# One-shot wrapper: redeploy the Sepolia (chain key 8) RelayerContractLite on usc-devnet.
set -euo pipefail
cd "$(dirname "$0")/.."
set -a; . ~/.usc-dev-deployer.env; set +a
export QUOTER_EOA=0x7CC15788445190C8EaADa403b010196f96A0f28d
export ASC_CONTRACTS_DIR=/private/tmp/claude-501/-Users-dylan-Projects-creditcoin3/7ea76496-06e9-4b09-97b2-3d23c97d60bc/scratchpad/asc-contracts-c83b337
exec npx tsx scripts/redeploy-lite-devnet.mts
