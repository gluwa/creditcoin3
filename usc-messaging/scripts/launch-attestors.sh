#!/usr/bin/env bash
#
# launch-attestors.sh — start the attestor zombienet (WITHOUT --well-known-keys), discover each
# attestor's derived message-vote EVM address from its logs, and write those addresses into both
# attestor/config.yaml (`write_ability.attestors`) and a `--attestor-set` value for the relayer.
#
# It does NOT launch the relayer — run that yourself in a separate terminal (so its logs stay
# visible) using the printed `--attestor-set`. This script stays in the foreground with the
# zombienet's logs, exactly like running the zombienet directly.
#
# Prerequisite: deploy.ts must have already run (the OutboxFactory must be registered on-chain for
# the attestor's chain_key) — otherwise the attestors leave write-ability disabled and never log a
# signer address, and this script will time out.
#
# Usage:
#   bash usc-messaging/scripts/launch-attestors.sh [N]
# where N is the number of attestors (default 3). The chain_key and the source chain the attestors
# attest are read from .env: DESTINATION_CHAIN_KEY (e.g. 2 = local anvil, 3 = Sepolia) and
# DESTINATION_CHAIN_WS_URL (the destination chain's wss:// RPC). So the same command works for the
# local and Sepolia demos — just edit .env. CHAIN_KEY / ETH_URL / CC3_URL / FUNDING_ADDRESS still
# accept an inline override for one-off runs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENV_FILE="$SCRIPT_DIR/../.env"

# Load .env so CHAIN_KEY / ETH_URL / the EOAValidator-sync vars can be derived from it.
if [[ -f "$ENV_FILE" ]]; then
  # shellcheck disable=SC1090
  set -a; source "$ENV_FILE"; set +a
fi

N="${1:-3}"
# Attestor chain_key and the source chain it attests come from .env (DESTINATION_CHAIN_KEY /
# DESTINATION_CHAIN_WS_URL); both still accept an inline override for one-off runs.
CHAIN_KEY="${CHAIN_KEY:-${DESTINATION_CHAIN_KEY:-2}}"          # 2 = local anvil (Anvil1); 3 = Sepolia
ETH_URL="${ETH_URL:-${DESTINATION_CHAIN_WS_URL:-ws://localhost:8545}}"  # must match CHAIN_KEY's chain
CC3_URL="${CC3_URL:-ws://localhost:9944}"
FUNDING_ADDRESS="${FUNDING_ADDRESS:-//Alice}"

ZOMBIENET="$REPO_ROOT/target/release/attestor_zombienet"
ATTESTOR_BIN="$REPO_ROOT/target/release/attestor"
CONFIG="$REPO_ROOT/attestor/config.yaml"
LOGS_DIR="$REPO_ROOT/logs"
ATTESTOR_SET_FILE="$SCRIPT_DIR/.attestor-set"
DEADLINE_SECS=180

for bin in "$ZOMBIENET" "$ATTESTOR_BIN"; do
  if [[ ! -x "$bin" ]]; then
    echo "❌ missing $bin" >&2
    echo "   build it first: cargo build --features=fast-runtime --release" >&2
    exit 1
  fi
done
[[ -f "$CONFIG" ]] || { echo "❌ missing $CONFIG" >&2; exit 1; }
command -v node >/dev/null || { echo "❌ node is required (used to rewrite config.yaml)" >&2; exit 1; }

cd "$REPO_ROOT"
mkdir -p "$LOGS_DIR"

# Remove this run's stale per-attestor logs so we only read addresses from the run we start below.
rm -f "$LOGS_DIR"/attestor-zombie-*.json.* 2>/dev/null || true

# Stop any attestor/zombienet processes left over from a previous (e.g. aborted/timed-out) run.
# Their held p2p/API ports would otherwise make this run's attestors fail with
# "Address already in use". This matches the demo's own binary path only.
if pgrep -f "$ATTESTOR_BIN" >/dev/null 2>&1; then
  echo "🧹 Stopping leftover attestor/zombienet processes from a previous run…"
  pkill -f "$ATTESTOR_BIN" 2>/dev/null || true
  sleep 2
fi

echo "🧟 Launching $N attestor(s) for chain_key $CHAIN_KEY (no --well-known-keys)…"
"$ZOMBIENET" \
  -n "$N" \
  --bin="$ATTESTOR_BIN" \
  --chain-key="$CHAIN_KEY" \
  --eth-url="$ETH_URL" \
  --cc3-url="$CC3_URL" \
  --funding-address="$FUNDING_ADDRESS" \
  --config="$CONFIG" &
ZPID=$!

# Shut the zombienet down with SIGINT (not SIGTERM): it handles ctrl_c by gracefully chilling and
# stopping its child attestors, so they don't orphan and hold ports for the next run.
cleanup() {
  trap - INT TERM
  kill -INT "$ZPID" 2>/dev/null || true
}
trap cleanup INT TERM

