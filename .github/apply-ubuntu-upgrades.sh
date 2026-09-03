#!/bin/bash

# disable unattended upgrades to avoid an accidental restart
# which will cancel a currently running CI job
sudo systemctl stop unattended-upgrades.service
sudo systemctl disable unattended-upgrades.service

export DEBIAN_FRONTEND=noninteractive

# Never upgrade the bootloader on an ephemeral CI runner: grub-pc's postinst is interactive
# (device re-selection) and periodically breaks outright — either because debconf's
# install_devices is sometimes the literal string "multiselect" (grub-install then fails
# with "/multiselect does not exist" and leaves dpkg wedged), or the postinst just exits 1
# on Linode images (2026-08-06: failed "Provision VM" on every PR). These are throwaway CI
# VMs that never boot a new kernel from an updated bootloader, so keep grub out of the
# upgrade entirely.
sudo apt-mark hold grub-pc grub-pc-bin grub2-common grub-common shim-signed || true

sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y

# if some other maintainer script still failed, finish configuring what we can
# so a broken dpkg state does not cascade into the Docker and runner installs
sudo dpkg --configure -a || true
