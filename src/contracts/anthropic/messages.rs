use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, value::RawValue};

use crate::{
    contracts::json as json_contract,
    domain::BridgeError,
    domain::{
        ClaudeSessionId, ClaudeToolName, ConversationJson, ExecuteMessage, ModelRoute,
        OutputTokenLimit, PendingToolResult, SystemInstructions, ToolDefinition, ToolDescription,
        ToolResultDisposition, ToolResultText, ToolSet, ToolUseId,
    },
};

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
pub(super) enum ToolResultContent {
    String(String),
    Blocks(Vec<ToolResultTextBlock>),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub(super) enum ToolResultTextBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    /// Converts the validated Anthropic transport contract into an owned application request.
    ///
    /// # Errors
    ///
    /// Returns an explicit request error when provider data cannot satisfy domain invariants.
    pub fn execute_message(
        &self,
        session: ClaudeSessionId,
        route: ModelRoute,
    ) -> Result<ExecuteMessage, BridgeError> {
        self.validate(route.claude_model.as_str())?;
        let conversation =
            json_contract::document(self.messages.get().to_owned()).map_err(|error| {
                BridgeError::invalid_request(format!("messages is not valid JSON: {error}"))
            })?;
        let tools = self
            .tools
            .iter()
            .map(|tool| {
                let input_schema = json_contract::object(&tool.input_schema).map_err(|_| {
                    BridgeError::invalid_request(format!(
                        "tool {} input_schema must be a JSON object",
                        tool.name
                    ))
                })?;
                Ok(ToolDefinition::new(
                    ClaudeToolName::from(tool.name.clone()),
                    ToolDescription::new(tool.description.clone()),
                    input_schema,
                ))
            })
            .collect::<Result<Vec<_>, BridgeError>>()?;
        let output_schema = self
            .output_schema()?
            .cloned()
            .map(|value| json_contract::object(&value))
            .transpose()
            .map_err(|_| {
                BridgeError::invalid_request("output_config.format.schema must be a JSON object")
            })?;
        let pending_tool_result = self.final_tool_result()?.map(|result| {
            PendingToolResult::new(
                ToolUseId::from(result.tool_use_id),
                ToolResultText::new(result.text),
                if result.is_error {
                    ToolResultDisposition::Failure
                } else {
                    ToolResultDisposition::Success
                },
            )
        });
        let output_limit = OutputTokenLimit::new(self.max_tokens)
            .map_err(|_| BridgeError::invalid_request("max_tokens must be greater than zero"))?;
        let system = self
            .system
            .as_ref()
            .map_or_else(String::new, TextContent::text);
        Ok(ExecuteMessage::new(
            route,
            session,
            ConversationJson::new(conversation),
            SystemInstructions::new(system),
            ToolSet::new(tools),
            output_limit,
            output_schema,
            pending_tool_result,
        ))
    }

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
        validate_conversation(&messages)?;
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

    /// Extracts the single tool result used to continue an active App Server turn.
    ///
    /// # Errors
    ///
    /// Returns an error unless a continuation is exactly one tool result in a final user message.
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
        let contains_tool_result = blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"));
        if !contains_tool_result {
            return Ok(None);
        }
        if last.get("role").and_then(Value::as_str) != Some("user") || blocks.len() != 1 {
            return Err(BridgeError::unsupported(
                "a continuation must be a final user message containing exactly one tool_result block",
            ));
        }
        let block = blocks
            .first()
            .ok_or_else(|| BridgeError::protocol("tool continuation block disappeared"))?;
        let parsed = serde_json::from_value::<ToolResultBlock>(block.clone()).map_err(|error| {
            BridgeError::invalid_request(format!("invalid tool_result: {error}"))
        })?;
        Ok(Some(ToolResult {
            tool_use_id: parsed.tool_use_id,
            text: parsed.content.text(),
            is_error: parsed.is_error,
        }))
    }
}

fn validate_conversation(messages: &Value) -> Result<(), BridgeError> {
    let messages = messages
        .as_array()
        .ok_or_else(|| BridgeError::invalid_request("messages must be a JSON array"))?;
    if messages.is_empty() {
        return Err(BridgeError::invalid_request("messages must not be empty"));
    }
    for (message_index, message) in messages.iter().enumerate() {
        let message = message.as_object().ok_or_else(|| {
            BridgeError::invalid_request(format!("message {message_index} must be an object"))
        })?;
        let role = message.get("role").and_then(Value::as_str).ok_or_else(|| {
            BridgeError::invalid_request(format!("message {message_index} has no valid role"))
        })?;
        if !matches!(role, "user" | "assistant" | "system") {
            return Err(BridgeError::unsupported(format!(
                "message {message_index} has unsupported role {role}"
            )));
        }
        let content = message.get("content").ok_or_else(|| {
            BridgeError::invalid_request(format!("message {message_index} has no content"))
        })?;
        match content {
            Value::String(_) => {}
            Value::Array(blocks) => {
                for (block_index, block) in blocks.iter().enumerate() {
                    validate_content_block(block, role, message_index, block_index)?;
                }
            }
            _ => {
                return Err(BridgeError::invalid_request(format!(
                    "message {message_index} content must be a string or array"
                )));
            }
        }
    }
    Ok(())
}

