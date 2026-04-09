#!/bin/bash

# Usage: wait-for-attestors.sh <rpc_url> [chain_key]
# chain_key defaults to 2 for backward compatibility.

TARGET_URL=${1:-http://127.0.0.1:9944}
CHAIN_KEY=${2:-2}

COUNTER=0
RESULT="null"

# Pre-computed storage keys for attestation.activeAttestors(chain_key)
# via the Chain state -> Storage tab in the Substrate Portal
CHAIN_KEY_2="0x6310fed47319b658f9b8b2504e0d72ec605e795422de90908f14285054a6764bfc069c24352798859c017ce862813d3b0200000000000000"
CHAIN_KEY_4="0x6310fed47319b658f9b8b2504e0d72ec605e795422de90908f14285054a6764ba4f5ced6668957bb2a9a954e7e50f5b50400000000000000"

if [[ "$CHAIN_KEY" == "4" ]]; then
    STORAGE_KEY="$CHAIN_KEY_4"
else
    STORAGE_KEY="$CHAIN_KEY_2"
fi

# make sure there is a node running at TARGET_URL
while [[ "$RESULT" == "null" && $COUNTER -lt 15 ]]; do
    RESULT=$(curl -H 'Content-Type: application/json' \
                -d "{\"id\":\"1\", \"jsonrpc\":\"2.0\", \"method\": \"state_getStorage\", \"params\":[\"$STORAGE_KEY\"]}" \
                "$TARGET_URL" | jq -r .result
        )

    echo "INFO: $COUNTER - Not ready yet, RESULT='$RESULT' ....."
    (( COUNTER=COUNTER+1 ))
    sleep 10

done

if [[ $COUNTER -ge 15 ]]; then
    exit 3
fi

echo "INFO: Attestors are ready"

exit 0
