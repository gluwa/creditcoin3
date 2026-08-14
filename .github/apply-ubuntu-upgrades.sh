#!/bin/bash

# disable unattended upgrades to avoid an accidental restart
# which will cancel a currently running CI job
sudo systemctl stop unattended-upgrades.service
sudo systemctl disable unattended-upgrades.service

export DEBIAN_FRONTEND=noninteractive

# grub-pc's postinst runs grub-install against debconf's install_devices, which
# on this image is sometimes the literal string "multiselect". grub-install then
# fails with "/multiselect does not exist" and leaves dpkg wedged, which breaks
# every later apt call. These are throwaway CI VMs that never boot a new kernel
# from an updated bootloader, so keep grub out of the upgrade entirely.
sudo apt-mark hold grub-pc grub-pc-bin grub2-common

sudo apt-get update
sudo apt-get upgrade -y

# if some other maintainer script still failed, finish configuring what we can
# so a broken dpkg state does not cascade into the Docker and runner installs
sudo dpkg --configure -a || true
