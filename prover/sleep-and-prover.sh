#!/bin/bash

# Sleep a number of seconds before starting prover
# to woarkaround an issue with docker-compose where prover starts
# and attempts to deploy a smart contract before the local chain is ready

SLEEP_FOR="${SLEEP_FOR:-0}"

# this is from inside the container
/bin/prover "$@"
