#!/bin/sh
set -eu
if [ -d /Library/Developer/CommandLineTools ]; then
  export DEVELOPER_DIR="${DEVELOPER_DIR:-/Library/Developer/CommandLineTools}"
fi
if command -v cargo >/dev/null 2>&1; then exec cargo "$@"; fi
export RUSTUP_HOME=/private/tmp/advanced-explorer-rustup
export CARGO_HOME=/private/tmp/advanced-explorer-cargo
export PATH="$CARGO_HOME/bin:$PATH"
exec cargo "$@"
