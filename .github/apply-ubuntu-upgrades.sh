#!/bin/bash

# disable unattended upgrades to avoid an accidental restart
# which will cancel a currently running CI job
sudo systemctl stop unattended-upgrades.service
sudo systemctl disable unattended-upgrades.service

# Never upgrade the bootloader on an ephemeral CI runner: grub-pc's postinst is interactive
# (device re-selection) and periodically breaks outright on Linode images (2026-08-06: postinst
# exit 1 failed "Provision VM" on every PR). The VM is destroyed after one workflow — a grub
# update can only hurt it.
sudo apt-mark hold grub-pc grub-pc-bin grub2-common grub-common shim-signed || true

sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y
