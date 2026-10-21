#!/usr/bin/env bash
# Start a local attestor fleet, discover its public identities, and synchronize the destination
# registry. E2E_* paths isolate concurrent runs; only this launcher's child process is stopped.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENV_FILE="${E2E_ENV_FILE:-$SCRIPT_DIR/../.env}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

N="${1:-3}"
if [[ ! "$N" =~ ^[0-9]+$ ]] || (( N < 3 )); then
  echo "❌ at least 3 attestors are required" >&2
  exit 1
fi
CHAIN_KEY="${CHAIN_KEY:-${DESTINATION_CHAIN_KEY:-2}}"
ETH_URL="${ETH_URL:-${DESTINATION_CHAIN_WS_URL:-ws://localhost:8545}}"
CC3_URL="${CC3_URL:-${CREDITCOIN_SUBSTRATE_WS_URL:-ws://localhost:9944}}"
FUNDING_ADDRESS="${FUNDING_ADDRESS:-//Alice}"
ZOMBIENET="${E2E_ZOMBIENET_BIN:-$REPO_ROOT/target/release/attestor_zombienet}"
ATTESTOR_BIN="${E2E_ATTESTOR_BIN:-$REPO_ROOT/target/release/attestor}"
STATE_DIR="${E2E_STATE_DIR:-$REPO_ROOT/.attestor-state}"
CONFIG="${E2E_ATTESTOR_CONFIG:-$STATE_DIR/attestor.yaml}"
LOGS_DIR="${E2E_ATTESTOR_LOGS:-$REPO_ROOT/logs}"
ATTESTOR_SET_FILE="${E2E_ATTESTOR_SET_FILE:-$SCRIPT_DIR/.attestor-set}"
READY_PATH="${E2E_ATTESTOR_READY_PATH:-$STATE_DIR/attestor-ready.json}"
DEADLINE_SECS="${E2E_ATTESTOR_TIMEOUT_SECS:-240}"
P2P_PORT_BASE="${E2E_ATTESTOR_P2P_PORT_BASE:-9000}"
API_PORT_BASE="${E2E_ATTESTOR_API_PORT_BASE:-9100}"

for bin in "$ZOMBIENET" "$ATTESTOR_BIN"; do
  [[ -x "$bin" ]] || { echo "❌ missing executable $bin" >&2; exit 1; }
done
for tool in node cast; do
  command -v "$tool" >/dev/null || { echo "❌ $tool is required" >&2; exit 1; }
done
: "${VOTE_VALIDATOR_ADDR:?set VOTE_VALIDATOR_ADDR to the deployed EOAValidator}"
: "${DESTINATION_CHAIN_PRIVATE_KEY:?set the destination registry owner key}"
DEST_RPC="${DESTINATION_CHAIN_RPC_URL:-http://127.0.0.1:8545}"
mkdir -p "$LOGS_DIR" "$STATE_DIR" "$(dirname "$CONFIG")" "$(dirname "$ATTESTOR_SET_FILE")" "$(dirname "$READY_PATH")"
# These are this launcher's scoped output files, never process-name-wide cleanup.
rm -f "$READY_PATH" "$ATTESTOR_SET_FILE" "$LOGS_DIR"/attestor-zombie-*.json.*

# Configure the on-chain committee before any attestor starts. The source template remains
# untouched; each run has its own state/config, and attestors hot-reload the registry update below.
node --input-type=module - "$REPO_ROOT/attestor/config.yaml" "$CONFIG" "$VOTE_VALIDATOR_ADDR" "$STATE_DIR" <<'JS'
import fs from "node:fs";
const [source, output, validator, state] = process.argv.slice(2);
if (!/^0x[0-9a-fA-F]{40}$/.test(validator)) throw new Error("invalid validator address");
const text = fs.readFileSync(source, "utf8");
const start = text.search(/^write_ability:\s*$/m);
if (start < 0) throw new Error("write_ability config section is missing");
const tail = text.slice(start).split("\n");
const end = tail.findIndex((line, i) => i > 0 && /^[a-zA-Z_][\w-]*:/.test(line));
const block = end < 0 ? tail : tail.slice(0, end);
const set = (key, value) => {
  const index = block.findIndex(line => new RegExp(`^  ${key}:`).test(line));
  if (index < 0) block.push(`  ${key}: ${value}`);
  else block[index] = `  ${key}: ${value}`;
};
set("enabled", "true");
set("validator_address", JSON.stringify(validator));
set("state_dir", JSON.stringify(state));
set("start_block", "1");
fs.writeFileSync(output, text.slice(0, start) + block.concat(end < 0 ? [] : tail.slice(end)).join("\n"));
JS

cd "$REPO_ROOT"
echo "🧟 Launching $N attestors for chain_key $CHAIN_KEY with on-chain committee discovery…"
KEY_ARGS=()
# The isolated E2E runner uses public development identities so it can collect pre-revocation
# approvals. Other launchers retain the existing randomly generated identity behavior.
if [[ -n "${E2E_ATTESTOR_SEEDS_FILE:-}" ]]; then
  KEY_ARGS+=(--seeds-file="$E2E_ATTESTOR_SEEDS_FILE")
