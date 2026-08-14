#!/bin/bash
# shellcheck source=/dev/null

set -euo pipefail

# Idempotence is based on Docker actually working, NOT on the presence of
# docker.list. The list is written half way through the install, so a run that
# failed after that point used to make the next run skip every remaining step
# and exit 0 with Docker still missing.
if sudo docker info > /dev/null 2>&1; then
    echo "INFO: Docker Engine is already installed and running"
    exit 0
fi

# Best effort: some of these are absent on the image, and `apt-get remove`
# exits non-zero when it cannot locate a package name at all.
for pkg in docker.io docker-doc docker-compose docker-compose-v2 podman-docker containerd runc; do
    sudo apt-get remove -y "$pkg" || true
done

# Add Docker's official GPG key:
sudo apt-get update
sudo apt-get install -y ca-certificates curl
sudo install -m 0755 -d /etc/apt/keyrings
sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
sudo chmod a+r /etc/apt/keyrings/docker.asc

# Add the repository to Apt sources:
echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu \
  $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
  sudo tee /etc/apt/sources.list.d/docker.list > /dev/null
sudo apt-get update

sudo apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

sudo service docker restart
sudo docker run hello-world

# current user can run docker commands
sudo usermod -aG docker "$(id -un)"
