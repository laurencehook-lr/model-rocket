use std::{path::Path, process::Command};

#[test]
fn launcher_isolates_bridge_and_claude_environments() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let launcher_bin = fixtures.join("launcher-bin");
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
        .env("PATH", format!("{}:{path}", launcher_bin.display()))
        .env("LAUNCHER_PROOF_FILE", &proof)
        .env("OPENAI_API_KEY", "must-not-reach-bridge")
        .env("AWS_ACCESS_KEY_ID", "must-not-reach-bridge")
        .env("AWS_SECRET_ACCESS_KEY", "must-not-reach-bridge")
        .env("GITHUB_TOKEN", "must-not-reach-bridge")
        .env("ANTHROPIC_API_KEY", "must-not-reach-either")
        .env("ANTHROPIC_AUTH_TOKEN", "must-not-reach-either")
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
    assert_eq!(evidence.trim(), "launcher environment isolated");
    Ok(())
}
