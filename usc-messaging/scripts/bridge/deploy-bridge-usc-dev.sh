#!/usr/bin/env bash
# Deploy the 2-way bridge on cc3-usc-dev (Creditcoin) <-> Sepolia, wired to the USC Outbox/Inbox
# already deployed by scripts/deploy.ts, and write the bridge-ui config.
#
# Prereqs: foundry (forge/cast), and a completed messaging deploy (.env has OUTBOX_ADDR / INBOX_ADDR
# populated). Uses the SAME RPCs/keys from .env as the messaging deploy.
#
#   cd usc-messaging && bash scripts/bridge/deploy-bridge-usc-dev.sh
#   # optional: fund an address with bridge tokens on both chains:
#   USER_ADDR=0xYourAddr bash scripts/bridge/deploy-bridge-usc-dev.sh
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "${BASH_SOURCE[0]}")/../.."   # -> usc-messaging
set -a; source .env; set +a

CONTRACTS="contracts"; UI="bridge-ui"
: "${OUTBOX_ADDR:?run scripts/deploy.ts first (OUTBOX_ADDR empty)}"
: "${INBOX_ADDR:?run scripts/deploy.ts first (INBOX_ADDR empty)}"

CC_RPC="$CREDITCOIN_RPC_URL";        CC_KEY="$CREDITCOIN_CHAIN_PRIVATE_KEY"
SEP_RPC="$DESTINATION_CHAIN_RPC_URL"; SEP_KEY="$DESTINATION_CHAIN_PRIVATE_KEY"
CHAIN_KEY="${DESTINATION_CHAIN_KEY:-7}"        # Sepolia write-ability chain_key on usc-dev
CC_CHAIN_ID="${SOURCE_CHAIN_ID:-102035}"       # Creditcoin usc-dev EVM chain id
SEP_CHAIN_ID=11155111                          # Sepolia

deploy() { # $1 rpc  $2 key  $3 spec  rest=ctor-args  -> echoes deployed address
  local rpc=$1 key=$2 spec=$3; shift 3
  # Match the "Deployed to:" line specifically — a bare 0x{40} regex also matches the first 40 hex
  # of the 64-char tx hash, which would capture the wrong value.
  (cd "$CONTRACTS" && forge create --rpc-url "$rpc" --private-key "$key" --broadcast "$spec" \
    ${@:+--constructor-args "$@"} 2>&1) \
    | grep -oiE 'Deployed to:?[[:space:]]*0x[a-fA-F0-9]{40}' | grep -oE '0x[a-fA-F0-9]{40}' | head -1
}

echo "=== 1. deploy bridge tokens + bridges (Sepolia first — CcBridge needs its address) ==="
SEP_TOK=$(deploy "$SEP_RPC" "$SEP_KEY" 'src/bridge/BridgeToken.sol:BridgeToken' 'Sepolia wCTC' wCTC)
# AnvilBridge = the destination-side bridge (generic EVM; receiveMessage is gated to the Inbox).
SEP_BRIDGE=$(deploy "$SEP_RPC" "$SEP_KEY" 'src/bridge/AnvilBridge.sol:AnvilBridge' "$SEP_TOK" "$INBOX_ADDR")
CC_TOK=$(deploy "$CC_RPC" "$CC_KEY" 'src/bridge/BridgeToken.sol:BridgeToken' 'Creditcoin wCTC' wCTC)
# CcBridge(token, outbox, destBridge, destChainKey) — destChainKey=7 for claim() native proofs.
CC_BRIDGE=$(deploy "$CC_RPC" "$CC_KEY" 'src/bridge/CcBridge.sol:CcBridge' \
  "$CC_TOK" "$OUTBOX_ADDR" "$SEP_BRIDGE" "$CHAIN_KEY")
[ -n "$SEP_BRIDGE" ] && [ -n "$CC_BRIDGE" ] || { echo "❌ bridge deploy failed"; exit 1; }

