#!/usr/bin/env bash
set -euo pipefail

export FAKE_ACCOUNT_TYPE=apiKey
exec "$(dirname "$0")/fake_codex.py" "$@"
