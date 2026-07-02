#!/usr/bin/env bash
# Bring up the full 2-way bridge backend for the UI and KEEP IT RUNNING:
#   anvil + cc3 node + USC contracts + bridge contracts + attestors + proof-gen + relayer.
# Funds your MetaMask address with gas + bridge tokens on both chains and writes the UI config.
#
#   USER_ADDR=0xYourMetaMaskAddress bash usc-messaging/scripts/bridge/dev-stack.sh
#
# Then in another terminal:  cd usc-messaging/bridge-ui && npm run dev
# Ctrl-C here tears everything down.
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
REL="$REPO/target/release"
CONTRACTS="$REPO/usc-messaging/contracts"
UI="$REPO/usc-messaging/bridge-ui"
LOGS=/tmp/bridge-dev-stack
mkdir -p "$LOGS"

USER_ADDR="${USER_ADDR:-}"
[ -n "$USER_ADDR" ] || { echo "Set USER_ADDR=0x<your MetaMask address>"; exit 1; }

cleanup() {
  echo "--- tearing down ---"
  killall -TERM message-relayer proof-gen-api-server attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null
  sleep 3; killall -KILL message-relayer proof-gen-api-server attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null
}
trap cleanup EXIT
killall -KILL message-relayer proof-gen-api-server attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null; sleep 1
rm -f "$REPO/usc-messaging/relayer-checkpoints.json"   # fresh --tmp chain each run

deploy() { # $1 rpc $2 key $3 spec, rest=ctor args -> echoes address
  local rpc=$1 key=$2 spec=$3; shift 3
  (cd "$CONTRACTS" && forge create --rpc-url "$rpc" --private-key "$key" --broadcast "$spec" \
    ${@:+--constructor-args "$@"} 2>&1) | grep -oE 'Deployed to:\s*0x[a-fA-F0-9]{40}' | grep -oE '0x[a-fA-F0-9]{40}' | head -1
}

echo "=== 1. start chains ==="
anvil --block-time 6 --chain-id 31337 --port 8545 >"$LOGS/anvil.log" 2>&1 &
RUST_LOG=info "$REL/creditcoin3-node" --dev --tmp >"$LOGS/cc3-node.log" 2>&1 &
cd "$REPO"
.github/wait-for-ethereum.sh 'http://127.0.0.1:8545' || exit 1
.github/wait-for-creditcoin.sh 'http://127.0.0.1:9944' || exit 1

echo "=== 2. deploy USC + bridge contracts ==="
cd "$REPO/usc-messaging"
npm install >"$LOGS/npm.log" 2>&1
cp .env.example .env
npx tsx scripts/deploy.ts >"$LOGS/deploy.log" 2>&1 || { echo "❌ deploy failed"; tail -20 "$LOGS/deploy.log"; exit 1; }
set -a; source .env; set +a
CC_RPC="$CREDITCOIN_RPC_URL"; CC_KEY="$CREDITCOIN_CHAIN_PRIVATE_KEY"
AN_RPC="$DESTINATION_CHAIN_RPC_URL"; AN_KEY="$DESTINATION_CHAIN_PRIVATE_KEY"
forge build --root "$CONTRACTS" >/dev/null 2>&1
AN_TOK=$(deploy "$AN_RPC" "$AN_KEY" 'src/bridge/BridgeToken.sol:BridgeToken' 'Anvil wCTC' wCTC)
AN_BRIDGE=$(deploy "$AN_RPC" "$AN_KEY" 'src/bridge/AnvilBridge.sol:AnvilBridge' "$AN_TOK" "$INBOX_ADDR")
CC_TOK=$(deploy "$CC_RPC" "$CC_KEY" 'src/bridge/BridgeToken.sol:BridgeToken' 'Creditcoin wCTC' wCTC)
CC_BRIDGE=$(deploy "$CC_RPC" "$CC_KEY" 'src/bridge/CcBridge.sol:CcBridge' "$CC_TOK" "$OUTBOX_ADDR" "$AN_BRIDGE" 2)
[ -n "$AN_BRIDGE" ] && [ -n "$CC_BRIDGE" ] || { echo "❌ bridge deploy failed"; exit 1; }

