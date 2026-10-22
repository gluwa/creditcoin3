#!/bin/bash

# Capture a try-runtime snapshot of one network's finalized state and publish it.
#
# Runs inside the network's cluster (the try-runtime-snapshot CronJob in cc-networks-iac),
# against an archive node that is already at head. The snapshot holds the chain's existing
# code, never a candidate runtime: every consumer starts from the same file and swaps in
# its own WASM in memory.
#
# Environment:
#   NETWORK                devnet | asc-devnet | testnet | mainnet (a key in config.json)
#   RPC_URL                http(s) RPC of an archive node, e.g. http://cc3-dev-rpc:9944
#   CONFIG                 config.json path (default: /etc/try-runtime/config.json)
#   WORK_DIR               scratch space for the snapshot (default: /work)
#   UPLOAD                 true to publish to Azure Blob, false for a local dry run
#   AZURE_STORAGE_ACCOUNT  storage account (UPLOAD=true)
#   AZURE_CONTAINER        blob container (UPLOAD=true)
#   BLOB_PREFIX            path inside the container (default: try-runtime)
#   KEEP                   snapshots to keep per network and toolchain (default: 14)
#
# Workload identity (AZURE_CLIENT_ID, AZURE_TENANT_ID, AZURE_FEDERATED_TOKEN_FILE) is
# injected by the cluster when the pod carries azure.workload.identity/use.
#
# Published layout, per network and CLI revision:
#   <prefix>/<network>/<cli-rev>/<block-number>-<block-hash>/{state.snap.zst,manifest.json,capture.log}
#   <prefix>/<network>/<cli-rev>/latest.json      written last, only after the rest is up
#
# The storage keys in config.json are twox128(pallet) ++ twox128(item), with
# ":__STORAGE_VERSION__:" as the item for a pallet's on-chain storage version.

set -euo pipefail

NETWORK="${NETWORK:?NETWORK is required}"
RPC_URL="${RPC_URL:?RPC_URL is required}"
CONFIG="${CONFIG:-/etc/try-runtime/config.json}"
WORK_DIR="${WORK_DIR:-/work}"
UPLOAD="${UPLOAD:-true}"
BLOB_PREFIX="${BLOB_PREFIX:-try-runtime}"
KEEP="${KEEP:-14}"

# try-runtime speaks WebSocket; the RPC port serves both.
WS_URL="${RPC_URL/#http/ws}"

# stderr, so a failure inside $(rpc ...) still reaches the job log.
log() { echo "[$(date -u +%Y-%m-%dT%H:%M:%SZ)] $*" >&2; }
fail() { log "FAIL: $*"; exit 1; }

# rpc and le_to_dec. A failed call ends the script through set -e.
# shellcheck source=SCRIPTDIR/lib.sh disable=SC1091
source "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/lib.sh"

[ -f "$CONFIG" ] || fail "config not found at $CONFIG"
# The script moves into WORK_DIR below; a relative CONFIG would then read as missing.
CONFIG="$(cd "$(dirname "$CONFIG")" && pwd)/$(basename "$CONFIG")"
EXPECTED_GENESIS=$(jq -r --arg n "$NETWORK" '.networks[$n].genesis // empty' "$CONFIG")
[ -n "$EXPECTED_GENESIS" ] || fail "network '$NETWORK' is not in $CONFIG"

CLI_REPOSITORY=$(jq -r '.cli.repository' "$CONFIG")
CLI_REV=$(jq -r '.cli.rev' "$CONFIG")
command -v try-runtime >/dev/null || fail "try-runtime is not on PATH"
# The image records the revision it compiled; a config that names another one (a
# mounted override, say) would publish snapshots under the wrong toolchain.
BUILT_REV_FILE=/etc/try-runtime/cli-rev
if [ -f "$BUILT_REV_FILE" ] && [ "$(cat "$BUILT_REV_FILE")" != "$CLI_REV" ]; then
    fail "image was built with try-runtime-cli $(cat "$BUILT_REV_FILE") but config pins $CLI_REV"
fi
CLI_BINARY_SHA256=$(sha256sum "$(command -v try-runtime)" | cut -d' ' -f1)
CLI_VERSION=$(try-runtime --version | head -n1)
TOOLCHAIN="${CLI_REV:0:12}"

