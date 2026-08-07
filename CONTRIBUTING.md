# Contributing to Model Rocket

Thanks for helping improve Model Rocket.
This project is an experimental local router that keeps Claude Code as the harness while routing selected model requests to Anthropic or the official Codex App Server.

Focused bug fixes, tests, documentation improvements, and small design improvements are welcome.
Please open an issue before starting a large protocol, security-boundary, or architecture design change so the approach can be discussed first.
Report suspected vulnerabilities privately by following [SECURITY.md](SECURITY.md).

## Understand the project boundary

Model Rocket does not patch Claude Code, intercept TLS, install a certificate authority, or accept API keys as router configuration.
It uses pinned native Claude Code and Codex executables, subscription sessions owned by those vendor tools, and a bearer-protected loopback listener.

Read these files before changing behaviour:

- [README.md](README.md) for installation, usage, security boundaries, and current limitations.
- [Bridge specification](docs/specs/bridge-spec.md) for the supported protocol and architecture.
- [Bridge test matrix](docs/specs/bridge-test-matrix.md) for the behavioural verification surface.

Model Rocket is independent of Anthropic and OpenAI.
Contributions must not imply vendor endorsement or remove the documented vendor-policy warning.

## Development setup

The repository pins Rust `1.94.1` in [rust-toolchain.toml](rust-toolchain.toml).
Install Rust through rustup, then install `just` and the CI-pinned test tools:

```bash
cargo install just
cargo install cargo-nextest --version 0.9.132 --locked
cargo install cargo-deny --version 0.19.1 --locked
```

The full wire suite requires the pinned native Codex executable installed as `$HOME/.local/bin/codex-native`; follow the binary-copy step in the [README installation instructions](README.md#quick-start).
The automated wire tests redirect outbound requests to a local capture server, so they need an `auth.json` file but do not need a valid ChatGPT login or paid subscription.
Runtime use and `just preflight` do require the subscription sessions and model entitlements documented in the README.

Clone the repository and inspect the available commands:

```bash
git clone https://github.com/laurencehook-lr/model-rocket.git
cd model-rocket
just --list
```

Build and run the default test suite:

```bash
cargo build --locked
cargo test --locked
```

Run Claude Code against the bridge built from the current checkout:

```bash
cargo build --locked

if [[ "$(uname -s)" == "Darwin" ]]; then
  codesign --force --sign - "$PWD/target/debug/model-rocket"
fi

MODEL_ROCKET_BRIDGE_BIN="$PWD/target/debug/model-rocket" \
MODEL_ROCKET_CONFIG="$PWD/config/model-routes.json" \
scripts/model-rocket
```

This development launch uses your installed vendor CLIs and subscription sessions.
It does not replace the isolated automated tests.

Do not update pinned vendor tools or dependencies incidentally.
A pin change must include its regenerated lockfile or executable digest, focused compatibility tests, and documentation updates.

## Architecture rules

Model Rocket uses ports and adapters with explicit domain, application, contract, policy, adapter, and composition boundaries.

- Domain code must remain independent of HTTP, processes, filesystems, provider DTOs, and async runtime types.
- Ports expose domain value objects rather than primitives, JSON values, paths, channels, or provider types.
- Adapters implement ports and contain infrastructure-specific translation.
- Ports and adapters contain no inline literals or macros.
- Unknown models, fields, events, credentials, and incompatible binaries fail explicitly rather than falling back.
- Claude Code remains responsible for tool execution; the GPT route translates model traffic only.

The architecture suite enforces these constraints mechanically.
Do not weaken or bypass it to make a change pass.

## Making a change

1. Reproduce a bug before fixing it.
2. Add or update focused tests that prove the externally visible contract.
3. Make the smallest complete change that fixes the root cause.
4. Update the README, specification, test matrix, and explainer when their contract changes.
5. Run the focused test while iterating.

Useful focused commands include:

```bash
cargo test --locked --test translation
cargo test --locked --test architecture
cargo test --features test-support --locked --test http_contract
```

Existing tests are contracts.
Do not delete, skip, narrow, or weaken them to obtain a green result.

## Required verification

Format the code, then run the complete local gate before opening a pull request:

```bash
just fmt

model_rocket_test_home="$(mktemp -d)"
trap 'rm -rf -- "$model_rocket_test_home"' EXIT
install -m 600 /dev/null "$model_rocket_test_home/auth.json"
CODEX_HOME="$model_rocket_test_home" just verify-full
```

`just verify-full` covers formatting, compilation, Clippy, architecture rules, default and security tests, Nextest, native Codex wire compatibility, Rustdoc, and dependency policy.
If an environment cannot run part of the gate, state exactly which command was not run and why.
Do not describe the full gate as passing unless every step completed successfully.

For a release-affecting change, also complete the interactive Anthropic-to-GPT-to-Anthropic sequence documented in the [README development section](README.md#development).

## Pull requests

Keep each pull request focused on one problem.
Include:

- the problem and its root cause;
- the design choice and relevant trade-offs;
- tests added or changed;
- exact verification commands and observed results;
- user-facing, security, compatibility, or resource-limit changes.

Never commit subscription credentials, authentication files, bearer values, customer code, captured prompts, or other sensitive data.
Use synthetic fixtures for protocol tests.

By contributing, you agree that your contribution is licensed under the repository's [Apache License 2.0](LICENSE).