echo "=== 3. fund $USER_ADDR (gas + tokens) + bridge liquidity ==="
LIQ=1000000000000000000000000; USER_TOK=1000000000000000000000  # 1,000,000 liq / 1,000 user
# gas
cast send "$USER_ADDR" --value 10ether --rpc-url "$AN_RPC" --private-key "$AN_KEY" >/dev/null
cast send "$USER_ADDR" --value 10ether --rpc-url "$CC_RPC" --private-key "$CC_KEY" >/dev/null
# user tokens
cast send "$AN_TOK" 'mint(address,uint256)' "$USER_ADDR" "$USER_TOK" --rpc-url "$AN_RPC" --private-key "$AN_KEY" >/dev/null
cast send "$CC_TOK" 'mint(address,uint256)' "$USER_ADDR" "$USER_TOK" --rpc-url "$CC_RPC" --private-key "$CC_KEY" >/dev/null
# bridge liquidity (release side)
cast send "$AN_TOK" 'mint(address,uint256)' "$AN_BRIDGE" "$LIQ" --rpc-url "$AN_RPC" --private-key "$AN_KEY" >/dev/null
cast send "$CC_TOK" 'mint(address,uint256)' "$CC_BRIDGE" "$LIQ" --rpc-url "$CC_RPC" --private-key "$CC_KEY" >/dev/null

echo "=== 4. write UI config ==="
CC_TOK="$CC_TOK" CC_BRIDGE="$CC_BRIDGE" AN_TOK="$AN_TOK" AN_BRIDGE="$AN_BRIDGE" \
  CC_CHAIN_ID=42 ANVIL_CHAIN_ID=31337 \
  node "$UI/scripts/write-config.mjs" || true

echo "=== 5. attestors + proof-gen + relayer ==="
( bash "$REPO/usc-messaging/scripts/launch-attestors.sh" 3 >"$LOGS/zombienet.log" 2>&1 ) &
for i in $(seq 1 100); do
  grep -q 'Attestor set ready' "$LOGS/zombienet.log" && { echo "✅ attestors ready"; break; }
  grep -qE 'Timed out|zombienet exited|❌' "$LOGS/zombienet.log" && { echo "❌ attestors failed"; tail -30 "$LOGS/zombienet.log"; exit 1; }
  sleep 3; [ "$i" = 100 ] && { echo "❌ attestors timed out"; exit 1; }
done
CHAIN_KEY=2 "$REL/proof-gen-api-server" --cc3-key //Alice --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545 --bind-host 127.0.0.1 --bind-port 3100 >"$LOGS/proof-gen.log" 2>&1 &
for i in $(seq 1 20); do grep -q 'Server listening on' "$LOGS/proof-gen.log" && break; sleep 2; done
ATTESTOR_SET="$(cat "$REPO/usc-messaging/scripts/.attestor-set")"
RUST_LOG=info "$REL/message-relayer" --single-route \
  --cc3-rpc-url ws://localhost:9944 --creditcoin-eth-rpc-url http://localhost:9944 \
  --chain-key 2 --cc3-chain-id 42 --outbox-address "$OUTBOX_ADDR" \
  --destination-rpc-url http://localhost:8545 --inbox-address "$INBOX_ADDR" \
  --signer-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --attestor-set "$ATTESTOR_SET" >"$LOGS/relayer.log" 2>&1 &
for i in $(seq 1 20); do grep -q 'libp2p subscriber online' "$LOGS/relayer.log" && break; sleep 2; done

cat <<EOF

============================================================
✅ BRIDGE DEV STACK READY
   Anvil token / bridge : $AN_TOK / $AN_BRIDGE
   CC    token / bridge : $CC_TOK / $CC_BRIDGE
   Funded user          : $USER_ADDR (gas + 1000 wCTC each chain)

Now run the UI in another terminal:
   cd usc-messaging/bridge-ui && npm run dev   ->  http://localhost:5174

In MetaMask, import a key for $USER_ADDR and add both networks
(the UI's Connect + auto network-switch will prompt you).
Logs: $LOGS    ·    Ctrl-C to tear everything down.
============================================================
EOF
# Keep the stack alive until Ctrl-C.
tail -f "$LOGS/relayer.log"
