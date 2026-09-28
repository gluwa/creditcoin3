#!/bin/bash

# Regenerate chainspecs/ascDevnetSpec.json and chainspecs/ascDevnetSpecRaw.json from
# asc_devnet_template_config() in node/src/chain_spec.rs.
#
# The genesis wasm is the runtime of the node that runs build-spec, so the node is built
# with the same features release.yml uses for devnet-flavoured tags. The template refuses
# to build from any other flavour.
#
# Changing the template changes the genesis hash: only regenerate before asc-dev launches.

set -euo pipefail

NODE=./target/release/creditcoin3-node
PLAIN=chainspecs/ascDevnetSpec.json
RAW=chainspecs/ascDevnetSpecRaw.json

cargo build --release --features devnet,metadata-hash

"$NODE" build-spec --chain asc_devnet_template --disable-default-bootnode > "$PLAIN"
"$NODE" build-spec --chain "$PLAIN" --raw --disable-default-bootnode > "$RAW"

# build-spec ends without a newline, which pre-commit's end-of-file-fixer would add back.
echo >> "$PLAIN"
echo >> "$RAW"

echo "INFO: wrote $PLAIN and $RAW"
echo "INFO: rebuild the node so asc_devnet_config() embeds the new raw spec"
