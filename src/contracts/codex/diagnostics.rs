//! Codex adapter diagnostics kept outside the literal-free adapter boundary.

use std::{fmt::Display, path::Path};

use crate::{domain::BridgeError, product::CODEX_CLI_VERSION_OUTPUT};

pub(crate) const CODEX_HOME_ENV: &str = "CODEX_HOME";
pub(crate) const HOME_ENV: &str = "HOME";
pub(crate) const PATH_ENV: &str = "PATH";
pub(crate) const CODEX_HOME_DIRECTORY: &str = ".codex";
pub(crate) const AUTH_FILE: &str = "auth.json";
pub(crate) const CONFIG_FILE: &str = "config.toml";
pub(crate) const MODEL_CATALOG_FILE: &str = "models.json";
pub(crate) const HTTP_SCHEME: &str = "http";
pub(crate) const CODEX_VERSION_ARGUMENT: &str = "--version";
pub(crate) const CODEX_HOME_UNKNOWN: &str = "Codex authentication home is unknown";
pub(crate) const CAPTURE_HOST_INVALID: &str = "capture endpoint must use an IP host";
pub(crate) const CAPTURE_ENDPOINT_INVALID: &str = "capture endpoint must use loopback HTTP";
pub(crate) const TOOL_CONTINUATION_MISSING: &str =
    "tool continuation has no pending App Server request";
pub(crate) const OUTPUT_LIMIT_INVALID: &str = "max_tokens does not fit this platform";
pub(crate) const TOKEN_USAGE_MISSING: &str =
    "text-producing turn completed without final token usage";
pub(crate) const TURN_SUBSCRIPTION_MISSING: &str = "turn has no App Server event subscription";
pub(crate) const TURN_TIMEOUT: &str = "timed out waiting for Codex App Server turn";
pub(crate) const TURN_STREAM_CLOSED: &str = "Codex App Server event stream closed";
pub(crate) const TOKEN_COUNT_OVERFLOW: &str = "streamed output token count overflowed";
pub(crate) const TOKEN_BOUNDARY_INVALID: &str = "GPT output token boundary is invalid";
pub(crate) const TOKEN_COUNT_UNDERFLOW: &str = "GPT output token count underflowed";
pub(crate) const REQUEST_ID_EXHAUSTED: &str = "JSON-RPC request id exhausted";
pub(crate) const CONNECTION_STOPPED: &str = "Codex App Server connection is not running";
pub(crate) const REQUEST_ID_REUSED: &str = "JSON-RPC request id was reused";
pub(crate) const RESPONSE_CHANNEL_CLOSED: &str = "Codex App Server response channel closed";
pub(crate) const THREAD_ID_REUSED: &str = "Codex App Server reused an active thread identifier";
pub(crate) const RESPONSE_ID_MISSING: &str = "JSON-RPC response has no numeric id";
pub(crate) const STDOUT_CLOSED: &str = "Codex App Server closed stdout";
pub(crate) const FRAME_LENGTH_OVERFLOW: &str = "App Server frame length overflowed";
pub(crate) const FRAME_BOUNDARY_INVALID: &str = "App Server frame boundary is invalid";
pub(crate) const CREATE_HOME_CONTEXT: &str = "cannot create isolated Codex home";
pub(crate) const SECURE_HOME_CONTEXT: &str = "cannot secure isolated Codex home";
pub(crate) const EXPOSE_AUTH_CONTEXT: &str =
    "cannot expose managed ChatGPT authentication to isolated Codex";
pub(crate) const WRITE_CONFIG_CONTEXT: &str = "cannot write isolated Codex configuration";
pub(crate) const ENCODE_CATALOG_CONTEXT: &str = "cannot encode restricted model catalog";
pub(crate) const WRITE_CATALOG_CONTEXT: &str = "cannot write restricted model catalog";
pub(crate) const START_CODEX_CONTEXT: &str = "cannot start codex";
pub(crate) const DECODE_TOKEN_BOUNDARY_CONTEXT: &str = "cannot decode GPT output token boundary";
pub(crate) const DECODE_OUTPUT_CONTEXT: &str = "cannot decode GPT output as UTF-8";
pub(crate) const DECODE_STABLE_TOKENS_CONTEXT: &str = "cannot decode stable GPT output tokens";
pub(crate) const WRITE_CODEX_CONTEXT: &str = "cannot write to codex";
pub(crate) const READ_STDOUT_CONTEXT: &str = "cannot read codex stdout";
pub(crate) const FRAME_UTF8_CONTEXT: &str = "App Server frame is not UTF-8";
pub(crate) const TOKENIZER_CONTEXT: &str = "cannot initialize the GPT-5 output tokenizer";
pub(crate) const INSPECT_CODEX_CONTEXT: &str = "cannot inspect codex";
pub(crate) const VERSION_UTF8_CONTEXT: &str = "codex version is not UTF-8";
pub(crate) const CODEX_STDIN_MISSING: &str = "codex stdin was not created";
pub(crate) const CODEX_STDOUT_MISSING: &str = "codex stdout was not created";
pub(crate) const FRAME_DELIMITER: u8 = b'\n';

