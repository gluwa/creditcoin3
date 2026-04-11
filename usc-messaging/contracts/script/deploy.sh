#!/usr/bin/env bash
# Deploy USC contracts via forge create (no git submodules).
# Usage: ./script/deploy.sh [rpc_url]
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RPC_URL="${1:-${RPC_URL:-http://127.0.0.1:8545}}"
PRIVATE_KEY="${PRIVATE_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
PAYEE="${PAYEE_ADDRESS:-0x0000000000000000000000000000000000000001}"

SOURCE_RPC_URL="${SOURCE_RPC_URL:-http://127.0.0.1:9944}"
SOURCE_PRIVATE_KEY="${SOURCE_PRIVATE_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
SOURCE_CHAIN_ID="${SOURCE_CHAIN_ID:-42}"
LOCAL_CHAIN_KEY="${LOCAL_CHAIN_KEY:-0x0000000000000000000000000000000000000000000000000000000000000001}"

cd "$CONTRACTS_DIR"

deploy_to_destination() {
  local out
  out=$(forge create "$1" --rpc-url "$RPC_URL" --private-key "$PRIVATE_KEY" "$2" 2>&1)
  echo "$out" | grep -oE "Deployed to destination chain at: 0x[a-fA-F0-9]{40}" | cut -d' ' -f5
}

deploy_to_source() {
  local out
  out=$(forge create "$1" --rpc-url "$SOURCE_RPC_URL" --private-key "$SOURCE_PRIVATE_KEY" "$2" 2>&1)
  echo "$out" | grep -oE "Deployed to source chain at: 0x[a-fA-F0-9]{40}" | cut -d' ' -f5
}

echo "Deploying to $RPC_URL..."

VALIDATOR=$(deploy_to_destination "src/DummyVoteValidator.sol:DummyVoteValidator" "")
DESTINATION=$(deploy_to_destination "src/TestDestination.sol:TestDestination" "")
RELAYER=$(deploy_to_destination "src/DummyRelayerContract.sol:DummyRelayerContract" "--constructor-args $PAYEE")

OUTBOX=$(deploy_to_source "src/SimpleOutbox.sol:Outbox" "")

INBOX_ARGS=$(cast abi-encode "constructor(address,uint256,bytes32)" "$VALIDATOR" "$SOURCE_CHAIN_ID" "$LOCAL_CHAIN_KEY")
INBOX=$(deploy_to_destination "src/DummyInbox.sol:DummyInbox" "--constructor-args $INBOX_ARGS")

DEPLOY_JSON=$(cat <<EOF
{"validator":"$VALIDATOR","inbox":"$INBOX","outbox":"$OUTBOX","destination":"$DESTINATION","relayer":"$RELAYER"}
EOF
)
echo "$DEPLOY_JSON" > ../deployments.json

echo "DummyVoteValidator: $VALIDATOR"
echo "DummyInbox: $INBOX"
echo "Outbox: $OUTBOX"
echo "TestDestination: $DESTINATION"
echo "DummyRelayerContract: $RELAYER"
echo "Wrote ../deployments.json"
