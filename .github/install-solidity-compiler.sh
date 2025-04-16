#!/bin/bash

# NOTE: needs to be executed via sudo

set -euo pipefail

add-apt-repository ppa:ethereum/ethereum
apt-get update
apt-get install solc
