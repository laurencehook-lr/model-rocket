# Model Rocket

Model Rocket lets one unmodified Claude Code process switch between Anthropic models and `gpt-5.6-sol` with `/model`.
Claude Code talks to one loopback router, which sends each `claude-*` request to Anthropic and the configured GPT request to the official Codex App Server.

## Current scope

- Anthropic models through Claude Code's existing subscription OAuth
- GPT through Codex-managed ChatGPT authentication
- Both providers in Claude Code's `/model` picker
- Optional `gpt-worktree-worker` subagent for Fable to delegate GPT work in isolated git worktrees
- Per-request model routing in one Claude Code process
- Incremental GPT streaming with final App Server token usage
- Claude Code interactive and non-interactive (`-p`) response modes
- Structured JSON output for Claude Code auxiliary requests
- Session-bound Claude Code tool calls and tool results
- Conservative `max_tokens` enforcement
- Claude function tools through App Server dynamic tools
- Tool-result continuation on the original App Server turn
- Loopback-only HTTP with a launch-scoped local bearer
- Explicit ChatGPT-account and model preflight
- Isolated Codex process with only Claude-supplied tools and bounded sessions

The bridge does not patch Claude Code, intercept TLS, install a CA, persist vendor tokens, or accept API keys.

## Use

Requirements: Claude Code 2.1.222, Codex CLI 0.146.0 logged in with ChatGPT, Rust 1.94.1, `just`, `curl`, and `openssl`.

```bash
just preflight
scripts/model-rocket
```

For a user-wide command, put `scripts/model-rocket` on `PATH`:

```bash
ln -s "$PWD/scripts/model-rocket" "$HOME/.local/bin/model-rocket"
```

`scripts/model-rocket` starts the router on a free loopback port with a fresh local token, launches the pinned Claude Code binary directly, and stops the router when Claude exits.
The direct binary path prevents terminal integrations such as cmux from replacing the router with their own `claude` command shim.
The launcher defaults to `$HOME/.local/bin/claude`; set `MODEL_ROCKET_CLAUDE_BIN` to another absolute path if the official binary is installed elsewhere.
The launcher supplies provider routing through a private launch-scoped settings file, rejects caller settings flags that could replace it, and removes the file when Claude exits.
User, project, and local settings remain available for normal Claude Code features but cannot replace the route.
Managed enterprise policy keeps its documented higher precedence; Model Rocket does not bypass administrator policy and is incompatible with a managed provider or proxy policy that replaces its loopback route or loopback proxy bypass.
Run `/model` inside that Claude Code process to switch between available Anthropic models and `gpt-5.6-sol`.
All normal Claude Code tools, hooks, skills, MCP servers, and permission handling remain in the Claude Code process.
Claude Code is told that the custom GPT model has the current 272,000-token Sol context window.

The router starts with Fable by default.

### Migrating from Clodex

Clodex short model aliases such as `sol` are not Model Rocket model identifiers.
If `~/.claude/settings.json` still contains `"model": "sol"`, replace it with `"model": "claude-fable-5"` before starting Model Rocket.
Sessions created with the old alias retain `sol` in their transcript and must not be resumed through Model Rocket.
Start a new Model Rocket session and select the exact `gpt-5.6-sol` custom model through `/model`.

Install the optional user-level worker once:

```bash
mkdir -p "$HOME/.claude/agents"
cp config/gpt-worktree-worker.md "$HOME/.claude/agents/"
```

Then ask Fable `Use @gpt-worktree-worker to implement <task>`.
Claude Code starts that GPT worker in a temporary git worktree while Fable remains the lead session.

This is an experimental compatibility layer, not a vendor-supported subscription integration.
Review the current Anthropic and OpenAI subscription terms and obtain company security approval before using it with company source or credentials.

## Development

```bash
just verify-full
just preflight
```

See [the first-edition specification](docs/specs/bridge-spec.md).

## Licence

Model Rocket is licensed under the [Apache License 2.0](LICENSE).
