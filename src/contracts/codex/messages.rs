//! Codex App Server JSON-RPC messages and validation.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    contracts::{
        codex::{self, tool_names::DynamicToolNames},
        json as json_contract,
    },
    domain::BridgeError,
    domain::{
        ClaudeToolName, CodexModelId, ContinueModelTurn, ModelCatalogue, StartModelTurn,
        TokenCount, TokenUsage, ToolCall, ToolResultDisposition, ToolUseId,
    },
    product::{CODEX_BASE_INSTRUCTIONS, CODEX_DEVELOPER_GUARD},
};

pub(crate) const ACCOUNT_READ_METHOD: &str = "account/read";
pub(crate) const MODEL_LIST_METHOD: &str = "model/list";
pub(crate) const THREAD_START_METHOD: &str = "thread/start";
pub(crate) const TURN_START_METHOD: &str = "turn/start";
pub(crate) const TURN_INTERRUPT_METHOD: &str = "turn/interrupt";
pub(crate) const INITIALIZE_METHOD: &str = "initialize";
pub(crate) const EVENT_AGENT_DELTA: &str = "item/agentMessage/delta";
pub(crate) const EVENT_TOKEN_USAGE: &str = "thread/tokenUsage/updated";
pub(crate) const EVENT_TOOL_CALL: &str = "item/tool/call";
pub(crate) const EVENT_ITEM_STARTED: &str = "item/started";
pub(crate) const EVENT_ITEM_COMPLETED: &str = "item/completed";
pub(crate) const EVENT_ERROR: &str = "error";
pub(crate) const EVENT_TURN_COMPLETED: &str = "turn/completed";
pub(crate) const EVENT_ACCOUNT_RATE_LIMITS_UPDATED: &str = "account/rateLimits/updated";
pub(crate) const EVENT_THREAD_STARTED: &str = "thread/started";
pub(crate) const EVENT_TURN_STARTED: &str = "turn/started";
pub(crate) const EVENT_REMOTE_CONTROL_STATUS_CHANGED: &str = "remoteControl/status/changed";
pub(crate) const EVENT_THREAD_SETTINGS_UPDATED: &str = "thread/settings/updated";
pub(crate) const EVENT_THREAD_STATUS_CHANGED: &str = "thread/status/changed";
pub(crate) const EVENT_CONFIG_WARNING: &str = "configWarning";
pub(crate) const STARTED_TIMESTAMP_FIELD: &str = "startedAtMs";
pub(crate) const COMPLETED_TIMESTAMP_FIELD: &str = "completedAtMs";

pub(crate) fn account_read_params() -> Value {
    serde_json::json!({"refreshToken": false})
}

pub(crate) fn account_type(response: &Value) -> Result<&str, BridgeError> {
    response
        .get("account")
        .and_then(Value::as_object)
        .and_then(|account| account.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("account/read returned no account type"))
}

pub(crate) fn is_managed_chatgpt(account_type: &str) -> bool {
    account_type == "chatgpt"
}

pub(crate) fn model_list_params(cursor: Option<&str>, limit: u64) -> Value {
    let mut params = Map::new();
    params.insert("includeHidden".to_owned(), Value::Bool(true));
    params.insert("limit".to_owned(), Value::from(limit));
    if let Some(cursor) = cursor {
        params.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
    }
    Value::Object(params)
}

pub(crate) fn model_page(
    response: &Value,
) -> Result<(Vec<CodexModelId>, Option<String>), BridgeError> {
    let models = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError::protocol("model/list returned no data array"))?;
    let models = models
        .iter()
        .map(|entry| {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| BridgeError::protocol("model/list entry has no id"))?;
            let model = entry
                .get("model")
                .and_then(Value::as_str)
                .ok_or_else(|| BridgeError::protocol("model/list entry has no model"))?;
            if id != model {
                return Err(BridgeError::protocol(
                    "model/list entry id does not match model",
                ));
            }
            Ok(CodexModelId::new(model))
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    let cursor = match response.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) => Some(cursor.clone()),
        Some(_) => {
            return Err(BridgeError::protocol(
                "model/list nextCursor is not a string or null",
            ));
        }
    };
    Ok((models, cursor))
}

pub(crate) fn is_turn_scoped_event(method: &str) -> bool {
    matches!(
        method,
        EVENT_AGENT_DELTA
            | EVENT_TOKEN_USAGE
            | EVENT_TOOL_CALL
            | EVENT_ITEM_STARTED
            | EVENT_ITEM_COMPLETED
            | EVENT_ERROR
            | EVENT_TURN_COMPLETED
    )
}

