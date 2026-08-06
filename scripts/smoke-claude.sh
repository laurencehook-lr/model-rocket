#!/usr/bin/env bash
set -euo pipefail

if [[ "${MODEL_ROCKET_ALLOW_LIVE_SMOKE:-}" != "1" ]]; then
  echo "Set MODEL_ROCKET_ALLOW_LIVE_SMOKE=1 only in a disposable test environment." >&2
  exit 2
fi

isolated_cwd="$(mktemp -d -t model-rocket-smoke.XXXXXX)"
trap 'rm -rf -- "$isolated_cwd"' EXIT

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bridge_bin="${MODEL_ROCKET_BRIDGE_BIN:-$HOME/.local/bin/model-rocket-bridge}"
model="$($bridge_bin canonical-route)"

response="$({
  MODEL_ROCKET_CWD="$isolated_cwd" \
    "$script_dir/model-rocket" --model "$model" --print "Reply with exactly: model-rocket text smoke passed"
})"
if [[ "$response" != *"model-rocket text smoke passed"* ]]; then
  echo "Claude smoke response did not contain the expected marker." >&2
  exit 1
fi
