#!/bin/bash

# NOTE: needs to be executed via sudo

set -euo pipefail

# -f so an HTTP error page (e.g. a 503 from the release CDN) is treated as a
# failure instead of being written to /usr/bin/solc as if it were the binary.
# --retry* because these release downloads intermittently drop the connection.
curl -fL --retry 5 --retry-all-errors --retry-delay 5 \
    https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux > /usr/bin/solc

chmod a+x /usr/bin/solc
