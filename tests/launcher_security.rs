use std::{path::Path, process::Command};

#[test]
fn launcher_isolates_bridge_and_claude_environments() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let launcher_bin = fixtures.join("launcher-bin");
    let cmux_cli_shims = fixtures.join("cmux-cli-shims");
    let path = std::env::var("PATH")?;
    let proof = std::env::temp_dir().join(format!(
        "model-rocket-launcher-proof-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7()
    ));
    let output = Command::new(root.join("scripts/model-rocket"))
        .current_dir(root)
        .env(
            "MODEL_ROCKET_BRIDGE_BIN",
            fixtures.join("fake_bridge_env.py"),
        )
        .env("MODEL_ROCKET_CLAUDE_BIN", launcher_bin.join("claude"))
        .env("PATH", format!("{}:{path}", cmux_cli_shims.display()))
        .env("LAUNCHER_PROOF_FILE", &proof)
        .env("OPENAI_API_KEY", "must-not-reach-bridge")
        .env("AWS_ACCESS_KEY_ID", "must-not-reach-bridge")
        .env("AWS_SECRET_ACCESS_KEY", "must-not-reach-bridge")
        .env("GITHUB_TOKEN", "must-not-reach-bridge")
        .env("ANTHROPIC_API_KEY", "must-not-reach-either")
        .env("ANTHROPIC_AUTH_TOKEN", "must-not-reach-either")
        .env("ANTHROPIC_BASE_URL", "https://must-not-win.invalid")
        .env("ANTHROPIC_CUSTOM_HEADERS", "must-not-win")
        .env("ANTHROPIC_CUSTOM_MODEL_OPTION", "must-not-win")
        .env("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME", "must-not-win")
        .env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "1000000")
        .env("HTTP_PROXY", "http://must-not-reach.invalid:8080")
        .env("HTTPS_PROXY", "http://must-not-reach.invalid:8080")
        .env("ALL_PROXY", "http://must-not-reach.invalid:8080")
        .env("NO_PROXY", "existing-upper.example")
        .env("no_proxy", "existing-lower.example")
        .env("CLAUDE_CODE_USE_BEDROCK", "1")
        .env("CLAUDE_CODE_USE_VERTEX", "1")
        .env("CLAUDE_CODE_USE_FOUNDRY", "1")
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "launcher failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    let evidence = std::fs::read_to_string(&proof)?;
    std::fs::remove_file(&proof)?;
    let mut evidence_lines = evidence.lines();
    assert_eq!(evidence_lines.next(), Some("launcher environment isolated"));
    let settings_file = evidence_lines.next().ok_or("missing settings path")?;
    assert!(
        !Path::new(settings_file).exists(),
        "launch-scoped routing settings were not removed"
    );
    assert_eq!(evidence_lines.next(), None);
    Ok(())
}

#[test]
fn launcher_rejects_caller_routing_options() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for argument in ["--settings=hostile.json", "--setting-sources=user"] {
        let output = Command::new(root.join("scripts/model-rocket"))
            .current_dir(root)
            .arg(argument)
            .output()?;

        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8(output.stderr)?.trim(),
            format!("Claude Code routing option is managed by Model Rocket: {argument}")
        );
    }
    Ok(())
}

#[test]
fn launcher_rejects_relative_claude_binary_path() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(root.join("scripts/model-rocket"))
        .current_dir(root)
        .env("MODEL_ROCKET_BRIDGE_BIN", "/usr/bin/true")
        .env("MODEL_ROCKET_CLAUDE_BIN", "claude")
        .output()?;

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr)?.trim(),
        "Claude Code binary path must be absolute: claude"
    );
    Ok(())
}
