#!/usr/bin/env bash
# Compatibility entry point for existing shortcuts and pre-v0.60 update records.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
if [[ $# == 0 ]]; then
    # Older installed apps invoke this path without the new --launch flag.
    exec bash "$ROOT/scripts/install-linux.sh" --launch
fi
exec bash "$ROOT/scripts/install-linux.sh" "$@"
