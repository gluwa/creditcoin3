#!/bin/bash

# Note: both WS and HTTP are served via the same port
TARGET_URL=${1:-http://127.0.0.1:9944}

COUNTER=0
RESULT="null"

# make sure there is a node running at TARGET_URL
while [[ "$RESULT" == "null" && $COUNTER -lt 15 ]]; do
    # this is attestation.activeAttestors(DEV_CHAIN_ID) via the Chain state -> Storage tab in the Substrate Portal
    RESULT=$(curl -H 'Content-Type: application/json' \
                -d '{"id":"1", "jsonrpc":"2.0", "method": "state_getStorage", "params":["0x6310fed47319b658f9b8b2504e0d72ec605e795422de90908f14285054a6764bfc069c24352798859c017ce862813d3b0200000000000000"]}' \
                "$TARGET_URL" | jq -r .result
        )

    echo "INFO: $COUNTER - Not ready yet, RESULT='$RESULT' ....."
    (( COUNTER=COUNTER+1 ))
    sleep 10

done

if [[ $COUNTER -ge 15 ]]; then
    exit 3
fi

exit 0