fi
"$ZOMBIENET" -n "$N" --bin="$ATTESTOR_BIN" --chain-key="$CHAIN_KEY" \
  --eth-url="$ETH_URL" --cc3-url="$CC3_URL" --funding-address="$FUNDING_ADDRESS" \
  --config="$CONFIG" --p2p-port-base="$P2P_PORT_BASE" --api-port-base="$API_PORT_BASE" \
  "${KEY_ARGS[@]}" -- --logs="$LOGS_DIR" &
ZPID=$!
cleanup() { kill -INT "$ZPID" 2>/dev/null || true; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

declare -a ADDRS BOOT_NODES
deadline=$(( $(date +%s) + DEADLINE_SECS ))
for (( i=0; i<N; i++ )); do
  while true; do
    kill -0 "$ZPID" 2>/dev/null || { echo "❌ zombienet exited before readiness" >&2; exit 1; }
    (( $(date +%s) <= deadline )) || { echo "❌ Timed out waiting for zombie-$i; check governance Discovery registration and RPCs" >&2; exit 1; }
    identities="$(node --input-type=module - "$LOGS_DIR" "$i" "$((P2P_PORT_BASE + i))" <<'JS'
import fs from "node:fs";
import path from "node:path";
const [directory, index, port] = process.argv.slice(2);
let address, bootNode;
for (const file of fs.readdirSync(directory).filter(f => f.startsWith(`attestor-zombie-${index}.json.`)).sort()) {
  for (const line of fs.readFileSync(path.join(directory, file), "utf8").split("\n")) {
    let fields;
    try { fields = JSON.parse(line).fields; } catch { continue; }
    if (fields?.message?.includes("message-vote signer ready") && /^0x[0-9a-fA-F]{40}$/.test(fields.evm_address ?? "")) address = fields.evm_address;
    if (fields?.message?.includes("new listen addr")) {
      const match = String(fields.address ?? "").match(/^\/ip4\/[^/]+\/tcp\/(\d+)\/p2p\/([A-Za-z0-9]+)$/);
      if (match?.[1] === port) bootNode = `/ip4/127.0.0.1/tcp/${port}/p2p/${match[2]}`;
    }
  }
}
if (address && bootNode) console.log(`${address} ${bootNode}`);
JS
)"
    if [[ -n "$identities" ]]; then
      read -r addr boot_node <<< "$identities"
      ADDRS[i]="$addr"
      BOOT_NODES[i]="$boot_node"
      echo "  zombie-$i → $addr ($boot_node)"
      break
    fi
    sleep 2
  done
done
SET="$(IFS=,; echo "${ADDRS[*]}")"
PEERS="$(IFS=,; echo "${BOOT_NODES[*]}")"
REGISTRY_ADDR="$(cast call "$VOTE_VALIDATOR_ADDR" 'attestorRegistry()(address)' --rpc-url "$DEST_RPC")"
cast send "$REGISTRY_ADDR" 'updateAttestorSet(address[])' "[$SET]" \
  --rpc-url "$DEST_RPC" --private-key "$DESTINATION_CHAIN_PRIVATE_KEY" >/dev/null
ACTUAL="$(cast call "$VOTE_VALIDATOR_ADDR" 'attestors()(address[])' --rpc-url "$DEST_RPC")"
THRESHOLD="$(cast call "$VOTE_VALIDATOR_ADDR" 'threshold()(uint256)' --rpc-url "$DEST_RPC")"
node --input-type=module - "$SET" "$PEERS" "$ACTUAL" "$THRESHOLD" "$READY_PATH" "$ATTESTOR_SET_FILE" <<'JS'
import fs from "node:fs";
const [set, peers, actual, threshold, ready, setFile] = process.argv.slice(2);
const addresses = set.split(",");
const normalized = list => list.map(a => a.toLowerCase()).sort();
const expected = normalized(addresses);
if (new Set(expected).size !== addresses.length || JSON.stringify(normalized(actual.match(/0x[0-9a-fA-F]{40}/g) ?? [])) !== JSON.stringify(expected)) throw new Error("live EOAValidator committee does not match attestors");
if (BigInt(threshold.split(/\s/)[0]) !== BigInt(Math.floor(addresses.length * 2 / 3) + 1)) throw new Error("unexpected validator threshold");
fs.writeFileSync(setFile, set + "\n");
fs.writeFileSync(ready + ".tmp", JSON.stringify({ addresses, bootNodes: peers.split(",") }, null, 2));
fs.renameSync(ready + ".tmp", ready);
JS
echo "✅ EOAValidator attestor set updated to the live attestors"
echo "✅ Attestor set ready — identities and bootstrap peers in $READY_PATH"
wait "$ZPID"
