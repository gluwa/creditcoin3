#!/bin/bash

set -x

# Install linode-cli
python3 --version
pip install -r .github/requirements.txt
linode-cli --version

# retry action re-runs this in the same workspace; reset prior attempt's state
rm -f ~/.ssh/id_rsa ~/.ssh/id_rsa.pub
git checkout -- .github/authorized_keys .github/linode-cloud-init.template

# Authorize hosted-runner
mkdir -p ~/.ssh/
ssh-keygen -q -t rsa -N '' -f ~/.ssh/id_rsa
cat ~/.ssh/id_rsa.pub >>.github/authorized_keys

# Provision VM
echo "INFO: From ENVs: RUNNER_VM_NAME=$LC_RUNNER_VM_NAME"

# inject authorized keys into cloud-init for the `ubuntu@` user
while read -r LINE; do
  echo "      - $LINE" >>.github/linode-cloud-init.template
done <.github/authorized_keys

echo "INFO: checking for a leftover VM from a prior attempt ..."
EXISTING_VM_ID=$(linode-cli linodes list --json --label "$LC_RUNNER_VM_NAME" | jq -r '.[0].id // empty')
if [ -n "$EXISTING_VM_ID" ]; then
  echo "INFO: deleting leftover VM $EXISTING_VM_ID (label $LC_RUNNER_VM_NAME) from a prior attempt"
  linode-cli linodes delete "$EXISTING_VM_ID"

  # delete is async - creating a VM with the same label before it finishes
  # fails on a label conflict, since Linode labels must be unique.
  echo "INFO: waiting for $EXISTING_VM_ID to finish deleting ..."
  for attempt in $(seq 1 12); do
    STILL_THERE=$(linode-cli linodes list --json --label "$LC_RUNNER_VM_NAME" | jq -r '.[0].id // empty')
    [ -z "$STILL_THERE" ] && break
    echo "DEBUG: $EXISTING_VM_ID still present on attempt $attempt, retrying ..."
    sleep 5
  done
fi

# retry until we get a VM
IP_ADDRESS=""
COUNTER=0
while [ -z "$IP_ADDRESS" ]; do
  # if all jobs retry rate-limited queries at the same time they still hit the limit
  # and subsequently fail. Max number of retries is hard-coded to 3 in linodecli
  # use up to 60 sec random delay to avoid everything being scheduled at once!
  sleep $((RANDOM % 60))

  VM_KIND=${VM_KIND:-github-provisioned-runner}
  echo "INFO: VM_KIND=$VM_KIND"

  KEEP_UNTIL=${KEEP_UNTIL:-$(date --utc "+%Y-%m-%dT%H:%M:%S" -d "+5 hours")}
  echo "INFO: KEEP_UNTIL=$KEEP_UNTIL UTC"

  # WARNING: we do not specify --authorized_keys for root b/c
  # linode-cli expects each key as a separate argument and iteratively constructing
  # the argument list hits issues with quoting the jey values b/c of white-space.
  # All SSH logins should be via the `ubuntu@` user. For more info see:
  # https://www.linode.com/community/questions/21290/how-to-pass-multiple-ssh-public-keys-with-linode-cli-linodes-create
  linode-cli linodes create --json \
    --image 'linode/ubuntu26.04' --region "$LINODE_REGION" \
    --type "$LINODE_VM_SIZE" --label "$LC_RUNNER_VM_NAME" \
    --root_pass "$(uuidgen)" --backups_enabled false --booted true --private_ip false \
    --tags "$VM_KIND" --tags "keep_until_$KEEP_UNTIL" \
    --metadata.user_data "$(base64 --wrap 0 <.github/linode-cloud-init.template)" >"retry_$COUNTER.json"

  IP_ADDRESS=$(jq -r '.[0].ipv4[0]' <"retry_$COUNTER.json")

  ((COUNTER = COUNTER + 1))
done

# provision the GitHub Runner binary on the VM
# passing additional ENV values
SSH_USER_AT_HOSTNAME="ubuntu@$IP_ADDRESS"
echo "INFO: $SSH_USER_AT_HOSTNAME"

# ssh to the VM with connect/keepalive timeouts, so a half-open connection to a
# still-booting VM fails fast enough to be retried instead of hanging.
vm_ssh() {
  ssh -i ~/.ssh/id_rsa -o StrictHostKeyChecking=no \
    -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 "$@"
}

# make sure we have ssh connectivity first by retrying multiple times
echo "INFO: checking for ssh connectivity ..."
until vm_ssh "$SSH_USER_AT_HOSTNAME" cat /etc/os-release; do
  echo "DEBUG: retrying ssh connection ..."
  sleep 30
done

# One successful ssh does NOT mean the VM is ready: sshd accepts a connection
# while cloud-init is still configuring, then restarts, and the next connections
# die with "kex_exchange_identification: Connection reset by peer". Wait for
# cloud-init to actually finish before running anything that matters.
# `status --wait` blocks until cloud-init finishes, but exits non-zero if it
# ended up in an error/degraded state. Bound the retries so a degraded VM cannot
# spin here until the job's 15 minute timeout - the steps below retry anyway.
echo "INFO: waiting for cloud-init to finish ..."
for attempt in 1 2 3 4 5; do
  if vm_ssh "$SSH_USER_AT_HOSTNAME" 'sudo cloud-init status --wait'; then
    break
  fi
  echo "DEBUG: cloud-init not settled on attempt $attempt, retrying ..."
  sleep 15
done

# Run a provisioning script on the VM, retrying transient ssh/apt failures.
# Only safe for idempotent scripts - see the runner registration below.
run_remote_script() {
  local script="$1"
  local attempt
  for attempt in 1 2 3; do
    if vm_ssh "$SSH_USER_AT_HOSTNAME" <"$script"; then
      return 0
    fi
    echo "DEBUG: $script failed on attempt $attempt, retrying ..."
    sleep 30
  done
  echo "ERROR: $script still failing after 3 attempts"
  return 1
}

# explicitly upgrade before doing anything else to prevent accidental restarts
echo "INFO: attempting Ubuntu upgrade ..."
run_remote_script .github/apply-ubuntu-upgrades.sh || true

# WARNING: commands below won't be retried if they fail b/c we want to
# detect such failures and not continue further
set -euo pipefail

echo "INFO: installing upstream Docker Engine ..."
run_remote_script .github/install-docker-engine-from-upstream.sh

# NOTE: not retried internally (still safe under the step-level retry in
# deploy-runner.yml - provision-github-runner.sh's config.sh uses --replace,
# so re-registering under the same name on a retried attempt is not an error).
echo "INFO: provisioning GitHub runner ..."
vm_ssh -o SendEnv=LC_GITHUB_REPO_ADMIN_TOKEN,LC_RUNNER_VM_NAME,LC_WORKFLOW_ID,LC_PROXY_ENABLED,LC_PROXY_SECRET_VARIANT,LC_PROXY_TYPE \
  "$SSH_USER_AT_HOSTNAME" <.github/provision-github-runner.sh
