#!/bin/bash

# NOTE: needs to be executed via sudo

set -euo pipefail

curl -L -H "Cache-Control: no-cache" https://github.com/argotorg/solidity/releases/download/v0.8.31/solc-static-linux > /usr/bin/solc

chmod a+x /usr/bin/solc