fn validate_content_block(
    block: &Value,
    role: &str,
    message_index: usize,
    block_index: usize,
) -> Result<(), BridgeError> {
    let block = block.as_object().ok_or_else(|| {
        BridgeError::invalid_request(format!(
            "message {message_index} content block {block_index} must be an object"
        ))
    })?;
    let block_type = block.get("type").and_then(Value::as_str).ok_or_else(|| {
        BridgeError::invalid_request(format!(
            "message {message_index} content block {block_index} has no valid type"
        ))
    })?;
    if role == "system" && block_type != "text" {
        return Err(BridgeError::unsupported(format!(
            "message {message_index} content block {block_index} type {block_type} is invalid for role system"
        )));
    }
    match block_type {
        "text" => {
            required_block_string(block, "text", message_index, block_index, block_type)?;
        }
        "tool_use" => {
            require_block_role(role, "assistant", message_index, block_index, block_type)?;
            required_block_string(block, "id", message_index, block_index, block_type)?;
            required_block_string(block, "name", message_index, block_index, block_type)?;
            if block.get("input").is_none_or(|input| !input.is_object()) {
                return Err(invalid_block_field(
                    message_index,
                    block_index,
                    block_type,
                    "input",
                ));
            }
        }
        "tool_result" => {
            if role != "user" {
                return Err(BridgeError::unsupported(
                    "a continuation must be a final user message containing exactly one tool_result block",
                ));
            }
            required_block_string(block, "tool_use_id", message_index, block_index, block_type)?;
            validate_tool_result_content(
                block.get("content"),
                message_index,
                block_index,
                block_type,
            )?;
            if block
                .get("is_error")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(invalid_block_field(
                    message_index,
                    block_index,
                    block_type,
                    "is_error",
                ));
            }
        }
        "image" | "document" => {
            return Err(BridgeError::unsupported(format!(
                "message {message_index} content block {block_index} uses unsupported type {block_type}"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn validate_tool_result_content(
    content: Option<&Value>,
    message_index: usize,
    block_index: usize,
    block_type: &str,
) -> Result<(), BridgeError> {
    match content {
        Some(Value::String(_)) => Ok(()),
        Some(Value::Array(blocks)) => {
            for (content_index, block) in blocks.iter().enumerate() {
                let block = block.as_object().ok_or_else(|| {
                    BridgeError::invalid_request(format!(
                        "message {message_index} content block {block_index} tool_result content block {content_index} must be an object"
                    ))
                })?;
                if block.get("type").and_then(Value::as_str) != Some("text")
                    || block.get("text").is_none_or(|text| !text.is_string())
                {
                    return Err(BridgeError::unsupported(format!(
                        "message {message_index} content block {block_index} tool_result content block {content_index} must be text"
                    )));
                }
            }
            Ok(())
        }
        _ => Err(invalid_block_field(
            message_index,
            block_index,
            block_type,
            "content",
        )),
    }
}

fn required_block_string(
    block: &Map<String, Value>,
    field: &str,
    message_index: usize,
    block_index: usize,
    block_type: &str,
) -> Result<(), BridgeError> {
    if block.get(field).is_none_or(|value| !value.is_string()) {
        return Err(invalid_block_field(
            message_index,
            block_index,
            block_type,
            field,
        ));
    }
    Ok(())
}

fn require_block_role(
    actual: &str,
    expected: &str,
    message_index: usize,
    block_index: usize,
    block_type: &str,
) -> Result<(), BridgeError> {
    if actual != expected {
        return Err(BridgeError::unsupported(format!(
            "message {message_index} content block {block_index} type {block_type} requires role {expected}"
        )));
    }
    Ok(())
}

fn invalid_block_field(
    message_index: usize,
    block_index: usize,
    block_type: &str,
    field: &str,
) -> BridgeError {
    BridgeError::invalid_request(format!(
        "message {message_index} content block {block_index} type {block_type} has no valid {field}"
    ))
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

#[cfg(test)]
mod tests {
    use super::MessagesRequest;

    #[test]
    fn tool_definition_rejects_unsupported_extra_fields() {
        let request = serde_json::from_str::<MessagesRequest>(
            r#"{
                "model":"anthropic-model-rocket-gpt-5.6-sol-normal-high",
                "max_tokens":100,
                "messages":[],
                "tools":[{
                    "name":"weather",
                    "description":"Get weather",
                    "input_schema":{"type":"object"},
                    "cache_control":{"type":"ephemeral"}
                }]
            }"#,
        );
        assert!(request.is_err());
    }
}