mkdir -p "$WORK_DIR"
cd "$WORK_DIR"
rm -f state.snap state.snap.zst capture.log manifest.json latest.json latest.current.json

STARTED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
log "capturing $NETWORK from $RPC_URL with $CLI_VERSION"

# 1. The node must serve the chain we think it does.
OBSERVED_GENESIS=$(rpc chain_getBlockHash '[0]' | jq -r .)
[ "$OBSERVED_GENESIS" = "$EXPECTED_GENESIS" ] \
    || fail "genesis is $OBSERVED_GENESIS, expected $EXPECTED_GENESIS for $NETWORK"

# 2. Pin one finalized block and read everything else at that same hash, so a long
# capture cannot mix state from different heads.
BLOCK_HASH=$(rpc chain_getFinalizedHead '[]' | jq -r .)
HEADER=$(rpc chain_getHeader "[\"$BLOCK_HASH\"]")
BLOCK_NUMBER=$(printf '%d' "$(jq -r .number <<<"$HEADER")")
PARENT_HASH=$(jq -r .parentHash <<<"$HEADER")
STATE_ROOT=$(jq -r .stateRoot <<<"$HEADER")
log "pinned finalized block #$BLOCK_NUMBER $BLOCK_HASH (state root $STATE_ROOT)"

TIMESTAMP_RAW=$(rpc state_getStorage "[\"$(jq -r .storage_keys.timestamp_now "$CONFIG")\", \"$BLOCK_HASH\"]" | jq -r .)
[ "$TIMESTAMP_RAW" != "null" ] || fail "Timestamp::Now is not set at $BLOCK_HASH"
BLOCK_TIMESTAMP_MS=$(le_to_dec "$TIMESTAMP_RAW")

RUNTIME_VERSION=$(rpc state_getRuntimeVersion "[\"$BLOCK_HASH\"]")
CODE_HASH=$(rpc state_getStorageHash "[\"0x3a636f6465\", \"$BLOCK_HASH\"]" | jq -r .)
METADATA_SHA256=$(rpc state_getMetadata "[\"$BLOCK_HASH\"]" | jq -r . | sed 's/^0x//' | xxd -r -p | sha256sum | cut -d' ' -f1)

# 3. Migration-relevant state at the pinned block: every pallet's on-chain storage
# version, and whether a multi-block migration is running (an active cursor means the
# chain is mid-upgrade and the state is not a stable baseline).
STORAGE_VERSIONS='{}'
while IFS=$'\t' read -r pallet key; do
    raw=$(rpc state_getStorage "[\"$key\", \"$BLOCK_HASH\"]" | jq -r .)
    if [ "$raw" = "null" ]; then
        version=0
    else
        version=$(le_to_dec "$raw")
    fi
    STORAGE_VERSIONS=$(jq -c --arg p "$pallet" --argjson v "$version" '. + {($p): $v}' <<<"$STORAGE_VERSIONS")
done < <(jq -r '.storage_keys.pallet_storage_versions | to_entries[] | "\(.key)\t\(.value)"' "$CONFIG")

MBM_CURSOR=$(rpc state_getStorage "[\"$(jq -r .storage_keys.mbm_cursor "$CONFIG")\", \"$BLOCK_HASH\"]")
if [ "$MBM_CURSOR" = "null" ]; then
    MBM_IN_PROGRESS=false
else
    MBM_IN_PROGRESS=true
    log "WARN: a multi-block migration is in progress at this block (cursor $MBM_CURSOR)"
fi

# 4. Capture the full state, child tries included, with the chain's own code.
try-runtime --runtime existing \
    create-snapshot \
    --uri "$WS_URL" \
    --at "$BLOCK_HASH" \
    --child-tree \
    state.snap \
    2>&1 | tee capture.log

[ -s state.snap ] || fail "try-runtime exited without writing state.snap"

# 5. Qualify the capture before anything is published.
#
# The CLI keeps going when the node rejects part of a batch: the key is stored as absent.
# Any such error means the snapshot is missing data.
BATCH_ERRORS=$(grep -c "batch item error" capture.log || true)
[ "$BATCH_ERRORS" -eq 0 ] || fail "$BATCH_ERRORS values were lost during the download"

