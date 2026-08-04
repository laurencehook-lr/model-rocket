#!/usr/bin/env bash
set -euo pipefail

if [[ "${MODEL_ROCKET_ALLOW_LIVE_SMOKE:-}" != "1" ]]; then
  echo "Set MODEL_ROCKET_ALLOW_LIVE_SMOKE=1 only in a disposable test environment." >&2
  exit 2
fi

model="gpt-5.6-sol"

isolated_cwd="$(mktemp -d -t model-rocket-smoke.XXXXXX)"
trap 'rm -rf -- "$isolated_cwd"' EXIT

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

MODEL_ROCKET_CWD="$isolated_cwd" MODEL_ROCKET_DEFAULT_MODEL="$model" \
  "$script_dir/model-rocket" --print "Reply with exactly: model-rocket text smoke passed"
