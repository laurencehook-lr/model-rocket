use std::fs;

use model_rocket::claude_settings;
use model_rocket::domain::{
    ClaudeModelId, CodexModelId, ContextTokens, ModelCatalogue, ModelDefinition, ModelRoute,
    ReasoningEffort, ServiceTier,
};
use serde_json::Value;

#[test]
fn accepts_missing_and_empty_model_overrides() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("accepted");
    fs::create_dir_all(&directory)?;
    let empty = directory.join("empty.json");
    fs::write(&empty, r#"{"modelOverrides":{}}"#)?;

    let result = claude_settings::validate([directory.join("missing.json"), empty]);
    fs::remove_dir_all(directory)?;

    result?;
    Ok(())
}

#[test]
fn rejects_non_empty_model_overrides() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("rejected");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(
        &settings,
        r#"{"modelOverrides":{"claude-fable-5":"unexpected-route"}}"#,
    )?;

    let result = claude_settings::validate([&settings]);
    fs::remove_dir_all(directory)?;

    let error = match result {
        Ok(()) => return Err("modelOverrides were accepted".into()),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        format!(
            "configuration error: Claude modelOverrides is incompatible with Model Rocket: {}",
            settings.display()
        )
    );
    Ok(())
}

#[test]
fn rejects_malformed_settings() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("malformed");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(&settings, b"{")?;

    let result = claude_settings::validate([&settings]);
    fs::remove_dir_all(directory)?;

    let error = match result {
        Ok(()) => return Err("malformed settings were accepted".into()),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .starts_with("configuration error: cannot parse Claude settings ")
    );
    Ok(())
}

#[test]
fn rejects_oversized_settings_without_unbounded_reading() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = scratch_directory("oversized");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(&settings, vec![b' '; 1024 * 1024 + 1])?;

    let result = claude_settings::validate([&settings]);
    fs::remove_dir_all(directory)?;

    let error = result.err().ok_or("oversized settings were accepted")?;
    assert!(error.to_string().contains("settings exceed 1048576 bytes"));
    Ok(())
}

#[test]
fn rejects_disabled_hooks() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("disabled-hooks");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(&settings, r#"{"disableAllHooks":true}"#)?;

    let result = claude_settings::validate([&settings]);
    fs::remove_dir_all(directory)?;

    let error = match result {
        Ok(()) => return Err("disabled hooks were accepted".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("disableAllHooks=true"));
    Ok(())
}

#[test]
fn config_change_guard_blocks_model_overrides() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("guard-block");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(
        &settings,
        r#"{"modelOverrides":{"claude-fable-5":"unexpected-route"}}"#,
    )?;
    let input = format!(
        r#"{{"source":"project_settings","file_path":{}}}"#,
        serde_json::to_string(&settings)?
    );
    let mut output = Vec::new();

    claude_settings::guard_config_change(input.as_bytes(), &mut output)?;
    fs::remove_dir_all(directory)?;

    let decision: Value = serde_json::from_slice(&output)?;
    assert_eq!(
        decision.get("decision").and_then(Value::as_str),
        Some("block")
    );
    assert!(
        decision
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.contains("modelOverrides"))
    );
    Ok(())
}

#[test]
fn config_change_guard_allows_safe_settings() -> Result<(), Box<dyn std::error::Error>> {
    let directory = scratch_directory("guard-allow");
    fs::create_dir_all(&directory)?;
    let settings = directory.join("settings.json");
    fs::write(&settings, r#"{"permissions":{"deny":["Bash(rm *)"]}}"#)?;
    let input = format!(
        r#"{{"source":"local_settings","file_path":{}}}"#,
        serde_json::to_string(&settings)?
    );
    let mut output = Vec::new();

    claude_settings::guard_config_change(input.as_bytes(), &mut output)?;
    fs::remove_dir_all(directory)?;

    assert!(output.is_empty());
    Ok(())
}

#[test]
fn config_change_guard_blocks_oversized_input() -> Result<(), Box<dyn std::error::Error>> {
    let mut output = Vec::new();
    claude_settings::guard_config_change(vec![b' '; 1024 * 1024 + 1].as_slice(), &mut output)?;

    let decision: Value = serde_json::from_slice(&output)?;
    assert_eq!(
        decision.get("decision").and_then(Value::as_str),
        Some("block")
    );
    assert!(
        decision
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.contains("exceeds 1048576 bytes"))
    );
    Ok(())
}

#[test]
fn routing_settings_encode_dynamic_routes_and_minimum_context()
-> Result<(), Box<dyn std::error::Error>> {
    let models = vec![
        ModelDefinition::new(
            "gpt-large",
            "Large",
            "Large model",
            ContextTokens::new(1_000_000)?,
        )?,
        ModelDefinition::new(
            "gpt-small",
            "Small",
            "Small model",
            ContextTokens::new(250_000)?,
        )?,
    ];
    let routes = vec![
        ModelRoute {
            claude_model: ClaudeModelId::new("anthropic-model-rocket-gpt-large-high"),
            display_name: "Large \"quoted\"".into(),
            description: "Large route with \\ slash".into(),
            codex_model: CodexModelId::new("gpt-large"),
            service_tier: ServiceTier::Standard,
            reasoning_effort: ReasoningEffort::High,
        },
        ModelRoute {
            claude_model: ClaudeModelId::new("anthropic-model-rocket-gpt-small-low"),
            display_name: "Small".into(),
            description: "Small route".into(),
            codex_model: CodexModelId::new("gpt-small"),
            service_tier: ServiceTier::Fast,
            reasoning_effort: ReasoningEffort::Low,
        },
    ];
    let canonical_route = ClaudeModelId::new("anthropic-model-rocket-gpt-large-high");
    let catalogue = ModelCatalogue::new(models, routes, &canonical_route)?;
    let mut encoded = Vec::new();
    claude_settings::write_routing_settings(
        &catalogue,
        "/safe/model-rocket-bridge",
        "http://127.0.0.1:1234/token",
        "localhost,127.0.0.1",
        &mut encoded,
    )?;
    let settings: Value = serde_json::from_slice(&encoded)?;
    let available = settings
        .get("availableModels")
        .and_then(Value::as_array)
        .ok_or("availableModels missing")?;
    assert_eq!(
        available,
        &[
            "fable",
            "opus",
            "sonnet",
            "haiku",
            "anthropic-model-rocket-gpt-large-high",
            "anthropic-model-rocket-gpt-small-low",
        ]
        .map(Value::from)
    );
    assert_eq!(
        settings
            .pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION_NAME")
            .and_then(Value::as_str),
        Some("Large \"quoted\"")
    );
    assert_eq!(
        settings
            .pointer("/env/ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION")
            .and_then(Value::as_str),
        Some("Large route with \\ slash")
    );
    assert_eq!(
        settings
            .pointer("/env/CLAUDE_CODE_MAX_CONTEXT_TOKENS")
            .and_then(Value::as_str),
        Some("250000")
    );
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        assert_eq!(
            settings
                .get("env")
                .and_then(|environment| environment.get(name))
                .and_then(Value::as_str),
            Some(""),
            "routing settings did not neutralize {name}"
        );
    }
    Ok(())
}

fn scratch_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "model-rocket-claude-settings-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7()
    ))
}