# The root the CLI computed from what it downloaded must equal the block header's state
# root. That is the proof the snapshot holds the block's complete state, child tries
# included, and nothing else.
COMPUTED_ROOT=$(grep -oE 'initialized state externalities with storage root 0x[0-9a-f]{64}' capture.log \
    | tail -n1 | grep -oE '0x[0-9a-f]{64}' || true)
[ -n "$COMPUTED_ROOT" ] || fail "could not find the computed storage root in the capture log"
[ "$COMPUTED_ROOT" = "$STATE_ROOT" ] \
    || fail "computed storage root $COMPUTED_ROOT does not match the header's $STATE_ROOT"
log "storage root matches the header: $COMPUTED_ROOT"

# The file starts with its SCALE Compact<u16> format version; 0x10 encodes 4.
FORMAT_BYTE=$(head -c1 state.snap | xxd -p)
[ "$FORMAT_BYTE" = "10" ] || fail "unexpected snapshot format byte 0x$FORMAT_BYTE (expected 0x10, format 4)"

RAW_BYTES=$(stat -c %s state.snap)
RAW_SHA256=$(sha256sum state.snap | cut -d' ' -f1)
zstd -q -T0 -6 state.snap -o state.snap.zst
ZST_BYTES=$(stat -c %s state.snap.zst)
ZST_SHA256=$(sha256sum state.snap.zst | cut -d' ' -f1)
COMPLETED_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

BLOCK_DIR="$(printf '%012d' "$BLOCK_NUMBER")-$BLOCK_HASH"
NETWORK_PREFIX="$BLOB_PREFIX/$NETWORK/$TOOLCHAIN"
SNAPSHOT_PREFIX="$NETWORK_PREFIX/$BLOCK_DIR"

# 6. Manifest: everything a consumer needs to decide whether this snapshot fits its run.
jq -n \
    --arg network "$NETWORK" \
    --arg expected_genesis "$EXPECTED_GENESIS" \
    --arg observed_genesis "$OBSERVED_GENESIS" \
    --arg block_hash "$BLOCK_HASH" \
    --argjson block_number "$BLOCK_NUMBER" \
    --arg parent_hash "$PARENT_HASH" \
    --arg state_root "$STATE_ROOT" \
    --argjson block_timestamp_ms "$BLOCK_TIMESTAMP_MS" \
    --argjson runtime_version "$RUNTIME_VERSION" \
    --arg code_hash "$CODE_HASH" \
    --arg metadata_sha256 "$METADATA_SHA256" \
    --argjson storage_versions "$STORAGE_VERSIONS" \
    --argjson mbm_cursor "$MBM_CURSOR" \
    --argjson mbm_in_progress "$MBM_IN_PROGRESS" \
    --arg started_at "$STARTED_AT" \
    --arg completed_at "$COMPLETED_AT" \
    --arg rpc_url "$RPC_URL" \
    --arg image "${IMAGE:-unknown}" \
    --arg source_commit "${SOURCE_COMMIT:-unknown}" \
    --arg cli_repository "$CLI_REPOSITORY" \
    --arg cli_rev "$CLI_REV" \
    --arg cli_version "$CLI_VERSION" \
    --arg cli_binary_sha256 "$CLI_BINARY_SHA256" \
    --arg computed_root "$COMPUTED_ROOT" \
    --arg snapshot_path "$SNAPSHOT_PREFIX/state.snap.zst" \
    --argjson raw_bytes "$RAW_BYTES" \
    --arg raw_sha256 "$RAW_SHA256" \
    --argjson zst_bytes "$ZST_BYTES" \
    --arg zst_sha256 "$ZST_SHA256" \
    '{
        schema_version: 1,
        network: $network,
        genesis: {expected: $expected_genesis, observed: $observed_genesis},
        block: {
            hash: $block_hash,
            number: $block_number,
            parent_hash: $parent_hash,
            state_root: $state_root,
            timestamp_ms: $block_timestamp_ms
        },
        runtime: {
            spec_name: $runtime_version.specName,
            spec_version: $runtime_version.specVersion,
            impl_version: $runtime_version.implVersion,
            transaction_version: $runtime_version.transactionVersion,
            state_version: $runtime_version.stateVersion,
            code_hash: $code_hash,
            metadata_sha256: $metadata_sha256
        },
        migration_state: {
            pallet_storage_versions: $storage_versions,
            mbm_cursor: $mbm_cursor,
            mbm_in_progress: $mbm_in_progress
        },
        capture: {
            started_at: $started_at,
            completed_at: $completed_at,
            rpc_url: $rpc_url,
            image: $image,
            source_commit: $source_commit
        },
        tooling: {
            cli_repository: $cli_repository,
            cli_rev: $cli_rev,
            cli_version: $cli_version,
            cli_binary_sha256: $cli_binary_sha256,
            snapshot_format_version: 4
        },
        coverage: {full_state: true, pallet_filters: [], prefix_filters: [], child_tree: true},
        qualification: {
            computed_root: $computed_root,
            root_matches_header: true,
            batch_item_errors: 0
        },
        artifact: {
            path: $snapshot_path,
            compression: "zstd",
            raw_bytes: $raw_bytes,
            raw_sha256: $raw_sha256,
            zst_bytes: $zst_bytes,
            zst_sha256: $zst_sha256
        }
    }' >manifest.json

