#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  exec "$(dirname "$0")/fake_codex.py" "$@"
fi

for name in \
  OPENAI_API_KEY \
  AZURE_OPENAI_API_KEY \
  OPENAI_BASE_URL \
  OPENAI_API_BASE \
  AWS_ACCESS_KEY_ID \
  AWS_SECRET_ACCESS_KEY \
  AWS_SESSION_TOKEN \
  GITHUB_TOKEN \
  ANTHROPIC_API_KEY \
  ANTHROPIC_AUTH_TOKEN; do
  if [[ -n "${!name+x}" ]]; then
    echo "forbidden environment variable reached child: $name" >&2
    exit 42
  fi
done

if [[ "$HOME" != "$CODEX_HOME" || "$CODEX_HOME" != */model-rocket-codex-home-* ]]; then
  echo "Codex child did not receive an isolated home" >&2
  exit 43
fi
if [[ ! -L "$CODEX_HOME/auth.json" ]]; then
  echo "Codex child did not receive the managed-auth symlink" >&2
  exit 44
fi
if ! awk '
  $0 == "[agents]" { in_agents = 1; next }
  in_agents && /^\[/ { exit found ? 0 : 1 }
  in_agents && $0 == "enabled = false" { found = 1 }
  END { exit found ? 0 : 1 }
' "$CODEX_HOME/config.toml"; then
  echo "isolated Codex config did not disable agents" >&2
  exit 45
fi
if [[ " $* " != *" --disable multi_agent_v2 "* || " $* " != *" model_catalog_json="* ]]; then
  echo "Codex child did not receive tool-surface restrictions" >&2
  exit 46
fi

exec "$(dirname "$0")/fake_codex.py" "$@"
