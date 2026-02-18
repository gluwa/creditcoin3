#!/usr/bin/env bash
# Deploy USC messaging contracts to Anvil or configured RPC.
# Usage: ./scripts/deploy.sh [rpc_url]
# Env: RPC_URL, PRIVATE_KEY, PAYEE_ADDRESS, CREDITCOIN_CHAIN_ID

set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_DIR="$(cd "$SCRIPT_DIR/../contracts" && pwd)"
RPC_URL="${1:-${RPC_URL:-http://127.0.0.1:8545}}"

cd "$CONTRACTS_DIR"
./script/deploy.sh "$RPC_URL"
