#!/usr/bin/env bash

set -euo pipefail

# Match the compiler previously selected by setup-protoc's 23.x default. Download
# the fixed release directly: release-list discovery intermittently finds no match.
PROTOC_VERSION=23.4
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        PROTOC_ARCH=x86_64
        PROTOC_SHA256=0502f286ac9ed860b629a7965a14527b1f2dd131e4283fa23c2d7f184672aa9a
        ;;
    Linux-aarch64)
        PROTOC_ARCH=aarch_64
        PROTOC_SHA256=1c7750b6e038305b5a7fc3d0cda1ebefdf106a4f30a787bf826ed2fc47c3967d
        ;;
    *)
        echo "Unsupported protoc platform: $(uname -s)-$(uname -m)" >&2
        exit 1
        ;;
esac

PROTOC_DIR="${RUNNER_TEMP:?}/creditcoin-protoc-${PROTOC_VERSION}"
PROTOC_ZIP="${PROTOC_DIR}.zip"
curl --fail --location --retry 3 --retry-all-errors --connect-timeout 15 --max-time 180 \
    "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${PROTOC_ARCH}.zip" \
    --output "$PROTOC_ZIP"
printf '%s  %s\n' "$PROTOC_SHA256" "$PROTOC_ZIP" | sha256sum --check
unzip -oq "$PROTOC_ZIP" -d "$PROTOC_DIR"
"${PROTOC_DIR}/bin/protoc" --version
echo "${PROTOC_DIR}/bin" >> "${GITHUB_PATH:?}"
