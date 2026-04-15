#!/usr/bin/env bash
# Deploy USC contracts via forge create, reading from the repo .env
# and writing deployed addresses back into that same .env.
#
# Usage:
#   ./script/deploy.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$CONTRACTS_DIR/.." && pwd)"
ENV_FILE="$REPO_ROOT/.env"

cd "$CONTRACTS_DIR"

# Load .env if present
if [ -f "$ENV_FILE" ]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

# Hard coded or derived values
SOURCE_CHAIN_ID="${SOURCE_CHAIN_ID:-42}"
LOCAL_CHAIN_KEY="${LOCAL_CHAIN_KEY:-0x0000000000000000000000000000000000000000000000000000000000000001}"
PAYEE="$(cast wallet address --private-key "$CREDITCOIN_CHAIN_PRIVATE_KEY")"

deploy_to_destination() {
  local out addr
  out=$(forge create --rpc-url "$DESTINATION_CHAIN_RPC_URL" --private-key "$DESTINATION_CHAIN_PRIVATE_KEY" --broadcast "$@" 2>&1)
  echo "$out" >&2
  addr=$(echo "$out" | grep -oE "Deployed to: 0x[a-fA-F0-9]{40}" | awk '{print $3}')
  [ -n "$addr" ] || { echo "Failed to parse deployed address for $1" >&2; exit 1; }
  echo "$addr"
}

deploy_to_source() {
  local out addr
  out=$(forge create --rpc-url "$CREDITCOIN_RPC_URL" --private-key "$CREDITCOIN_CHAIN_PRIVATE_KEY" --broadcast "$@" 2>&1)
  echo "$out" >&2
  addr=$(echo "$out" | grep -oE "Deployed to: 0x[a-fA-F0-9]{40}" | awk '{print $3}')
  [ -n "$addr" ] || { echo "Failed to parse deployed address for $1" >&2; exit 1; }
  echo "$addr"
}

update_env_var() {
  local key="$1"
  local value="$2"

  python3 - "$ENV_FILE" "$key" "$value" <<'PY'
import re
import sys
from pathlib import Path

env_path = Path(sys.argv[1])
key = sys.argv[2]
value = sys.argv[3]

if env_path.exists():
    text = env_path.read_text()
else:
    text = ""

pattern = re.compile(rf'^{re.escape(key)}=.*$', re.MULTILINE)
new_line = f'{key}="{value}"'

if pattern.search(text):
    text = pattern.sub(new_line, text)
else:
    if text and not text.endswith("\n"):
        text += "\n"
    text += new_line + "\n"

env_path.write_text(text)
PY
}

echo "Deploying to source: $CREDITCOIN_RPC_URL, destination: $DESTINATION_CHAIN_RPC_URL..."

# Source chain
RELAYER=$(deploy_to_source "src/DummyRelayerContract.sol:DummyRelayerContract" \
  --constructor-args "$PAYEE")
OUTBOX=$(deploy_to_source "src/SimpleOutbox.sol:Outbox")
DAPP=$(deploy_to_source "src/SimpleDApp.sol:SimpleDApp" \
  --constructor-args "$OUTBOX")

# Destination chain
VALIDATOR=$(deploy_to_destination "src/DummyVoteValidator.sol:DummyVoteValidator")
DESTINATION=$(deploy_to_destination "src/TestDestination.sol:TestDestination")
INBOX=$(deploy_to_destination "src/SimpleInbox.sol:SimpleInbox" \
  --constructor-args "$VALIDATOR" "$SOURCE_CHAIN_ID" "$LOCAL_CHAIN_KEY")

# Write addresses back into .env
update_env_var "INBOX_ADDR" "$INBOX"
update_env_var "VOTE_VALIDATOR_ADDR" "$VALIDATOR"
update_env_var "DESTINATION_CONTRACT_ADDR" "$DESTINATION"
update_env_var "OUTBOX_ADDR" "$OUTBOX"
update_env_var "RELAYER_CONTRACT_ADDR" "$RELAYER"
update_env_var "DAPP_CONTRACT_ADDR" "$DAPP"

echo "DummyVoteValidator: $VALIDATOR"
echo "SimpleInbox: $INBOX"
echo "SimpleOutbox: $OUTBOX"
echo "SimpleDApp: $DAPP"
echo "TestDestination: $DESTINATION"
echo "DummyRelayerContract: $RELAYER"
echo "Updated $ENV_FILE"