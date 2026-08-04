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

Requirements: Claude Code 2.1.221, Codex CLI 0.146.0 logged in with ChatGPT, Rust 1.94.1, `just`, `curl`, and `openssl`.

```bash
just preflight
scripts/model-rocket
```

For a user-wide command, put `scripts/model-rocket` on `PATH`:

```bash
ln -s "$PWD/scripts/model-rocket" "$HOME/.local/bin/model-rocket"
```

`scripts/model-rocket` starts the router on a free loopback port with a fresh local token, launches unmodified Claude Code, and stops the router when Claude exits.
Run `/model` inside that Claude Code process to switch between available Anthropic models and `gpt-5.6-sol`.
All normal Claude Code tools, hooks, skills, MCP servers, and permission handling remain in the Claude Code process.

To make the normal `claude` command use the router in Zsh, add this line once to `~/.zshrc`, then open a new shell:

```shell
alias claude='model-rocket'
```

Run `claude`, then use `/model` to switch providers in the same session.
Use `\claude` when you explicitly need Claude Code without Model Rocket.
To remove the integration, delete the alias line from `~/.zshrc`.
The router starts with Fable by default.
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