echo "=== 2. seed bridge liquidity (the release side of each direction) ==="
LIQ=1000000000000000000000000   # 1,000,000
cast send "$SEP_TOK" 'mint(address,uint256)' "$SEP_BRIDGE" "$LIQ" --rpc-url "$SEP_RPC" --private-key "$SEP_KEY" >/dev/null
cast send "$CC_TOK"  'mint(address,uint256)' "$CC_BRIDGE"  "$LIQ" --rpc-url "$CC_RPC"  --private-key "$CC_KEY"  >/dev/null
if [ -n "${USER_ADDR:-}" ]; then
  echo "   funding $USER_ADDR with 1000 bridge tokens on each chain (native gas is on you)"
  cast send "$SEP_TOK" 'mint(address,uint256)' "$USER_ADDR" 1000000000000000000000 --rpc-url "$SEP_RPC" --private-key "$SEP_KEY" >/dev/null
  cast send "$CC_TOK"  'mint(address,uint256)' "$USER_ADDR" 1000000000000000000000 --rpc-url "$CC_RPC"  --private-key "$CC_KEY"  >/dev/null
fi

echo "=== 3. write bridge-ui config ==="
# Note: every value here is PUBLIC (this file is served to the browser). Sepolia reads go straight
# to publicnode from the browser (it serves CORS *); proxying it through Vercel gets the proxy's
# egress IPs 403'd. The CC RPC keeps the same-origin /rpc/cc proxy.
cat > "$UI/public/bridge-config.json" <<JSON
{
  "destChainKey": $CHAIN_KEY,
  "proofGenPath": "/proofgen",
  "chains": {
    "creditcoin": {
      "name": "Creditcoin (usc-dev)",
      "chainId": $CC_CHAIN_ID,
      "metamaskRpcUrl": "https://rpc.usc-devnet.creditcoin.network",
      "readRpcPath": "/rpc/cc",
      "currencySymbol": "CTC",
      "token": "$CC_TOK",
      "bridge": "$CC_BRIDGE"
    },
    "dest": {
      "name": "Sepolia",
      "chainId": $SEP_CHAIN_ID,
      "metamaskRpcUrl": "https://ethereum-sepolia-rpc.publicnode.com",
      "readRpcPath": "https://ethereum-sepolia-rpc.publicnode.com",
      "readRpcFallbacks": [
        "https://sepolia.drpc.org",
        "https://1rpc.io/sepolia",
        "https://sepolia.gateway.tenderly.co"
      ],
      "currencySymbol": "SepETH",
      "token": "$SEP_TOK",
      "bridge": "$SEP_BRIDGE"
    }
  }
}
JSON

# Persist addresses to .env for reference / the claim script.
sed -i.bak '/^BRIDGE_/d' .env && rm -f .env.bak
{ echo "BRIDGE_CC_TOKEN=$CC_TOK"; echo "BRIDGE_CC=$CC_BRIDGE"
  echo "BRIDGE_SEP_TOKEN=$SEP_TOK"; echo "BRIDGE_SEP=$SEP_BRIDGE"; } >> .env

cat <<EOF

============================================================
✅ Bridge deployed + wired to the USC messaging layer (chain_key $CHAIN_KEY)
   Creditcoin token / bridge : $CC_TOK / $CC_BRIDGE   (outbox $OUTBOX_ADDR)
   Sepolia    token / bridge : $SEP_TOK / $SEP_BRIDGE  (inbox  $INBOX_ADDR)
   UI config written         : $UI/public/bridge-config.json

CC -> Sepolia works once attestors + relayer are live (message path).
Sepolia -> CC works once proof-gen serves chain_key $CHAIN_KEY (native proof path).
UI: point bridge-ui/vite.config.ts proxy /rpc/cc + /rpc/anvil + /proofgen at the usc-dev
    endpoints (or serve behind a proxy that does), then \`cd bridge-ui && npm run dev\`.
============================================================
EOF