echo "⏳ Waiting for attestors to report their message-vote signer addresses (timeout ${DEADLINE_SECS}s)…"
declare -a ADDRS
deadline=$(( $(date +%s) + DEADLINE_SECS ))
for (( i=0; i<N; i++ )); do
  addr=""
  while [[ -z "$addr" ]]; do
    if ! kill -0 "$ZPID" 2>/dev/null; then
      echo "❌ zombienet exited before all signer addresses were found." >&2
      exit 1
    fi
    if (( $(date +%s) > deadline )); then
      echo "❌ Timed out waiting for zombie-$i's signer address." >&2
      echo "   Did you run deploy.ts first? Write-ability stays disabled (no signer logged) until" >&2
      echo "   the OutboxFactory is registered for this chain_key." >&2
      cleanup
      exit 1
    fi
    f="$(ls -t "$LOGS_DIR"/attestor-zombie-"$i".json.* 2>/dev/null | head -1 || true)"
    if [[ -n "$f" ]]; then
      # The "message-vote signer ready" line carries the attestor's derived EVM vote address.
      addr="$(grep -h 'evm_address' "$f" 2>/dev/null | grep -oE '0x[0-9a-fA-F]{40}' | tail -1 || true)"
    fi
    [[ -z "$addr" ]] && sleep 2
  done
  ADDRS[$i]="$addr"
  echo "  zombie-$i → $addr"
done

# Comma-separated set for the relayer's --attestor-set.
SET="$(IFS=,; echo "${ADDRS[*]}")"

# Rewrite the `write_ability.attestors` list in config.yaml (preserving everything else).
node -e '
  const fs = require("fs");
  const [path, set] = process.argv.slice(1);
  const addrs = set.split(",");
  const lines = fs.readFileSync(path, "utf8").split("\n");
  const out = [];
  let i = 0;
  while (i < lines.length) {
    out.push(lines[i]);
    if (/^  attestors:\s*$/.test(lines[i])) {
      i++;
      while (i < lines.length && /^\s+-\s/.test(lines[i])) i++; // drop old entries
      addrs.forEach((a, n) => out.push(`    - "${a}"  # zombie-${n}`));
      continue;
    }
    i++;
  }
  fs.writeFileSync(path, out.join("\n"));
' "$CONFIG" "$SET"
echo "✍️  Updated $CONFIG  (write_ability.attestors)"

printf '%s\n' "$SET" > "$ATTESTOR_SET_FILE"

# Sync the on-chain EOAValidator's attestor set with the addresses we just discovered, so the
# destination Inbox's validateVotes accepts exactly these attestors. deploy.ts seeds the validator
# best-effort (it runs before the attestors); this is the authoritative update. The destination
# deployer (DESTINATION_CHAIN_PRIVATE_KEY) is the validator admin. (.env was sourced at startup.)
if [[ -n "${VOTE_VALIDATOR_ADDR:-}" ]]; then
  if command -v cast >/dev/null 2>&1; then
    echo "🔗 Syncing EOAValidator attestor set on the destination chain ($VOTE_VALIDATOR_ADDR)…"
    if cast send "$VOTE_VALIDATOR_ADDR" 'updateAttestorSet(address[])' "[$SET]" \
        --rpc-url "${DESTINATION_CHAIN_RPC_URL:-http://127.0.0.1:8545}" \
        --private-key "${DESTINATION_CHAIN_PRIVATE_KEY:-}" \
        >/dev/null 2>&1; then
      echo "✅ EOAValidator attestor set updated to the live attestors"
    else
      echo "⚠️  Could not update the EOAValidator set (is VOTE_VALIDATOR_ADDR an EOAValidator from deploy.ts, with this key as admin?). deliverMessage will revert until the set matches. Continuing." >&2
    fi
  else
    echo "⚠️  cast not found on PATH; skipping EOAValidator set sync — run updateAttestorSet(address[]) manually." >&2
  fi
fi

cat <<EOF

✅ Attestor set ready (also saved to $ATTESTOR_SET_FILE):

    --attestor-set $SET

In a SEPARATE terminal, launch the relayer with that flag, e.g.:

  source usc-messaging/.env
  cargo run -p message-relayer -- --single-route \\
    --cc3-rpc-url $CC3_URL \\
    --creditcoin-eth-rpc-url http://localhost:9944 \\
    --chain-key $CHAIN_KEY --cc3-chain-id 42 \\
    --outbox-address "\$OUTBOX_ADDR" \\
    --destination-rpc-url "\$DESTINATION_CHAIN_RPC_URL" \\
    --inbox-address "\$INBOX_ADDR" \\
    --signer-key "\$DESTINATION_CHAIN_PRIVATE_KEY" \\
    --attestor-set $SET

Note: config.yaml was updated after the attestors started, so it takes effect on the next restart;
this run already works because each attestor publishes its own vote and the relayer aggregates them
using --attestor-set above.

🧟 Attestors are running. The zombienet keeps this terminal (Ctrl-C to stop everything), but it
prints little after startup: each attestor writes its own logs (block attestation + write-ability
votes) to per-attestor files, not to stdout. Tail them in another terminal, e.g.:

  tail -F logs/attestor-zombie-0.json.* | hl -P -l i -h spans -h filename -h line-number   # or: tail -F logs/attestor-zombie-0.json.*
EOF

# Hand the terminal back to the zombienet: keep it in the foreground (Ctrl-C stops it).
wait "$ZPID"
