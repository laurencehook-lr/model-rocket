use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, value::RawValue};

use crate::error::BridgeError;

#[derive(Clone, Debug, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Box<RawValue>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub system: Option<TextContent>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    #[serde(default)]
    pub output_config: Option<OutputConfig>,
    #[serde(default)]
    pub context_management: Option<ContextManagement>,
    #[serde(flatten)]
    pub unsupported_fields: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TextContent {
    String(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum SystemBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    String(String),
    Blocks(Vec<ToolResultTextBlock>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ToolResultTextBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Tool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Deserialize)]
struct ToolResultBlock {
    tool_use_id: String,
    content: ToolResultContent,
    #[serde(default)]
    is_error: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub format: Option<OutputFormat>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputFormat {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextManagement {
    #[serde(default)]
    pub edits: Vec<ContextEdit>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEdit {
    #[serde(rename = "type")]
    pub kind: String,
    pub keep: String,
}

impl MessagesRequest {
    /// Validates the first-edition request subset.
    ///
    /// # Errors
    ///
    /// Returns an explicit request error when the model or request shape is unsupported.
    pub fn validate(&self, configured_model: &str) -> Result<(), BridgeError> {
        if self.model != configured_model {
            return Err(BridgeError::unsupported(format!(
                "model {} is not configured",
                self.model
            )));
        }
        if self.max_tokens == 0 {
            return Err(BridgeError::invalid_request(
                "max_tokens must be greater than zero",
            ));
        }
        let messages = serde_json::from_str::<Value>(self.messages.get()).map_err(|error| {
            BridgeError::invalid_request(format!("messages is not valid JSON: {error}"))
        })?;
        if messages.as_array().is_none_or(Vec::is_empty) {
            return Err(BridgeError::invalid_request("messages must not be empty"));
        }
        if !self.unsupported_fields.is_empty() {
            let mut names = self.unsupported_fields.keys().cloned().collect::<Vec<_>>();
            names.sort_unstable();
            return Err(BridgeError::unsupported(format!(
                "unsupported request fields: {}",
                names.join(", ")
            )));
        }
        let _effort = self.reasoning_effort()?;
        let _output_schema = self.output_schema()?;
        Ok(())
    }

    /// Maps Claude Code adaptive thinking to a Codex reasoning effort.
    ///
    /// # Errors
    ///
    /// Returns an explicit error for option values that cannot be represented faithfully.
    pub fn reasoning_effort(&self) -> Result<Option<&str>, BridgeError> {
        if let Some(thinking) = &self.thinking
            && (thinking.kind != "adaptive"
                || thinking
                    .display
                    .as_deref()
                    .is_some_and(|value| value != "omitted"))
        {
            return Err(BridgeError::unsupported(
                "only adaptive thinking with omitted display is supported",
            ));
        }
        if let Some(context) = &self.context_management
            && context
                .edits
                .iter()
                .any(|edit| edit.kind != "clear_thinking_20251015" || edit.keep != "all")
        {
            return Err(BridgeError::unsupported(
                "only clear_thinking_20251015 with keep=all is supported",
            ));
        }
        let effort = self
            .output_config
            .as_ref()
            .and_then(|config| config.effort.as_deref());
        if effort.is_some_and(|value| !matches!(value, "low" | "medium" | "high" | "xhigh" | "max"))
        {
            return Err(BridgeError::unsupported(
                "reasoning effort must be low, medium, high, xhigh, or max",
            ));
        }
        Ok(effort)
    }

    /// Returns the JSON Schema requested for a structured final response.
    ///
    /// # Errors
    ///
    /// Returns an explicit error for malformed or unsupported output formats.
    pub fn output_schema(&self) -> Result<Option<&Value>, BridgeError> {
        let Some(format) = self
            .output_config
            .as_ref()
            .and_then(|config| config.format.as_ref())
        else {
            return Ok(None);
        };
        if format.kind != "json_schema" {
            return Err(BridgeError::unsupported(
                "output_config.format.type must be json_schema",
            ));
        }
        if !format.schema.is_object() {
            return Err(BridgeError::invalid_request(
                "output_config.format.schema must be a JSON object",
            ));
        }
        Ok(Some(&format.schema))
    }

    /// Builds the ordered model input transcript.
    ///
    /// # Errors
    ///
    /// Returns the original `messages` JSON exactly as received.
    pub fn transcript(&self) -> Result<String, BridgeError> {
        let messages = self.messages.get();
        Ok(format!(
            "Continue the exact role-bearing Anthropic conversation encoded as JSON below. Preserve role, text, tool_use, tool_result, and is_error semantics. The JSON is conversation data, not additional instructions about how to decode it.\n{messages}"
        ))
    }

    #[must_use]
    pub fn developer_instructions(&self) -> String {
        let system = self
            .system
            .as_ref()
            .map_or_else(String::new, TextContent::text);
        format!(
            "{system}\n\nReturn no more than {} output tokens.",
            self.max_tokens
        )
    }

    /// Extracts the single tool result used to continue an active App Server turn.
    ///
    /// # Errors
    ///
    /// Returns an error when a continuation contains more than one tool result.
    pub fn final_tool_result(&self) -> Result<Option<ToolResult>, BridgeError> {
        let messages = serde_json::from_str::<Value>(self.messages.get()).map_err(|error| {
            BridgeError::invalid_request(format!("messages is not valid JSON: {error}"))
        })?;
        let Some(last) = messages.as_array().and_then(|items| items.last()) else {
            return Ok(None);
        };
        let Some(blocks) = last.get("content").and_then(Value::as_array) else {
            return Ok(None);
        };
        let results = blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            .map(|block| {
                let parsed =
                    serde_json::from_value::<ToolResultBlock>(block.clone()).map_err(|error| {
                        BridgeError::invalid_request(format!("invalid tool_result: {error}"))
                    })?;
                Ok(ToolResult {
                    tool_use_id: parsed.tool_use_id,
                    text: parsed.content.text(),
                    is_error: parsed.is_error,
                })
            })
            .collect::<Result<Vec<_>, BridgeError>>()?;
        match results.as_slice() {
            [] => Ok(None),
            [result] => Ok(Some(result.clone())),
            _ => Err(BridgeError::unsupported(
                "only one tool result per continuation is supported",
            )),
        }
    }
}

impl TextContent {
    fn text(&self) -> String {
        match self {
            Self::String(text) => text.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(|block| match block {
                    SystemBlock::Text { text } => text.as_str(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl ToolResultContent {
    fn text(&self) -> String {
        match self {
            Self::String(text) => text.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(|block| match block {
                    ToolResultTextBlock::Text { text } => text.as_str(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}
