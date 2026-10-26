#!/usr/bin/env bash
# Tests the installer.
source "$(dirname "$0")/lib.sh"

echo "== installer tests"
bash packaging/test-install.sh
