#!/bin/bash

set -xeuo pipefail

cat /proc/cpuinfo
free -m
cat /proc/meminfo

./target/release/creditcoin3-node benchmark machine --chain dev
