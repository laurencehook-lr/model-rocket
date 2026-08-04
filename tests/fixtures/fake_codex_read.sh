#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex-cli 0.146.0"
  exit 0
fi

export FAKE_TOOL_NAME=Read
export 'FAKE_TOOL_ARGUMENTS={"file_path":"/workspace/model-rocket/README.md"}'
exec "$(dirname "$0")/fake_codex.py" "$@"
