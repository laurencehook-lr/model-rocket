default: help

help:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

check:
    cargo check --all-targets --all-features --locked
    cargo test --locked --test architecture

lint:
    cargo clippy --all-targets --all-features --locked -- -D warnings

test:
    cargo test --all-targets --all-features --locked -- --skip real_codex_production_adapter_

security-test:
    OPENAI_API_KEY=must-not-reach-child AZURE_OPENAI_API_KEY=must-not-reach-child OPENAI_BASE_URL=https://invalid.example OPENAI_API_BASE=https://invalid.example AWS_ACCESS_KEY_ID=must-not-reach-child AWS_SECRET_ACCESS_KEY=must-not-reach-child AWS_SESSION_TOKEN=must-not-reach-child GITHUB_TOKEN=must-not-reach-child ANTHROPIC_API_KEY=must-not-reach-child ANTHROPIC_AUTH_TOKEN=must-not-reach-child cargo test --features test-support --locked --test app_server_process
    cargo test --features test-support --locked --test http_contract forbidden_builtin_tool_lifecycle_fails_closed
    cargo test --features test-support --locked --test http_contract oversized_app_server_frame_fails_closed
    cargo test --features test-support --locked --test http_contract requests_beyond_one_scheduler_window_are_queued_and_multiplexed
    cargo test --features test-support --locked --test http_contract retained_tool_sessions_do_not_consume_turn_scheduler_capacity
    cargo test --features test-support --locked --test http_contract next_request_restarts_the_shared_process_after_failure
    cargo test --features test-support --locked --test launcher_security

nextest:
    cargo nextest run --all-targets --all-features --locked -E 'not binary(real_codex_tool_surface)'

wire-test:
    cargo test --features test-support --locked --test real_codex_tool_surface

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
    @just wire-test
    @just doc
    @just deny

preflight:
    MODEL_ROCKET_CONFIG="${MODEL_ROCKET_CONFIG:-$PWD/config/model-routes.json}" cargo run --locked -- preflight

serve:
    MODEL_ROCKET_CONFIG="${MODEL_ROCKET_CONFIG:-$PWD/config/model-routes.json}" cargo run --locked -- serve

smoke-claude:
    MODEL_ROCKET_CONFIG="${MODEL_ROCKET_CONFIG:-$PWD/config/model-routes.json}" scripts/smoke-claude.sh
