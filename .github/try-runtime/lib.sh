# shellcheck shell=bash
# Helpers shared by capture.sh and check-baseline.sh. Source it; it sets no options.
#
# Requires RPC_URL (an HTTP JSON-RPC endpoint) to be set before rpc is called.

# One JSON-RPC call; prints the result as JSON. A JSON-RPC error (a rate limit, say) comes
# back as HTTP 200, and reading its missing .result as null would turn a failed cursor read
# into "no migration running" and a failed version read into version 0. So anything but a
# response with a result and no error fails; only a genuine `"result": null` means absent
# storage.
rpc() {
    local response
    response=$(curl -sS --fail --max-time 120 -H 'Content-Type: application/json' \
        -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2}" "$RPC_URL") || {
        echo "FAIL: RPC $1 could not be reached at $RPC_URL" >&2
        return 1
    }
    if ! jq -e 'has("result") and (has("error") | not)' <<<"$response" >/dev/null 2>&1; then
        echo "FAIL: RPC $1 returned $(jq -c '.error // .' <<<"$response" 2>/dev/null || echo "$response")" >&2
        return 1
    fi
    jq -c '.result' <<<"$response"
}

# SCALE little-endian unsigned integer (hex, 0x-prefixed) to decimal.
le_to_dec() {
    local hex="${1#0x}" be="" i
    for ((i = ${#hex} - 2; i >= 0; i -= 2)); do
        be+="${hex:i:2}"
    done
    printf '%d' "0x${be:-0}"
}
