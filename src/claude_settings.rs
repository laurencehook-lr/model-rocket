use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    domain::{BridgeError, ModelCatalogue},
    product::DEFAULT_AVAILABLE_CLAUDE_MODELS,
};

#[derive(Deserialize)]
struct ClaudeSettings {
    #[serde(default, rename = "modelOverrides")]
    model_overrides: BTreeMap<String, serde_json::Value>,
    #[serde(default, rename = "disableAllHooks")]
    disable_all_hooks: bool,
}

#[derive(Deserialize)]
struct ConfigChangeInput {
    source: String,
    file_path: Option<PathBuf>,
}

#[derive(Serialize)]
struct BlockDecision {
    decision: &'static str,
    reason: String,
}

/// Rejects lower-precedence Claude settings that can change the model selected by the picker.
///
/// Missing files are ignored because Claude Code treats those settings scopes as absent.
///
/// # Errors
///
/// Returns an error when an existing settings file is unreadable, malformed, or contains a
/// non-empty `modelOverrides` map.
pub fn validate(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Result<(), BridgeError> {
    for path in paths {
        validate_file(path.as_ref())?;
    }
    Ok(())
}

/// Blocks a live settings change when its file cannot be proved safe for Model Rocket routing.
///
/// # Errors
///
/// Returns an error only when the blocking decision cannot be written to the hook output.
pub fn guard_config_change(
    mut input: impl Read,
    mut output: impl Write,
) -> Result<(), BridgeError> {
    let mut encoded = Vec::new();
    let decision = match input.read_to_end(&mut encoded) {
        Ok(_) => evaluate_config_change(&encoded),
        Err(error) => Some(format!("cannot read ConfigChange input: {error}")),
    };
    let Some(reason) = decision else {
        return Ok(());
    };
    serde_json::to_writer(
        &mut output,
        &BlockDecision {
            decision: "block",
            reason,
        },
    )
    .map_err(|error| {
        BridgeError::configuration(format!("cannot encode ConfigChange decision: {error}"))
    })?;
    writeln!(output).map_err(|error| {
        BridgeError::configuration(format!("cannot write ConfigChange decision: {error}"))
    })
}

fn evaluate_config_change(encoded: &[u8]) -> Option<String> {
    let input: ConfigChangeInput = match serde_json::from_slice(encoded) {
        Ok(input) => input,
        Err(error) => return Some(format!("invalid ConfigChange input: {error}")),
    };
    if !matches!(
        input.source.as_str(),
        "user_settings" | "project_settings" | "local_settings"
    ) {
        return Some(format!("unexpected ConfigChange source: {}", input.source));
    }
    let path = input
        .file_path
        .ok_or_else(|| "ConfigChange input has no file_path".to_owned());
    path.and_then(|path| validate_file(&path).map_err(|error| error.to_string()))
        .err()
}

fn validate_file(path: &Path) -> Result<(), BridgeError> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BridgeError::configuration(format!(
                "cannot read Claude settings {}: {error}",
                path.display()
            )));
        }
    };
    let settings: ClaudeSettings = serde_json::from_slice(&contents).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot parse Claude settings {}: {error}",
            path.display()
        ))
    })?;
    if settings.model_overrides.is_empty() {
        if settings.disable_all_hooks {
            return Err(BridgeError::configuration(format!(
                "Claude disableAllHooks=true is incompatible with Model Rocket: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    Err(BridgeError::configuration(format!(
        "Claude modelOverrides is incompatible with Model Rocket: {}",
        path.display()
    )))
}

/// Writes the complete launch-scoped Claude settings document with dynamic routes safely encoded.
///
/// # Errors
///
/// Returns an error when the settings document cannot be encoded or written.
pub fn write_routing_settings(
    catalogue: &ModelCatalogue,
    bridge_bin: &str,
    base_url: &str,
    proxy_bypass: &str,
    mut output: impl Write,
) -> Result<(), BridgeError> {
    let canonical = catalogue.canonical_route();
    let mut available_models = DEFAULT_AVAILABLE_CLAUDE_MODELS
        .iter()
        .map(|model| Value::String((*model).to_owned()))
        .collect::<Vec<_>>();
    available_models.extend(
        catalogue
            .routes()
            .iter()
            .map(|route| Value::String(route.claude_model.as_str().to_owned())),
    );
    let mut environment = Map::new();
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
        "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
        "CLAUDE_CODE_USE_GATEWAY",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CLAUDE_CODE_USE_MANTLE",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
        "CLAUDE_CODE_OAUTH_SCOPES",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        environment.insert(name.to_owned(), Value::String(String::new()));
    }
    for (name, value) in [
        ("ANTHROPIC_BASE_URL", base_url.to_owned()),
        (
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            canonical.claude_model.as_str().to_owned(),
        ),
        (
            "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
            canonical.display_name.to_string(),
        ),
        (
            "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
            canonical.description.to_string(),
        ),
        (
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            catalogue.minimum_context_tokens().get().to_string(),
        ),
        ("NO_PROXY", proxy_bypass.to_owned()),
        ("no_proxy", proxy_bypass.to_owned()),
    ] {
        environment.insert(name.to_owned(), Value::String(value));
    }
    let settings = json!({
        "apiKeyHelper": "",
        "fallbackModel": [],
        "hooks": {
            "ConfigChange": [{
                "matcher": "user_settings|project_settings|local_settings",
                "hooks": [{
                    "type": "command",
                    "command": format!("{bridge_bin} guard-settings-change"),
                    "timeout": 5
                }]
            }]
        },
        "availableModels": available_models,
        "env": environment
    });
    serde_json::to_writer_pretty(&mut output, &settings).map_err(|error| {
        BridgeError::configuration(format!("cannot encode routing settings: {error}"))
    })?;
    writeln!(output).map_err(|error| {
        BridgeError::configuration(format!("cannot write routing settings: {error}"))
    })
}
