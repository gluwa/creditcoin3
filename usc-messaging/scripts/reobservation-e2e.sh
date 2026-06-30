#!/usr/bin/env bash
# reobservation-e2e.sh — exercise the liveness-recovery (reobservation) path end-to-end.
#
# Scenario (the "relayer missed every vote" case):
#   1. Bring up anvil + cc3 node, deploy contracts, launch 3 attestors.
#   2. Publish a message with NO relayer running. The attestors index it, sign, and gossip their
#      votes — but nobody is listening, and gossipsub does not replay history.
#   3. Start the relayer with its block-cursor checkpoint rewound to *before* the publish, so it
#      re-indexes the message from chain logs (Case-B catch-up) but holds ZERO votes for it.
#   4. The message sits below quorum. After the stall window the relayer broadcasts a
#      ReobservationRequest; each attestor independently re-verifies the tx on its own RPC, re-signs,
#      and re-gossips. The relayer now reaches 2/3+1 and delivers.
#
# Asserts: the relayer logged a reobservation request, at least one attestor re-signed, and the
# destination dApp actually received the message.
#
#   bash usc-messaging/scripts/reobservation-e2e.sh
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REL="$REPO/target/release"
USC="$REPO/usc-messaging"
CKPT="$USC/relayer-checkpoints.json"
LOGS=/tmp/reobs-e2e
mkdir -p "$LOGS"

cleanup() {
  echo "--- tearing down ---"
  killall -TERM message-relayer attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null
  sleep 3; killall -KILL message-relayer attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null
}
trap cleanup EXIT
killall -KILL message-relayer attestor_zombienet attestor creditcoin3-node anvil 2>/dev/null; sleep 1
rm -f "$CKPT"

for b in creditcoin3-node attestor attestor_zombienet message-relayer; do
  [ -x "$REL/$b" ] || { echo "❌ missing $REL/$b — build with: cargo build --release --features=fast-runtime"; exit 1; }
done

echo "=== 1. start chains ==="
anvil --block-time 2 --chain-id 31337 --port 8545 >"$LOGS/anvil.log" 2>&1 &
RUST_LOG=info "$REL/creditcoin3-node" --dev --tmp >"$LOGS/cc3-node.log" 2>&1 &
cd "$REPO"
.github/wait-for-ethereum.sh 'http://127.0.0.1:8545' || exit 1
.github/wait-for-creditcoin.sh 'http://127.0.0.1:9944' || exit 1

echo "=== 2. deploy USC contracts ==="
cd "$USC"
npm install >"$LOGS/npm.log" 2>&1
cp .env.example .env
npx tsx scripts/deploy.ts >"$LOGS/deploy.log" 2>&1 || { echo "❌ deploy failed"; tail -20 "$LOGS/deploy.log"; exit 1; }
set -a; source .env; set +a

echo "=== 3. launch attestors (no relayer yet) ==="
( bash "$REPO/usc-messaging/scripts/launch-attestors.sh" 3 >"$LOGS/zombienet.log" 2>&1 ) &
for i in $(seq 1 100); do
  grep -q 'Attestor set ready' "$LOGS/zombienet.log" && { echo "✅ attestors ready"; break; }
  grep -qE 'Timed out|zombienet exited|❌' "$LOGS/zombienet.log" && { echo "❌ attestors failed"; tail -30 "$LOGS/zombienet.log"; exit 1; }
  sleep 3; [ "$i" = 100 ] && { echo "❌ attestors timed out"; exit 1; }
done
ATTESTOR_SET="$(cat "$USC/scripts/.attestor-set")"

echo "=== 4. record source block, then publish WITHOUT a relayer ==="
# Cursor to rewind to: everything strictly after this block is rescanned when the relayer starts.
B0="$(cast block-number --rpc-url http://127.0.0.1:9944)"
echo "source block before publish: $B0"
# publish-message.ts publishes the tx then blocks waiting for MessageDelivered (which, by design,
# won't happen until reobservation). The messageId is logged within seconds of publish, so cap it
# with a timeout: the on-chain publish is already committed once the tx confirms.
timeout 45 npx tsx scripts/publish-message/publish-message.ts >"$LOGS/publish.log" 2>&1 || true
MSG_ID="$(grep -m1 'messageId:' "$LOGS/publish.log" | grep -oE '0x[0-9a-fA-F]{64}' | head -1)"
[ -n "$MSG_ID" ] || { echo "❌ could not read messageId from publish"; tail -20 "$LOGS/publish.log"; exit 1; }
echo "published messageId=$MSG_ID"

echo "=== 5. let attestors vote + gossip into the void (relayer absent) ==="
sleep 20
SIGNED=$(grep -lh 'queued message vote for gossip' "$REPO"/logs/attestor-zombie-*.json.* 2>/dev/null | wc -l | tr -d ' ')
echo "attestors that produced a vote with no relayer listening: $SIGNED"

echo "=== 6. rewind relayer checkpoint past the publish, then start the relayer ==="
printf '{\n  "outbox:2": %s\n}\n' "$B0" > "$CKPT"
RUST_LOG=info,message_relayer=debug "$REL/message-relayer" --single-route \
  --cc3-rpc-url ws://localhost:9944 --creditcoin-eth-rpc-url http://localhost:9944 \
  --chain-key 2 --cc3-chain-id 42 --outbox-address "$OUTBOX_ADDR" \
  --destination-rpc-url http://localhost:8545 --inbox-address "$INBOX_ADDR" \
  --signer-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --attestor-set "$ATTESTOR_SET" \
  --checkpoint-path "$CKPT" >"$LOGS/relayer.log" 2>&1 &
for i in $(seq 1 20); do grep -q 'libp2p subscriber online' "$LOGS/relayer.log" && break; sleep 2; done
echo "✅ relayer online (indexing the past message with zero votes)"

echo "=== 7. wait for stall → reobservation → re-sign → delivery (up to ~160s) ==="
DELIVERED=0
for i in $(seq 1 80); do
  CNT="$(cast call "$DESTINATION_CONTRACT_ADDR" 'messageCount()(uint256)' --rpc-url http://127.0.0.1:8545 2>/dev/null || echo 0)"
  if [ "${CNT:-0}" -ge 1 ]; then DELIVERED=1; break; fi
  sleep 2
done

echo
echo "================ RESULT ================"
REQUESTED=$(grep -c 'requesting reobservation for stalled message' "$LOGS/relayer.log" 2>/dev/null || echo 0)
RESIGNED=$(grep -lh 're-signing reobserved message' "$REPO"/logs/attestor-zombie-*.json.* 2>/dev/null | wc -l | tr -d ' ')
echo "relayer reobservation requests : $REQUESTED"
echo "attestors that re-signed       : $RESIGNED"
echo "message delivered (dApp count) : $DELIVERED"

FAIL=0
[ "$REQUESTED" -ge 1 ] || { echo "❌ relayer never requested reobservation"; FAIL=1; }
[ "$RESIGNED" -ge 1 ]  || { echo "❌ no attestor re-signed on reobservation"; FAIL=1; }
[ "$DELIVERED" = 1 ]   || { echo "❌ message was not delivered"; FAIL=1; }

if [ "$FAIL" = 0 ]; then
  echo "✅✅ REOBSERVATION RECOVERY VERIFIED — a fully-missed message was delivered via re-observation"
else
  echo "--- relayer log (tail) ---"; tail -40 "$LOGS/relayer.log"
  echo "--- one attestor log (tail) ---"; tail -25 "$REPO"/logs/attestor-zombie-0.json.* 2>/dev/null
fi
exit $FAIL
