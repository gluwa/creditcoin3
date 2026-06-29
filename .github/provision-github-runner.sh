#!/bin/bash

set -eo pipefail

mkdir actions-runner
pushd actions-runner || exit 1

curl -L https://github.com/actions/runner/releases/download/v2.335.1/actions-runner-linux-x64-2.335.1.tar.gz > runner.tar.gz

tar xzf ./runner.tar.gz
sudo ./bin/installdependencies.sh
# for 3rd party dependencies and building the code
sudo apt install curl gnupg -y
# apt-key is removed in Ubuntu 26.04, so use the signed-by keyring pattern instead.
sudo install -m 0755 -d /etc/apt/keyrings
curl -sS https://dl.yarnpkg.com/debian/pubkey.gpg | sudo gpg --dearmor -o /etc/apt/keyrings/yarn.gpg
sudo chmod a+r /etc/apt/keyrings/yarn.gpg
echo "deb [signed-by=/etc/apt/keyrings/yarn.gpg] https://dl.yarnpkg.com/debian/ stable main" | sudo tee /etc/apt/sources.list.d/yarn.list
sudo apt-get update
sudo apt install -y build-essential clang curl gcc git-lfs jq \
    libclang-dev llvm-dev libpq-dev libssl-dev pipx pkg-config protobuf-compiler \
    unzip yarn

OWNER_REPO_SLUG="${LC_OWNER_REPO_SLUG}"
REPOSITORY_URL="https://github.com/$OWNER_REPO_SLUG"
EPHEMERAL=${LC_RUNNER_EPHEMERAL:-true}

# we need a temporary registration token first
REGISTRATION_TOKEN=$(curl --silent -X POST \
    -H "Accept: application/vnd.github+json" \
    -H "Authorization: Bearer $LC_GITHUB_REPO_ADMIN_TOKEN" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "https://api.github.com/repos/$OWNER_REPO_SLUG/actions/runners/registration-token" | jq -r '.token')

if [ "$REGISTRATION_TOKEN" == "null" ]; then
    echo "ERROR: REGISTRATION_TOKEN is null"
    exit 1
fi

if [ -z "$REGISTRATION_TOKEN" ]; then
    echo "ERROR: REGISTRATION_TOKEN is empty"
    exit 2
fi

# Important: ephemeral runners are removed after a single job is executed on them
# which is inline with the VM lifecycle
./config.sh --unattended --ephemeral "$EPHEMERAL" --url "$REPOSITORY_URL" --token "$REGISTRATION_TOKEN" \
    --name "$LC_RUNNER_VM_NAME" \
    --labels "$LC_RUNNER_VM_NAME,workflow-$LC_WORKFLOW_ID,proxy-$LC_PROXY_ENABLED,secret-$LC_PROXY_SECRET_VARIANT,type-$LC_PROXY_TYPE"
nohup ./run.sh >/dev/null 2>&1 </dev/null &
