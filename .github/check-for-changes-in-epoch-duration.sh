#!/bin/bash

set -euo pipefail

# Colorful output.
function greenprint {
    echo -e "\033[1;32m[$(date -Isecond)] ${1}\033[0m"
}

function yellowprint {
    echo -e "\033[1;33m[$(date -Isecond)] ${1}\033[0m"
}

function redprint {
    echo -e "\033[1;31m[$(date -Isecond)] ${1}\033[0m"
}

check_block_time() {
    # WARNING: exits on error
    from=$1
    to=$2

    if git --no-pager diff "${from}...${to}" | grep 'MILLISECS_PER_BLOCK'; then
        redprint "FAIL: modified line(s) referencing MILLISECS_PER_BLOCK found!"
        redprint "FAIL: Don't change the value of this variable! This will brick the blockchain!"
        exit 1
    else
        greenprint "PASS: modified lines referencing MILLISECS_PER_BLOCK not found!"
    fi
}

check_blocks_for_faster_epoch() {
    # WARNING: exits on error
    from=$1
    to=$2

    if git --no-pager diff "${from}...${to}" | grep 'BLOCKS_FOR_FASTER_EPOCH'; then
        redprint "FAIL: modified line(s) referencing BLOCKS_FOR_FASTER_EPOCH found!"
        redprint "FAIL: Don't change the value of this variable! This will brick Devnet!"
        exit 1
    else
        greenprint "PASS: modified lines referencing BLOCKS_FOR_FASTER_EPOCH not found!"
    fi
}

check_epoch_duration() {
    # WARNING: exits on error
    from=$1
    to=$2

    if git --no-pager diff "${from}...${to}" | grep 'EPOCH_DURATION'; then
        redprint "FAIL: modified line(s) referencing EPOCH_DURATION found!"
        redprint "FAIL: Don't change the value of this variable! This will brick the blockchain!"
        exit 1
    else
        greenprint "PASS: modified lines referencing EPOCH_DURATION not found!"
    fi
}

check_slot_duration() {
    # WARNING: exits on error
    from=$1
    to=$2

    if git --no-pager diff "${from}...${to}" | grep 'SLOT_DURATION'; then
        redprint "FAIL: modified line(s) referencing SLOT_DURATION found!"
        redprint "FAIL: Don't change the value of this variable! This will brick the blockchain!"
        exit 1
    else
        greenprint "PASS: modified lines referencing SLOT_DURATION not found!"
    fi
}


#### main part

FROM=$(git rev-parse "${1:-origin/dev}")
TO=$(git rev-parse "${2:-HEAD}")

yellowprint "DEBUG: Inspecting range $FROM...$TO"

if [ -z "$FROM" ]; then
    redprint "ERROR: FROM is empty. Exiting..."
    exit 2
fi

if [ -z "$TO" ]; then
    redprint "ERROR: TO is empty. Exiting..."
    exit 2
fi

if git --no-pager diff --name-only "${FROM}"..."${TO}" | grep -e '^runtime'; then
    yellowprint "INFO: runtime/ dir has been modified. Checking for critical changes!"
    check_block_time "${FROM}" "${TO}"
    check_blocks_for_faster_epoch "${FROM}" "${TO}"
    check_epoch_duration "${FROM}" "${TO}"
    check_slot_duration "${FROM}" "${TO}"
else
    greenprint "INFO: runtime/ dir has NOT been modified. All good!"
fi
