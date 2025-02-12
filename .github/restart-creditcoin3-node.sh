#!/usr/bin/env bash

# This script:
# - stops a running creditcoin3-node process
# - then starts a new process using exactly the same command\

set -ex

PID=$(pidof creditcoin3-node)
CMD=$(tr '\0' ' ' < "/proc/$PID/cmdline")
STDOUT=$(readlink -f "/proc/$PID/fd/1")

for i in $(seq 5); do
    echo "INFO: stopping creditcoin3-node ... $i"
    killall -TERM creditcoin3-node

    # wait a bit; prover will retry connection
    sleep 3

    # start the node again by accounting for output redirection
    $CMD 1>>"$STDOUT" 2>&1 &

    .github/wait-for-creditcoin.sh 'http://127.0.0.1:9944'
done
