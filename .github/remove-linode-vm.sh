#!/bin/bash

set -x

# Install linode-cli
python3 --version
pipx install linode-cli
export PATH="$PATH:~/.local/bin"
linode-cli --version

# only VMs carrying this tag are ours to manage - anything else (e.g. manually
# created VMs) must never be touched by the cleaner, no matter their age/tags.
RUNNER_TAG="github-provisioned-runner"

# if a specific VM was not specified then remove all zombies
if [ -z "$LC_RUNNER_VM_NAME" ]; then
    THRESHOLD=$(date --utc "+%Y-%m-%dT%H:%M:%S" -d "5 hours ago")

    # dump JSON for debugging purposes
    linode-cli linodes list --json | jq -r
    for VM_ID in $(linode-cli linodes list --json | jq -r ".[] | select(.created <= \"$THRESHOLD\") | select(.tags | index(\"$RUNNER_TAG\")) | .id"); do
        TAGS=$(linode-cli linodes list --id "$VM_ID" --json | jq -r '.[0].tags[]')

        KEEP_UNTIL_TAG=$(printf '%s\n' "$TAGS" | grep '^keep_until_' || true)

        if [ -z "$KEEP_UNTIL_TAG" ]; then
            echo "INFO: $RUNNER_TAG VM $VM_ID has no keep_until tag. Going to remove expired VM $VM_ID"
            linode-cli linodes delete "$VM_ID"
        else
            KEEP_UNTIL=$(echo -n "$KEEP_UNTIL_TAG" | sed s/keep_until_//)
            NOW=$(date --utc "+%Y-%m-%dT%H:%M:%S")

            if [[ "$KEEP_UNTIL" < "$NOW" ]]; then
                echo "INFO: $NOW is past $KEEP_UNTIL. Going to remove expired VM $VM_ID"
                linode-cli linodes delete "$VM_ID"
            else
                echo "INFO: $NOW is before $KEEP_UNTIL. NOT removing VM $VM_ID"
            fi
        fi
    done
else
    VM_ID=$(linode-cli linodes list --json --label "$LC_RUNNER_VM_NAME" | jq -r '.[0].id')
    linode-cli linodes delete "$VM_ID"
fi
