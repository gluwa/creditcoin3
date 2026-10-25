#!/bin/bash

# NOTE: needs to be executed via sudo

set -euo pipefail

curl -L https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux > /usr/bin/solc

chmod a+x /usr/bin/solc
