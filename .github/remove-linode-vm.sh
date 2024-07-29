#!/bin/bash

set -x

# Install linode-cli
python3 --version
pipx install linode-cli
export PATH="$PATH:~/.local/bin"
linode-cli --version

VM_ID=$(linode-cli linodes list --json --label "$LC_RUNNER_VM_NAME" | jq -r '.[0].id')
linode-cli linodes delete "$VM_ID"
