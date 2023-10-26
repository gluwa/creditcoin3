#!/usr/bin/env bash

# This script:
# - Checks for changes in transaction version in runtime/src/version.rs
# - Downloads latest release binary from gluwa/creditcoin (RELEASE_BIN)
# - Compiles and build a binary from the current branch (HEAD_BIN)
# - Runs the two nodes

set -ex

HEAD_BIN=./target/release/creditcoin-next-node
HEAD_WS=ws://localhost:9944
RELEASE_WS=ws://localhost:9955

runtimes=(
  "creditcoin-next-runtime"
)

# First we fetch the latest released binary
latest_release_tag() {
  # WARNING: $GITHUB_TOKEN must be defined in the calling environment because this is a private repository
  curl --silent --header "Authorization: Bearer $GITHUB_TOKEN" "https://api.github.com/repos/$1/releases/latest" | jq -r '.tag_name'
}

latest_release_url() {
  # WARNING: $GITHUB_TOKEN must be defined in the calling environment because this is a private repository
  curl --silent --header "Authorization: Bearer $GITHUB_TOKEN" "https://api.github.com/repos/$1/releases/latest" | jq -r '.url'
}

latest_tag=$(latest_release_tag 'gluwa/creditcoin-next')
latest_url=$(latest_release_url 'gluwa/creditcoin-next')
RELEASE_BIN="./creditcoin-next-node"
echo "[+] Fetching binary for Creditcoin version $latest_tag"
# WARNING: $GITHUB_TOKEN must be defined in the calling environment because this is a private repository
asset_url=$(curl --silent --header "Authorization: Bearer $GITHUB_TOKEN" \
    "${latest_url}" | jq -r ".assets[] | select(.name==\"creditcoin-${latest_tag}-x86_64-unknown-linux-gnu.zip\") | .url"
)
curl --header "Authorization: Bearer $GITHUB_TOKEN" --header "Accept: application/octet-stream" -L \
    "${asset_url}" --output creditcoin.zip
unzip creditcoin.zip
chmod +x "$RELEASE_BIN"
git fetch --depth="${GIT_DEPTH:-100}" origin 'refs/tags/*:refs/tags/*'


for RUNTIME in "${runtimes[@]}"; do
  echo "[+] Checking runtime: ${RUNTIME}"

  release_transaction_version=$(git show "tags/$latest_tag:runtime/src/version.rs" | grep 'transaction_version')

  current_transaction_version=$(
    grep 'transaction_version' "./runtime/src/version.rs"
  )

  echo "[+] Release: ${release_transaction_version}"
  echo "[+] Ours: ${current_transaction_version}"


  # Start running the nodes in the background
  $HEAD_BIN --chain=local --tmp >head-node.log 2>&1 &
  $RELEASE_BIN --chain=local --rpc-port 9955 --tmp --port 30555 >release-node.log 2>&1 &
  jobs

  #Wait for HEAD BINARY
  ./.github/wait-for-creditcoin.sh 'http://127.0.0.1:9944'
  #Wait for RELEASE BINARY
  ./.github/wait-for-creditcoin.sh 'http://127.0.0.1:9955'

  changed_extrinsics=$(
    polkadot-js-metadata-cmp "$RELEASE_WS" "$HEAD_WS" \
      | sed 's/^ \+//g' | grep -e 'idx: [0-9]\+ -> [0-9]\+' || true
  )

  # compare to mainnet and testnet explicitly b/c latest release could be any of them
  # for now this comparison is only used to provide more info in CI
  # polkadot-js-metadata-cmp wss://rpc.cc3-mainnet.creditcoin.network/ws "$HEAD_WS" > metadata-cmp-with-mainnet.txt
  # polkadot-js-metadata-cmp wss://rpc.cc3-testnet.creditcoin.network/ws "$HEAD_WS" > metadata-cmp-with-testnet.txt

  if [ -n "$changed_extrinsics" ]; then
    echo "[!] Extrinsics indexing/ordering has changed in the ${RUNTIME} runtime! If this change is intentional, please bump transaction_version in lib.rs. Changed extrinsics:"
    echo "$changed_extrinsics"

    if [ "$release_transaction_version" == "$current_transaction_version" ]; then
        exit 1
    else
        echo "[+] Transaction version for ${RUNTIME} has been bumped since last release. Exiting."
    fi
  fi

  echo "[+] No change in extrinsics ordering for the ${RUNTIME} runtime"
  jobs -p | xargs kill -9
done
