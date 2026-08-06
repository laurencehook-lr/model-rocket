#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex-cli 0.145.0"
  exit 0
fi

exit 72
