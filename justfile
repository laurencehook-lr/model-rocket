default: help

help:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

check:
    cargo check --all-targets --all-features --locked

lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings

test:
    cargo test --all-targets --all-features --locked

security-test:
    OPENAI_API_KEY=must-not-reach-child AZURE_OPENAI_API_KEY=must-not-reach-child OPENAI_BASE_URL=https://invalid.example OPENAI_API_BASE=https://invalid.example AWS_ACCESS_KEY_ID=must-not-reach-child AWS_SECRET_ACCESS_KEY=must-not-reach-child AWS_SESSION_TOKEN=must-not-reach-child GITHUB_TOKEN=must-not-reach-child ANTHROPIC_API_KEY=must-not-reach-child ANTHROPIC_AUTH_TOKEN=must-not-reach-child cargo test --locked --test app_server_process
    cargo test --locked --test http_contract forbidden_builtin_tool_lifecycle_fails_closed
    cargo test --locked --test http_contract oversized_app_server_frame_fails_closed
    cargo test --locked --test http_contract retained_tool_sessions_are_bounded_to_four
    cargo test --locked --test launcher_security
    cargo test --locked --test real_codex_tool_surface

nextest:
    cargo nextest run --all-targets --all-features --locked

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked

deny:
    cargo deny check

verify:
    @just fmt-check
    @just check
    @just lint
    @just test
    @just security-test

verify-full:
    @just verify
    @just nextest
    @just doc
    @just deny

preflight:
    cargo run --locked -- preflight

serve:
    cargo run --locked -- serve

smoke-claude:
    scripts/smoke-claude.sh
