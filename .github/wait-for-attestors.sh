#!/bin/bash

# Note: both WS and HTTP are served via the same port
TARGET_URL=${1:-http://127.0.0.1:9944}

COUNTER=0
RESULT="0x00000000"
# make sure there is a node running at TARGET_URL
while [[ "$RESULT" != "0x00000000" && $COUNTER -lt 10 ]]; do
    echo "INFO: $COUNTER - Not ready yet, RESULT='$RESULT' ....."
    (( COUNTER=COUNTER+1 ))
    sleep 10

    # this is attestation.counterForAttestors() via the Chain state -> Storage tab in the Substrate Portal
    RESULT=$(curl -H 'Content-Type: application/json' \
                -d '{"id":"1", "jsonrpc":"2.0", "method": "state_getStorage", "params":["0x6310fed47319b658f9b8b2504e0d72ecfc94f595b8112f70a51eb7452df8d7fb"]}' \
                "$TARGET_URL" | jq -r .result
        )
done

if [[ $COUNTER -gt 9 ]]; then
    exit 3
fi

exit 0
