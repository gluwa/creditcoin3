#!/bin/bash

# NOTE: needs to be executed via sudo

# Installs the Azure CLI from Microsoft's apt repository.
#
# We pin the repository to a fixed Ubuntu codename instead of using
# https://aka.ms/InstallAzureCLIDeb, which derives the codename from the
# running system. Our self-hosted runners moved to linode/ubuntu26.04
# (resolute), and Microsoft publishes dist metadata for resolute but ships no
# packages in it:
#
#     dists/resolute/main/binary-amd64/Packages -> 200, 0 bytes
#     dists/noble/main/binary-amd64/Packages    -> 200, 22 KB
#
# So InRelease downloads fine and apt then reports "Unable to locate package
# azure-cli". Pinning is safe because the azure-cli deb bundles its own Python
# and depends only on C libraries (libc6, libffi8, libssl3t64, libuuid1,
# zlib1g, libbz2-1.0), all of which resolute still ships.
#
# Revisit REPO_CODENAME once Microsoft publishes for a newer release.

set -euo pipefail

REPO_CODENAME="noble"
KEYRING="/etc/apt/keyrings/microsoft.gpg"

mkdir -p /etc/apt/keyrings

curl -sLS https://packages.microsoft.com/keys/microsoft.asc \
    | gpg --dearmor --yes -o "$KEYRING"
chmod go+r "$KEYRING"

cat > /etc/apt/sources.list.d/azure-cli.list <<EOF
deb [arch=$(dpkg --print-architecture) signed-by=$KEYRING] https://packages.microsoft.com/repos/azure-cli/ $REPO_CODENAME main
EOF

apt-get update
apt-get install -y azure-cli

az version
