#!/usr/bin/env bash
#
# verify-block-hash-param.sh
#
# Verifies the Frontier fix that lets the eth JSON-RPC block param accept a bare
# 32-byte block-hash string (as Geth / op-deployer's forking layer send it).
# Before the fix, `visit_str` parsed every `0x`-prefixed string as a hex u64 and
# overflowed on a 66-char hash with: -32602 "Invalid block number: number too
# large to fit in target type". After the fix it resolves to the Hash variant.
#
# Usage:
#   scripts/verify-block-hash-param.sh [RPC_URL]
#
# RPC_URL defaults to http://127.0.0.1:9944
#
# Exit code 0 = all checks passed, non-zero = a check failed.

set -euo pipefail

RPC_URL="${1:-http://127.0.0.1:9944}"

command -v curl >/dev/null || { echo "error: curl not found" >&2; exit 2; }
command -v jq   >/dev/null || { echo "error: jq not found"   >&2; exit 2; }

pass=0
fail=0

# rpc METHOD PARAMS_JSON  -> prints the raw JSON response on stdout
rpc() {
	local method="$1" params="$2"
	curl -s -X POST "$RPC_URL" \
		-H 'Content-Type: application/json' \
		--data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}"
}

# Extract .result or, on error, print the error and return 1.
result_or_err() {
	local resp="$1"
	if echo "$resp" | jq -e '.error' >/dev/null 2>&1; then
		echo "    error: $(echo "$resp" | jq -c '.error')" >&2
		return 1
	fi
	echo "$resp" | jq -r '.result'
}

ok()   { echo "  PASS: $1"; pass=$((pass + 1)); }
bad()  { echo "  FAIL: $1"; fail=$((fail + 1)); }

echo "RPC target: $RPC_URL"
echo

# --- Setup: grab a real recent block (hash + its number) ----------------------
echo "[setup] fetching latest block"
latest_resp="$(rpc eth_getBlockByNumber '["latest", false]')"
BLOCK_HASH="$(echo "$latest_resp"   | jq -r '.result.hash')"
BLOCK_NUMBER="$(echo "$latest_resp" | jq -r '.result.number')"
if [[ -z "$BLOCK_HASH" || "$BLOCK_HASH" == "null" ]]; then
	echo "error: could not read latest block hash; is the node up at $RPC_URL?" >&2
	echo "response: $latest_resp" >&2
	exit 2
fi
echo "  block number: $BLOCK_NUMBER"
echo "  block hash:   $BLOCK_HASH"

# A well-known funded dev account (Alith) — any address works, we only need the
# call to resolve, not a non-zero result.
ADDR="0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac"
echo "  test address: $ADDR"
echo

# --- 1. Direct repro: the exact call op-deployer makes ------------------------
echo "[1] eth_getTransactionCount(addr, <bare block hash>)"
resp="$(rpc eth_getTransactionCount "[\"$ADDR\", \"$BLOCK_HASH\"]")"
if nonce_by_hash="$(result_or_err "$resp")"; then
	ok "resolved to nonce $nonce_by_hash (no overflow error)"
else
	bad "bare block-hash param rejected — the fix is NOT active on this node"
	nonce_by_hash=""
fi
echo

# --- 2a. Correctness: nonce-by-hash == nonce-by-equivalent-number -------------
echo "[2a] cross-check: nonce(by hash) == nonce(by number $BLOCK_NUMBER)"
resp="$(rpc eth_getTransactionCount "[\"$ADDR\", \"$BLOCK_NUMBER\"]")"
if nonce_by_num="$(result_or_err "$resp")"; then
	if [[ -n "$nonce_by_hash" && "$nonce_by_hash" == "$nonce_by_num" ]]; then
		ok "match ($nonce_by_num)"
	else
		bad "mismatch: by-hash=$nonce_by_hash by-number=$nonce_by_num"
	fi
else
	bad "nonce-by-number call failed"
fi
echo

# --- 2b. Sibling state methods share the same resolution path -----------------
echo "[2b] sibling state methods accept the bare block hash"

check_method() {
	local label="$1" method="$2" params="$3"
	local r
	r="$(rpc "$method" "$params")"
	if result_or_err "$r" >/dev/null; then
		ok "$label"
	else
		bad "$label"
	fi
}

check_method "eth_getCode"       eth_getCode       "[\"$ADDR\", \"$BLOCK_HASH\"]"
check_method "eth_getBalance"    eth_getBalance    "[\"$ADDR\", \"$BLOCK_HASH\"]"
check_method "eth_getStorageAt"  eth_getStorageAt  "[\"$ADDR\", \"0x0\", \"$BLOCK_HASH\"]"
check_method "eth_call"          eth_call          "[{\"to\":\"$ADDR\"}, \"$BLOCK_HASH\"]"
echo

# --- 2c. Regression: existing block-param forms still work --------------------
echo "[2c] regression: tag / hex number / EIP-1898 object forms still work"
check_method "tag 'latest'"       eth_getTransactionCount "[\"$ADDR\", \"latest\"]"
check_method "hex number"         eth_getTransactionCount "[\"$ADDR\", \"$BLOCK_NUMBER\"]"
check_method "object {blockHash}" eth_getTransactionCount "[\"$ADDR\", {\"blockHash\":\"$BLOCK_HASH\"}]"
check_method "object {blockNumber}" eth_getTransactionCount "[\"$ADDR\", {\"blockNumber\":\"$BLOCK_NUMBER\"}]"
echo

# --- Summary ------------------------------------------------------------------
echo "-----------------------------------------"
echo "passed: $pass   failed: $fail"
if [[ "$fail" -ne 0 ]]; then
	exit 1
fi
echo "All block-hash block-param checks passed."
