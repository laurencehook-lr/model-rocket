#!/usr/bin/env bash
set -euo pipefail

if [[ "${MODEL_ROCKET_ALLOW_LIVE_SMOKE:-}" != "1" ]]; then
  echo "Set MODEL_ROCKET_ALLOW_LIVE_SMOKE=1 only in a disposable test environment." >&2
  exit 2
fi

if [[ -z "${MODEL_ROCKET_BEARER:-}" ]]; then
  echo "MODEL_ROCKET_BEARER must be set to a launch-scoped random value." >&2
  exit 2
fi

: "${MODEL_ROCKET_LISTEN:=127.0.0.1:0}"
model="gpt-5.6-sol"

isolated_cwd="$(mktemp -d -t model-rocket-smoke.XXXXXX)"
ready_file="$(mktemp -t model-rocket-smoke.XXXXXX.ready)"
bridge_pid=""
trap 'kill -INT "$bridge_pid" 2>/dev/null || true; wait "$bridge_pid" 2>/dev/null || true; rm -rf -- "$isolated_cwd"; rm -f "$ready_file"' EXIT

MODEL_ROCKET_CWD="$isolated_cwd" MODEL_ROCKET_READY_FILE="$ready_file" cargo run --locked -- serve >model-rocket.log 2>&1 &
bridge_pid=$!

for _attempt in {1..50}; do
  if [[ -s "$ready_file" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -s "$ready_file" ]]; then
  cat model-rocket.log >&2
  echo "Bridge did not publish its listening address." >&2
  exit 1
fi

listen_addr="$(<"$ready_file")"

env -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN \
  ANTHROPIC_BASE_URL="http://${listen_addr}" \
  ANTHROPIC_CUSTOM_HEADERS="x-model-rocket-token: $MODEL_ROCKET_BEARER" \
  ANTHROPIC_CUSTOM_MODEL_OPTION="$model" \
  ANTHROPIC_CUSTOM_MODEL_OPTION_NAME="GPT through Codex subscription" \
  claude --model "$model" --print "Reply with exactly: model-rocket text smoke passed"
