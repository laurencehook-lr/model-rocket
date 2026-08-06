//! Typed Codex App Server protocol contract.

use crate::domain::{ReasoningEffort, ServiceTier};

pub(crate) mod diagnostics;
pub(crate) mod executable;
pub(crate) mod messages;
pub(crate) mod tool_names;

pub const ISOLATED_CONFIG: &str = r#"check_for_update_on_startup = false
web_search = "disabled"

[orchestrator.skills]
enabled = false

[orchestrator.mcp]
enabled = false

[agents]
enabled = false

[analytics]
enabled = false

[tools.update_plan]
enabled = false

[tools.experimental_request_user_input]
enabled = false

[features]
apps = false
browser_use = false
code_mode = false
code_mode_only = false
computer_use = false
current_time_reminder = false
default_mode_request_user_input = false
deferred_executor = false
hooks = false
image_generation = false
multi_agent = false
multi_agent_v2 = false
plugins = false
skill_search = false
standalone_web_search = false
token_budget = false
tool_suggest = false
unified_exec = false
"#;

#[must_use]
pub const fn reasoning_effort(value: ReasoningEffort) -> &'static str {
    match value {
        ReasoningEffort::Low => "low",
        ReasoningEffort::High => "high",
    }
}

#[must_use]
pub const fn service_tier(value: ServiceTier) -> Option<&'static str> {
    match value {
        ServiceTier::Standard => None,
        ServiceTier::Fast => Some("priority"),
    }
}

/// Builds the restricted model catalogue supplied to Codex App Server.
///
/// # Errors
///
/// Returns an error when the embedded catalogue contract is invalid.
pub fn restricted_model_catalog(
    catalogue: &crate::domain::ModelCatalogue,
) -> Result<serde_json::Value, crate::domain::BridgeError> {
    messages::restricted_model_catalog(catalogue)
}