pub(crate) fn validate_ignored_notification(message: &RpcMessage) -> Result<bool, BridgeError> {
    if message.method.as_deref() != Some(EVENT_CONFIG_WARNING) {
        return Ok(false);
    }
    if message.id.is_some() || message.result.is_some() || message.error.is_some() {
        return Err(BridgeError::protocol(
            "configWarning is not a JSON-RPC notification",
        ));
    }
    let params = message
        .params
        .clone()
        .ok_or_else(|| BridgeError::protocol("configWarning has no params"))?;
    let warning = serde_json::from_value::<ConfigWarningParams>(params)
        .map_err(|error| BridgeError::protocol(format!("invalid configWarning params: {error}")))?;
    warning.validate()?;
    Ok(true)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigWarningParams {
    summary: String,
    #[serde(default, rename = "details")]
    _details: Option<String>,
    #[serde(default, rename = "path")]
    _path: Option<String>,
    #[serde(default)]
    range: Option<TextRange>,
}

impl ConfigWarningParams {
    fn validate(&self) -> Result<(), BridgeError> {
        if self.summary.is_empty() {
            return Err(BridgeError::protocol(
                "configWarning summary must not be empty",
            ));
        }
        if let Some(range) = &self.range {
            range.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextRange {
    start: TextPosition,
    end: TextPosition,
}

impl TextRange {
    fn validate(&self) -> Result<(), BridgeError> {
        self.start.validate()?;
        self.end.validate()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPosition {
    line: usize,
    column: usize,
}

impl TextPosition {
    fn validate(&self) -> Result<(), BridgeError> {
        if self.line == 0 || self.column == 0 {
            return Err(BridgeError::protocol(
                "configWarning range positions must be one-based",
            ));
        }
        Ok(())
    }
}

pub(crate) fn thread_start_params(
    request: &StartModelTurn,
    cwd: &str,
    tools: &DynamicToolNames,
    provider: Option<&str>,
) -> Value {
    let dynamic_tools = tools
        .tools()
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    let mut params = Map::new();
    params.insert(
        "model".to_owned(),
        Value::String(request.model().as_str().to_owned()),
    );
    params.insert("cwd".to_owned(), Value::String(cwd.to_owned()));
    params.insert("ephemeral".to_owned(), Value::Bool(true));
    params.insert(
        "approvalPolicy".to_owned(),
        Value::String("never".to_owned()),
    );
    params.insert("sandbox".to_owned(), Value::String("read-only".to_owned()));
    params.insert("environments".to_owned(), Value::Array(Vec::new()));
    params.insert("runtimeWorkspaceRoots".to_owned(), Value::Array(Vec::new()));
    params.insert("dynamicTools".to_owned(), Value::Array(dynamic_tools));
    params.insert(
        "baseInstructions".to_owned(),
        Value::String(CODEX_DEVELOPER_GUARD.to_owned()),
    );
    params.insert(
        "developerInstructions".to_owned(),
        Value::String(request.developer_instructions().as_str().to_owned()),
    );
    if let Some(provider) = provider {
        params.insert(
            "modelProvider".to_owned(),
            Value::String(provider.to_owned()),
        );
    }
    if let Some(tier) = codex::service_tier(request.delivery()) {
        params.insert("serviceTier".to_owned(), Value::String(tier.to_owned()));
    }
    Value::Object(params)
}

pub(crate) fn turn_start_params(
    request: &StartModelTurn,
    thread_id: &str,
) -> Result<Value, BridgeError> {
    let mut params = Map::new();
    params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    params.insert(
        "model".to_owned(),
        Value::String(request.model().as_str().to_owned()),
    );
    params.insert(
        "input".to_owned(),
        serde_json::json!([{"type": "text", "text": request.prompt().as_str()}]),
    );
    params.insert(
        "effort".to_owned(),
        Value::String(codex::reasoning_effort(request.reasoning()).to_owned()),
    );
    if let Some(tier) = codex::service_tier(request.delivery()) {
        params.insert("serviceTier".to_owned(), Value::String(tier.to_owned()));
    }
    if let Some(schema) = request.output_schema() {
        let schema = json_contract::object_value(schema).map_err(|error| {
            BridgeError::protocol(format!("validated output schema is invalid: {error}"))
        })?;
        params.insert("outputSchema".to_owned(), schema);
    }
    Ok(Value::Object(params))
}

pub(crate) fn started_thread_id(response: &Value) -> Result<String, BridgeError> {
    response
        .get("thread")
        .and_then(|thread| thread.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| BridgeError::protocol("thread/start returned no thread id"))
}

pub(crate) fn started_turn_id(response: &Value) -> Result<String, BridgeError> {
    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| BridgeError::protocol("turn/start returned no turn id"))
}

pub(crate) fn tool_resolution(rpc_id: &JsonRpcRequestId, resolution: &ContinueModelTurn) -> Value {
    serde_json::json!({
        "id": rpc_id,
        "result": {
            "contentItems": [{"type": "inputText", "text": resolution.result().as_str()}],
            "success": resolution.disposition() == ToolResultDisposition::Success,
        }
    })
}

pub(crate) fn unsupported_notification(method: &str) -> BridgeError {
    BridgeError::protocol(format!(
        "Codex App Server sent unsupported notification {method}"
    ))
}

pub(crate) fn missing_message_method() -> BridgeError {
    BridgeError::protocol("Codex App Server event has no method")
}

pub(crate) fn interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    serde_json::json!({"threadId": thread_id, "turnId": turn_id})
}

pub(crate) fn initialize_params() -> Value {
    serde_json::json!({
        "clientInfo": {
            "name": "model_rocket",
            "title": "Model Rocket",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "experimentalApi": true,
            "optOutNotificationMethods": [
                EVENT_ACCOUNT_RATE_LIMITS_UPDATED,
                EVENT_THREAD_STARTED,
                EVENT_TURN_STARTED,
                EVENT_REMOTE_CONTROL_STATUS_CHANGED,
                EVENT_THREAD_SETTINGS_UPDATED,
                EVENT_THREAD_STATUS_CHANGED,
                EVENT_CONFIG_WARNING,
            ],
        },
    })
}

pub(crate) fn initialized_notification() -> Value {
    serde_json::json!({"method": "initialized", "params": {}})
}

pub(crate) fn rpc_request(method: &str, id: u64, params: &Value) -> Value {
    serde_json::json!({"method": method, "id": id, "params": params})
}

pub(crate) fn event_thread_id(message: &RpcMessage) -> Option<String> {
    message
        .params
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|params| params.get("threadId"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(crate) fn encode_message(value: &Value) -> Result<Vec<u8>, BridgeError> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| BridgeError::protocol(format!("cannot encode JSON-RPC: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn decode_message(line: &str) -> Result<RpcMessage, BridgeError> {
    serde_json::from_str(line)
        .map_err(|error| BridgeError::protocol(format!("invalid JSON-RPC from codex: {error}")))
}

pub(crate) fn restricted_model_catalog(catalogue: &ModelCatalogue) -> Result<Value, BridgeError> {
    let mut catalog: Value =
        serde_json::from_slice(include_bytes!("../../../config/codex-model-template.json"))
            .map_err(|error| {
                BridgeError::configuration(format!("restricted model template is invalid: {error}"))
            })?;
    let template = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .and_then(|models| models.first())
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| BridgeError::configuration("restricted model template has no model"))?;
    let models = catalogue
        .models()
        .iter()
        .map(|configured| {
            let mut model = template.clone();
            for (field, value) in [
                ("slug", Value::String(configured.id().as_str().to_owned())),
                (
                    "display_name",
                    Value::String(configured.display_name().to_owned()),
                ),
                (
                    "description",
                    Value::String(configured.description().to_owned()),
                ),
                (
                    "base_instructions",
                    Value::String(CODEX_BASE_INSTRUCTIONS.to_owned()),
                ),
                (
                    "context_window",
                    Value::from(configured.context_tokens().get()),
                ),
                (
                    "max_context_window",
                    Value::from(configured.context_tokens().get()),
                ),
            ] {
                model.insert(field.to_owned(), value);
            }
            Value::Object(model)
        })
        .collect();
    let destination = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            BridgeError::configuration("restricted model template has no models array")
        })?;
    *destination = models;
    Ok(catalog)
}

pub(crate) fn agent_message_delta<'a>(
    message: &'a RpcMessage,
    expected: (&str, &str),
) -> Result<&'a str, BridgeError> {
    let params = turn_scoped_params(message, expected, "agent message delta")?;
    params
        .get("itemId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("agent message delta has no item id"))?;
    params
        .get("delta")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("agent message delta has no text"))
}

pub(crate) fn validate_item_lifecycle(
    message: &RpcMessage,
    expected: (&str, &str),
    timestamp_field: &str,
) -> Result<(), BridgeError> {
    let params = turn_scoped_params(message, expected, "item lifecycle event")?;
    params
        .get(timestamp_field)
        .and_then(Value::as_i64)
        .ok_or_else(|| BridgeError::protocol("item lifecycle event has no valid timestamp"))?;
    let item = params
        .get("item")
        .ok_or_else(|| BridgeError::protocol("item lifecycle event has no item"))?;
    validate_allowed_item(item, "item lifecycle event")
}

fn validate_allowed_item(item: &Value, context: &str) -> Result<(), BridgeError> {
    let item = item
        .as_object()
        .ok_or_else(|| BridgeError::protocol(format!("{context} item is not an object")))?;
    item.get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol(format!("{context} has no item id")))?;
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol(format!("{context} has no item type")))?;
    match item_type {
        "agentMessage" => {
            required_item_string(item, "text", context)?;
        }
        "reasoning" => {
            validate_optional_string_array(item, "content", context)?;
            validate_optional_string_array(item, "summary", context)?;
        }
        "userMessage" => {
            let user_inputs = item
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| BridgeError::protocol(format!("{context} has no valid content")))?;
            validate_user_inputs(user_inputs, context)?;
        }
        "dynamicToolCall" => {
            item.get("arguments").ok_or_else(|| {
                BridgeError::protocol(format!("{context} has no dynamic tool arguments"))
            })?;
            required_item_string(item, "tool", context)?;
            match item.get("status").and_then(Value::as_str) {
                Some("inProgress" | "completed" | "failed") => {}
                _ => {
                    return Err(BridgeError::protocol(format!(
                        "{context} has no valid dynamic tool status"
                    )));
                }
            }
        }
        _ => {
            return Err(BridgeError::protocol(format!(
                "Codex attempted forbidden built-in item type {item_type}"
            )));
        }
    }
    Ok(())
}

