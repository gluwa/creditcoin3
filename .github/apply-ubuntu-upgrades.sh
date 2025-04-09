#!/bin/bash

# disable unattended upgrades to avoid an accidental restart
# which will cancel a currently running CI job
sudo systemctl stop unattended-upgrades.service
sudo systemctl disable unattended-upgrades.service

sudo apt-get update
sudo apt-get upgrade -y
