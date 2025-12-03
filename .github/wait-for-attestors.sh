#!/bin/bash

# Note: both WS and HTTP are served via the same port
TARGET_URL=${1:-http://127.0.0.1:9944}

COUNTER=0
RESULT="null"

# Choose the correct storage key based on the URL
# this is attestation.activeAttestors(DEV_CHAIN_ID) via the Chain state -> Storage tab in the Substrate Portal
CHAIN_KEY_2="0x6310fed47319b658f9b8b2504e0d72ec605e795422de90908f14285054a6764bfc069c24352798859c017ce862813d3b0200000000000000"
CHAIN_KEY_4="0x6310fed47319b658f9b8b2504e0d72ec605e795422de90908f14285054a6764ba4f5ced6668957bb2a9a954e7e50f5b50400000000000000"

if [[ "$TARGET_URL" == "http://127.0.0.1:9944" ]]; then
    STORAGE_KEY="$CHAIN_KEY_2"
else
    STORAGE_KEY="$CHAIN_KEY_4"
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