fn validate_user_inputs(user_inputs: &[Value], item_context: &str) -> Result<(), BridgeError> {
    for (index, input) in user_inputs.iter().enumerate() {
        let input_context = format!("{item_context} user content {index}");
        let input = input
            .as_object()
            .ok_or_else(|| BridgeError::protocol(format!("{input_context} is not an object")))?;
        let input_type = input
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::protocol(format!("{input_context} has no type")))?;
        match input_type {
            "text" => required_item_string(input, "text", &input_context)?,
            "image" | "audio" => required_item_string(input, "url", &input_context)?,
            "localImage" | "localAudio" => {
                required_item_string(input, "path", &input_context)?;
            }
            "skill" | "mention" => {
                required_item_string(input, "name", &input_context)?;
                required_item_string(input, "path", &input_context)?;
            }
            _ => {
                return Err(BridgeError::protocol(format!(
                    "{input_context} has unknown type {input_type}"
                )));
            }
        }
    }
    Ok(())
}

fn required_item_string(
    item: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<(), BridgeError> {
    item.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol(format!("{context} has no valid {field}")))?;
    Ok(())
}

fn validate_optional_string_array(
    item: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<(), BridgeError> {
    let Some(value) = item.get(field) else {
        return Ok(());
    };
    let values = value
        .as_array()
        .ok_or_else(|| BridgeError::protocol(format!("{context} has invalid {field}")))?;
    if values.iter().any(|value| !value.is_string()) {
        return Err(BridgeError::protocol(format!(
            "{context} has invalid {field}"
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum JsonRpcRequestId {
    Number(serde_json::Number),
    String(String),
}

impl JsonRpcRequestId {
    #[must_use]
    pub(crate) fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => value.as_u64(),
            Self::String(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for JsonRpcRequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::Number(value) if value.is_i64() || value.is_u64() => Ok(Self::Number(value)),
            Value::String(value) => Ok(Self::String(value)),
            _ => Err(serde::de::Error::custom(
                "JSON-RPC request id must be an integer or string",
            )),
        }
    }
}

fn deserialize_optional_request_id<'de, D>(
    deserializer: D,
) -> Result<Option<JsonRpcRequestId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    JsonRpcRequestId::deserialize(deserializer).map(Some)
}

fn deserialize_present_result<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

fn deserialize_present_error<'de, D>(deserializer: D) -> Result<Option<RpcError>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    RpcError::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
pub(crate) struct RpcMessage {
    #[serde(default, deserialize_with = "deserialize_optional_request_id")]
    pub(crate) id: Option<JsonRpcRequestId>,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) params: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_present_result")]
    pub(crate) result: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_present_error")]
    pub(crate) error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicToolCallParams {
    arguments: Value,
    call_id: String,
    thread_id: String,
    tool: String,
    turn_id: String,
}

pub(crate) struct ParsedToolCall {
    pub(crate) rpc_id: JsonRpcRequestId,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) tool_call: ToolCall,
}

pub(crate) fn parse_tool_call(
    message: RpcMessage,
    expected: (&str, &str),
    dynamic_tool_names: &DynamicToolNames,
) -> Result<ParsedToolCall, BridgeError> {
    let rpc_id = message
        .id
        .ok_or_else(|| BridgeError::protocol("dynamic tool call has no request id"))?;
    let params = message
        .params
        .ok_or_else(|| BridgeError::protocol("dynamic tool call has no params"))?;
    let call = serde_json::from_value::<DynamicToolCallParams>(params).map_err(|error| {
        BridgeError::protocol(format!("invalid dynamic tool call params: {error}"))
    })?;
    if call.thread_id != expected.0 || call.turn_id != expected.1 {
        return Err(BridgeError::protocol(
            "dynamic tool call does not match the active turn",
        ));
    }
    let claude_tool_name = dynamic_tool_names.claude_name(&call.tool)?;
    let arguments = json_contract::object(&call.arguments).map_err(|error| {
        BridgeError::protocol(format!("dynamic tool call arguments are invalid: {error}"))
    })?;
    Ok(ParsedToolCall {
        rpc_id,
        thread_id: call.thread_id,
        turn_id: call.turn_id,
        tool_call: ToolCall::new(
            ToolUseId::from(call.call_id),
            ClaudeToolName::from(claude_tool_name),
            arguments,
        ),
    })
}

pub(crate) struct AppServerTurnError {
    pub(crate) message: String,
    pub(crate) will_retry: bool,
}

pub(crate) fn app_server_error(
    message: &RpcMessage,
    expected: (&str, &str),
) -> Result<AppServerTurnError, BridgeError> {
    let params = turn_scoped_params(message, expected, "error notification")?;
    let will_retry = params
        .get("willRetry")
        .and_then(Value::as_bool)
        .ok_or_else(|| BridgeError::protocol("error notification has no retry flag"))?;
    let message = params
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| BridgeError::protocol("error notification has no message"))?;
    Ok(AppServerTurnError {
        message,
        will_retry,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

impl TurnStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::InProgress => "inProgress",
        }
    }
}

pub(crate) fn turn_status(
    message: &RpcMessage,
    expected: (&str, &str),
) -> Result<TurnStatus, BridgeError> {
    let params = message
        .params
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no params"))?;
    let thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no thread id"))?;
    let turn = params
        .get("turn")
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no turn"))?;
    let turn_id = turn
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no turn id"))?;
    if thread_id != expected.0 || turn_id != expected.1 {
        return Err(BridgeError::protocol(
            "turn/completed does not match the active turn",
        ));
    }
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no item list"))?;
    for (index, item) in items.iter().enumerate() {
        validate_allowed_item(item, &format!("turn/completed item {index}"))?;
    }
    match turn.get("status").and_then(Value::as_str) {
        Some("completed") => Ok(TurnStatus::Completed),
        Some("interrupted") => Ok(TurnStatus::Interrupted),
        Some("failed") => Ok(TurnStatus::Failed),
        Some("inProgress") => Ok(TurnStatus::InProgress),
        Some(status) => Err(BridgeError::protocol(format!(
            "turn/completed has unknown status {status}"
        ))),
        None => Err(BridgeError::protocol("turn/completed has no status")),
    }
}

pub(crate) fn token_usage(
    message: &RpcMessage,
    expected: (&str, &str),
) -> Result<TokenUsage, BridgeError> {
    let params = turn_scoped_params(message, expected, "token usage update")?;
    let token_usage = params
        .get("tokenUsage")
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::protocol("token usage update has no usage"))?;
    let last = params
        .get("tokenUsage")
        .and_then(|value| value.get("last"))
        .ok_or_else(|| BridgeError::protocol("token usage update has no last usage"))?;
    let total = token_usage
        .get("total")
        .ok_or_else(|| BridgeError::protocol("token usage update has no total usage"))?;
    let (input_tokens, output_tokens) = token_usage_breakdown(last, "last")?;
    token_usage_breakdown(total, "total")?;
    Ok(TokenUsage::new(
        TokenCount::new(input_tokens),
        TokenCount::new(output_tokens),
    ))
}