pub(crate) const PROCESS_ARGUMENTS: [&str; 4] = [
    "app-server",
    "--strict-config",
    "--disable",
    "multi_agent_v2",
];
pub(crate) const PROCESS_ARGUMENTS_AFTER_CATALOGUE: [&str; 6] = [
    "-c",
    "agents.enabled=false",
    "-c",
    "web_search=\"disabled\"",
    "--listen",
    "stdio://",
];
pub(crate) const CONFIG_FLAG: &str = "-c";
pub(crate) const CAPTURE_NAME_CONFIG: &str = "model_providers.capture.name=\"capture\"";
pub(crate) const CAPTURE_WIRE_CONFIG: &str = "model_providers.capture.wire_api=\"responses\"";
pub(crate) const CAPTURE_AUTH_CONFIG: &str = "model_providers.capture.requires_openai_auth=false";
pub(crate) const CAPTURE_WEBSOCKET_CONFIG: &str =
    "model_providers.capture.supports_websockets=false";
pub(crate) const CAPTURE_PROVIDER: &str = "capture";
pub(crate) const ALLOWED_ENVIRONMENT: [&str; 5] = ["USER", "TMPDIR", "LANG", "LC_ALL", "SHELL"];

pub(crate) fn configuration(context: &str, error: impl Display) -> BridgeError {
    BridgeError::configuration(format!("{context}: {error}"))
}

pub(crate) fn unavailable(context: &str, error: impl Display) -> BridgeError {
    BridgeError::unavailable(format!("{context}: {error}"))
}

pub(crate) fn protocol(context: &str, error: impl Display) -> BridgeError {
    BridgeError::protocol(format!("{context}: {error}"))
}

pub(crate) fn missing_auth(path: &Path) -> BridgeError {
    BridgeError::configuration(format!(
        "managed ChatGPT authentication is missing at {}",
        path.display()
    ))
}

pub(crate) fn isolated_home_name(process_id: u32, nonce: impl Display) -> String {
    format!("model-rocket-codex-home-{process_id}-{nonce}")
}

pub(crate) fn model_catalog_override(path: &Path) -> Result<String, BridgeError> {
    let encoded = serde_json::to_string(path)
        .map_err(|error| configuration("cannot encode model catalog path", error))?;
    Ok(format!("model_catalog_json={encoded}"))
}

pub(crate) fn capture_endpoint_override(endpoint: &str) -> Result<String, BridgeError> {
    let encoded = serde_json::to_string(endpoint)
        .map_err(|error| configuration("cannot encode capture endpoint override", error))?;
    Ok(format!("model_providers.capture.base_url={encoded}"))
}

pub(crate) fn wrong_account(account_type: &str) -> BridgeError {
    BridgeError::unavailable(format!(
        "managed ChatGPT authentication is required, found {account_type}"
    ))
}

pub(crate) fn unavailable_model(model: &str) -> BridgeError {
    BridgeError::unavailable(format!(
        "model {model} is not available to the managed ChatGPT account"
    ))
}

pub(crate) fn model_list_page_limit() -> BridgeError {
    BridgeError::protocol("model/list exceeded the page limit")
}

pub(crate) fn repeated_model_list_cursor(cursor: &str) -> BridgeError {
    BridgeError::protocol(format!("model/list repeated cursor {cursor}"))
}

pub(crate) fn missing_event_thread(method: &str) -> BridgeError {
    BridgeError::protocol(format!("{method} has no thread id"))
}

pub(crate) fn turn_status(status: &str) -> BridgeError {
    BridgeError::unavailable(format!("turn completed with status {status}"))
}

pub(crate) fn unsupported_method(method: &str) -> BridgeError {
    BridgeError::unsupported(format!(
        "App Server requested unsupported client method {method}"
    ))
}

pub(crate) fn request_timeout(method: &str) -> BridgeError {
    BridgeError::unavailable(format!(
        "timed out waiting for Codex App Server method {method}"
    ))
}

pub(crate) fn rpc_failure(method: &str, code: i64, message: &str) -> BridgeError {
    BridgeError::protocol(format!("{method} failed with {code}: {message}"))
}

pub(crate) fn missing_result(method: &str) -> BridgeError {
    BridgeError::protocol(format!("{method} returned no result"))
}

pub(crate) fn unexpected_request_id(id: u64) -> BridgeError {
    BridgeError::protocol(format!("received response for unexpected request id {id}"))
}

pub(crate) fn unknown_thread(thread_id: &str) -> BridgeError {
    BridgeError::protocol(format!(
        "received event for unknown active thread {thread_id}"
    ))
}

pub(crate) fn mismatched_event_scope(method: Option<&str>, thread_id: &str) -> BridgeError {
    match method {
        Some(super::messages::EVENT_AGENT_DELTA) => {
            BridgeError::protocol("agent message delta does not match the active turn")
        }
        Some(super::messages::EVENT_TOOL_CALL) => {
            BridgeError::protocol("dynamic tool call does not match the active turn")
        }
        _ => unknown_thread(thread_id),
    }
}

pub(crate) fn closed_thread_receiver(thread_id: &str) -> BridgeError {
    BridgeError::protocol(format!(
        "received event for closed active thread {thread_id}"
    ))
}

pub(crate) fn frame_too_large(maximum: usize) -> BridgeError {
    BridgeError::protocol(format!("App Server frame exceeds {maximum} bytes"))
}

pub(crate) fn wrong_version(actual: &str) -> BridgeError {
    BridgeError::configuration(format!(
        "Codex version must be exactly {CODEX_CLI_VERSION_OUTPUT}, found {actual}"
    ))
}

pub(crate) fn warn_home_cleanup(path: &Path, error: impl Display) {
    tracing::warn!(path = %path.display(), %error, "cannot remove isolated Codex home");
}

pub(crate) fn warn_process_kill(error: impl Display) {
    tracing::warn!(%error, "cannot terminate failed Codex App Server process");
}
