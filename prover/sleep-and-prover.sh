#!/bin/bash

# Sleep a number of seconds before starting prover
# to woarkaround an issue with docker-compose where prover starts
# and attempts to deploy a smart contract before the local chain is ready

SLEEP_FOR="${SLEEP_FOR:-0}"

date --iso-8601=seconds
echo "DEBUG: will sleep for '$SLEEP_FOR' seconds before starting prover ..."

sleep "$SLEEP_FOR"

date --iso-8601=seconds
echo "DEBUG: starting prover ..."

# this is from inside the container
/bin/prover "$@"