fn turn_scoped_params<'a>(
    message: &'a RpcMessage,
    expected: (&str, &str),
    event: &str,
) -> Result<&'a serde_json::Map<String, Value>, BridgeError> {
    let params = message
        .params
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| BridgeError::protocol(format!("{event} has no params")))?;
    let thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol(format!("{event} has no thread id")))?;
    let turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol(format!("{event} has no turn id")))?;
    if thread_id != expected.0 || turn_id != expected.1 {
        return Err(BridgeError::protocol(format!(
            "{event} does not match the active turn"
        )));
    }
    Ok(params)
}

fn token_usage_breakdown(value: &Value, label: &str) -> Result<(u64, u64), BridgeError> {
    let breakdown = value
        .as_object()
        .ok_or_else(|| BridgeError::protocol(format!("{label} usage is not an object")))?;
    let required_count = |field: &str| {
        breakdown
            .get(field)
            .and_then(Value::as_u64)
            .ok_or_else(|| BridgeError::protocol(format!("{label} usage has no {field}")))
    };
    let input_tokens = required_count("inputTokens")?;
    let output_tokens = required_count("outputTokens")?;
    required_count("cachedInputTokens")?;
    required_count("reasoningOutputTokens")?;
    required_count("totalTokens")?;
    if let Some(cache_write_input_tokens) = breakdown.get("cacheWriteInputTokens") {
        cache_write_input_tokens.as_u64().ok_or_else(|| {
            BridgeError::protocol(format!("{label} usage has invalid cacheWriteInputTokens"))
        })?;
    }
    Ok((input_tokens, output_tokens))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::{
        EVENT_ACCOUNT_RATE_LIMITS_UPDATED, EVENT_CONFIG_WARNING,
        EVENT_REMOTE_CONTROL_STATUS_CHANGED, EVENT_THREAD_SETTINGS_UPDATED, EVENT_THREAD_STARTED,
        EVENT_THREAD_STATUS_CHANGED, EVENT_TURN_STARTED, JsonRpcRequestId, RpcMessage,
        agent_message_delta, app_server_error, initialize_params, missing_message_method,
        model_page, parse_tool_call, token_usage, turn_status, unsupported_notification,
        validate_ignored_notification, validate_item_lifecycle,
    };
    use crate::contracts::codex::tool_names::DynamicToolNames;

    const EXPECTED: (&str, &str) = ("thread_expected", "turn_expected");

    fn message(method: &str, params: &Value) -> Result<RpcMessage, serde_json::Error> {
        serde_json::from_value(json!({"method": method, "params": params}))
    }

    #[test]
    fn model_page_rejects_non_string_cursor() -> Result<(), Box<dyn std::error::Error>> {
        let error = model_page(&json!({"data": [], "nextCursor": 7}))
            .err()
            .ok_or("non-string cursor must fail")?;
        assert!(error.to_string().contains("not a string or null"));
        Ok(())
    }

    #[test]
    fn json_rpc_request_id_accepts_only_integer_or_string_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let numeric: RpcMessage = serde_json::from_value(json!({"id": 7, "result": {}}))?;
        assert_eq!(
            numeric.id,
            Some(JsonRpcRequestId::Number(serde_json::Number::from(7)))
        );
        let negative: RpcMessage = serde_json::from_value(json!({"id": -7, "result": {}}))?;
        assert_eq!(
            negative.id,
            Some(JsonRpcRequestId::Number(serde_json::Number::from(-7)))
        );
        let string: RpcMessage =
            serde_json::from_value(json!({"id": "tool-request", "method": "item/tool/call"}))?;
        assert_eq!(
            string.id,
            Some(JsonRpcRequestId::String("tool-request".to_owned()))
        );
        for invalid in [json!(null), json!(true), json!(1.5), json!({}), json!([])] {
            assert!(
                serde_json::from_value::<RpcMessage>(json!({"id": invalid, "result": {}})).is_err(),
                "invalid JSON-RPC request id was accepted"
            );
        }
        Ok(())
    }

    #[test]
    fn unsupported_or_methodless_notifications_have_explicit_protocol_errors() {
        assert!(
            unsupported_notification("thread/unknown")
                .to_string()
                .contains("unsupported notification thread/unknown")
        );
        assert!(
            missing_message_method()
                .to_string()
                .contains("event has no method")
        );
    }

    #[test]
    fn initialization_suppresses_irrelevant_notifications() {
        assert_eq!(
            initialize_params()
                .pointer("/capabilities/optOutNotificationMethods")
                .and_then(Value::as_array),
            Some(&vec![
                Value::String(EVENT_ACCOUNT_RATE_LIMITS_UPDATED.to_owned()),
                Value::String(EVENT_THREAD_STARTED.to_owned()),
                Value::String(EVENT_TURN_STARTED.to_owned()),
                Value::String(EVENT_REMOTE_CONTROL_STATUS_CHANGED.to_owned()),
                Value::String(EVENT_THREAD_SETTINGS_UPDATED.to_owned()),
                Value::String(EVENT_THREAD_STATUS_CHANGED.to_owned()),
                Value::String(EVENT_CONFIG_WARNING.to_owned()),
            ])
        );
    }

    #[test]
    fn config_warning_requires_its_pinned_notification_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let valid: RpcMessage = serde_json::from_value(json!({
            "method": EVENT_CONFIG_WARNING,
            "params": {
                "summary": "config warning",
                "details": "using defaults",
                "path": "/tmp/config.toml",
                "range": {
                    "start": {"line": 1, "column": 2},
                    "end": {"line": 3, "column": 4}
                }
            }
        }))?;
        assert!(validate_ignored_notification(&valid)?);

        let startup: RpcMessage = serde_json::from_value(json!({
            "method": EVENT_CONFIG_WARNING,
            "params": {"summary": "empty auth fixture"}
        }))?;
        assert!(validate_ignored_notification(&startup)?);

        for invalid in [
            json!({"method": EVENT_CONFIG_WARNING, "params": {}}),
            json!({"method": EVENT_CONFIG_WARNING, "params": "invalid"}),
            json!({"id": 7, "method": EVENT_CONFIG_WARNING, "params": {"summary": "warning"}}),
            json!({"method": EVENT_CONFIG_WARNING, "result": {}, "params": {"summary": "warning"}}),
            json!({"method": EVENT_CONFIG_WARNING, "result": null, "params": {"summary": "warning"}}),
            json!({"method": EVENT_CONFIG_WARNING, "error": {"code": 1, "message": "error"}, "params": {"summary": "warning"}}),
            json!({"method": EVENT_CONFIG_WARNING, "params": {"summary": "warning", "unknown": true}}),
            json!({"method": EVENT_CONFIG_WARNING, "params": {
                "summary": "warning",
                "range": {
                    "start": {"line": 0, "column": 1},
                    "end": {"line": 1, "column": 1}
                }
            }}),
        ] {
            let message = serde_json::from_value::<RpcMessage>(invalid)?;
            assert!(validate_ignored_notification(&message).is_err());
        }
        assert!(
            serde_json::from_value::<RpcMessage>(json!({
                "method": EVENT_CONFIG_WARNING,
                "error": null,
                "params": {"summary": "warning"}
            }))
            .is_err()
        );

        let unknown: RpcMessage = serde_json::from_value(json!({
            "method": "thread/unknown",
            "params": {}
        }))?;
        assert!(!validate_ignored_notification(&unknown)?);
        Ok(())
    }

    #[test]
    fn every_consumed_turn_event_rejects_foreign_scope() -> Result<(), Box<dyn std::error::Error>> {
        let delta = message(
            "item/agentMessage/delta",
            &json!({
                "delta": "foreign",
                "itemId": "item_foreign",
                "threadId": "thread_foreign",
                "turnId": "turn_expected"
            }),
        )?;
        assert!(agent_message_delta(&delta, EXPECTED).is_err());

        let usage = message(
            "thread/tokenUsage/updated",
            &json!({
                "threadId": "thread_expected",
                "turnId": "turn_foreign",
                "tokenUsage": {}
            }),
        )?;
        assert!(token_usage(&usage, EXPECTED).is_err());

        for (method, timestamp_field) in [
            ("item/started", "startedAtMs"),
            ("item/completed", "completedAtMs"),
        ] {
            let lifecycle = message(
                method,
                &json!({
                    "threadId": "thread_foreign",
                    "turnId": "turn_expected",
                    (timestamp_field): 1,
                    "item": {"id": "item_foreign", "type": "agentMessage", "text": ""}
                }),
            )?;
            assert!(validate_item_lifecycle(&lifecycle, EXPECTED, timestamp_field).is_err());
        }

        let error = message(
            "error",
            &json!({
                "error": {"message": "foreign"},
                "threadId": "thread_foreign",
                "turnId": "turn_expected",
                "willRetry": false
            }),
        )?;
        assert!(app_server_error(&error, EXPECTED).is_err());

        let completion = message(
            "turn/completed",
            &json!({
                "threadId": "thread_expected",
                "turn": {"id": "turn_foreign", "items": [], "status": "completed"}
            }),
        )?;
        assert!(turn_status(&completion, EXPECTED).is_err());

        let tool_call: RpcMessage = serde_json::from_value(json!({
            "id": 1,
            "method": "item/tool/call",
            "params": {
                "arguments": {},
                "callId": "call_foreign",
                "threadId": "thread_expected",
                "tool": "model_rocket_tool_0",
                "turnId": "turn_foreign"
            }
        }))?;
        assert!(parse_tool_call(tool_call, EXPECTED, &DynamicToolNames::default()).is_err());
        Ok(())
    }

    #[test]
    fn recognized_turn_events_reject_missing_scope_fields() -> Result<(), Box<dyn std::error::Error>>
    {
        let delta = message(
            "item/agentMessage/delta",
            &json!({"delta": "foreign", "itemId": "item_foreign"}),
        )?;
        let error = agent_message_delta(&delta, EXPECTED)
            .err()
            .ok_or_else(|| std::io::Error::other("scope-free delta was accepted"))?;
        assert!(error.to_string().contains("has no thread id"));

        let completion = message(
            "turn/completed",
            &json!({
                "threadId": "thread_expected",
                "turn": {"items": [], "status": "completed"}
            }),
        )?;
        let error = turn_status(&completion, EXPECTED)
            .err()
            .ok_or_else(|| std::io::Error::other("scope-free completion was accepted"))?;
        assert!(error.to_string().contains("has no turn id"));
        Ok(())
    }

    #[test]
    fn lifecycle_and_terminal_items_require_their_type_specific_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        for item in [
            json!({"id": "item_agent", "type": "agentMessage"}),
            json!({"id": "item_reasoning", "type": "reasoning", "summary": "invalid"}),
            json!({"id": "item_user", "type": "userMessage", "content": "invalid"}),
            json!({"id": "item_user", "type": "userMessage", "content": [7]}),
            json!({
                "id": "item_user",
                "type": "userMessage",
                "content": [{"type": "text"}]
            }),
            json!({
                "id": "item_user",
                "type": "userMessage",
                "content": [{"type": "unknown"}]
            }),
            json!({
                "id": "item_tool",
                "type": "dynamicToolCall",
                "arguments": {},
                "tool": "weather"
            }),
        ] {
            let lifecycle = message(
                "item/completed",
                &json!({
                    "threadId": "thread_expected",
                    "turnId": "turn_expected",
                    "completedAtMs": 1,
                    "item": item
                }),
            )?;
            assert!(
                validate_item_lifecycle(&lifecycle, EXPECTED, "completedAtMs").is_err(),
                "malformed lifecycle item was accepted"
            );
        }

        let completion = message(
            "turn/completed",
            &json!({
                "threadId": "thread_expected",
                "turn": {
                    "id": "turn_expected",
                    "items": [{"id": "item_agent", "type": "agentMessage"}],
                    "status": "completed"
                }
            }),
        )?;
        assert!(
            turn_status(&completion, EXPECTED).is_err(),
            "malformed terminal item was accepted"
        );
        Ok(())
    }

    #[test]
    fn user_message_accepts_every_pinned_input_variant() -> Result<(), Box<dyn std::error::Error>> {
        let lifecycle = message(
            "item/completed",
            &json!({
                "threadId": "thread_expected",
                "turnId": "turn_expected",
                "completedAtMs": 1,
                "item": {
                    "id": "item_user",
                    "type": "userMessage",
                    "content": [
                        {"type": "text", "text": "hello"},
                        {"type": "image", "url": "https://example.invalid/image.png"},
                        {"type": "localImage", "path": "/tmp/image.png"},
                        {"type": "audio", "url": "https://example.invalid/audio.mp3"},
                        {"type": "localAudio", "path": "/tmp/audio.mp3"},
                        {"type": "skill", "name": "skill", "path": "/tmp/skill"},
                        {"type": "mention", "name": "file", "path": "/tmp/file"}
                    ]
                }
            }),
        )?;
        validate_item_lifecycle(&lifecycle, EXPECTED, "completedAtMs")?;
        Ok(())
    }
}