jq -n \
    --arg manifest "$SNAPSHOT_PREFIX/manifest.json" \
    --arg block_hash "$BLOCK_HASH" \
    --argjson block_number "$BLOCK_NUMBER" \
    --argjson block_timestamp_ms "$BLOCK_TIMESTAMP_MS" \
    --arg zst_sha256 "$ZST_SHA256" \
    '{manifest: $manifest, block_hash: $block_hash, block_number: $block_number,
      block_timestamp_ms: $block_timestamp_ms, zst_sha256: $zst_sha256}' >latest.json

log "snapshot #$BLOCK_NUMBER: $RAW_BYTES bytes raw, $ZST_BYTES bytes zstd"

if [ "$UPLOAD" != "true" ]; then
    log "UPLOAD=$UPLOAD: leaving state.snap.zst, manifest.json and latest.json in $WORK_DIR"
    exit 0
fi

# 7. Publish. Immutable objects first; the latest pointer moves only after they are all
# up, and never backwards.
STORAGE_ACCOUNT="${AZURE_STORAGE_ACCOUNT:?AZURE_STORAGE_ACCOUNT is required to upload}"
CONTAINER="${AZURE_CONTAINER:?AZURE_CONTAINER is required to upload}"

az login --service-principal \
    --username "$AZURE_CLIENT_ID" \
    --federated-token "$(cat "$AZURE_FEDERATED_TOKEN_FILE")" \
    --tenant "$AZURE_TENANT_ID" \
    --output none

blob() {
    az storage blob "$@" --account-name "$STORAGE_ACCOUNT" --container-name "$CONTAINER" \
        --auth-mode login --only-show-errors
}

for file in state.snap.zst capture.log manifest.json; do
    log "uploading $SNAPSHOT_PREFIX/$file"
    blob upload --name "$SNAPSHOT_PREFIX/$file" --file "$file" --no-progress --output none
done

if blob download --name "$NETWORK_PREFIX/latest.json" --file latest.current.json --no-progress --output none 2>/dev/null; then
    CURRENT_NUMBER=$(jq -r '.block_number // 0' latest.current.json)
else
    CURRENT_NUMBER=0
fi
if [ "$CURRENT_NUMBER" -ge "$BLOCK_NUMBER" ]; then
    log "latest.json already points at #$CURRENT_NUMBER; leaving it"
else
    blob upload --name "$NETWORK_PREFIX/latest.json" --file latest.json --overwrite --no-progress --output none
    log "latest.json now points at #$BLOCK_NUMBER"
fi

# 8. Keep the newest $KEEP snapshots. Directory names start with the zero-padded block
# number, so name order is age order.
mapfile -t SNAPSHOTS < <(blob list --prefix "$NETWORK_PREFIX/" --query "[?ends_with(name, '/manifest.json')].name" -o tsv \
    | sed 's#/manifest.json$##' | sort)
if [ "${#SNAPSHOTS[@]}" -gt "$KEEP" ]; then
    for old in "${SNAPSHOTS[@]:0:${#SNAPSHOTS[@]}-KEEP}"; do
        log "pruning $old"
        az storage blob delete-batch --account-name "$STORAGE_ACCOUNT" --source "$CONTAINER" \
            --pattern "$old/*" --auth-mode login --only-show-errors --output none
    done
fi

log "published $SNAPSHOT_PREFIX"
