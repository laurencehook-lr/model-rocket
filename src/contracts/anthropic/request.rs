use axum::http::{HeaderMap, Uri};
use serde::Deserialize;
use serde_json::json;

use crate::{
    contracts::json as json_contract,
    domain::BridgeError,
    domain::{
        AnthropicOperation, AnthropicRequest, AnthropicRequestBody, ClaudeSessionId,
        ModelCatalogue, ModelRequest, PresentedCredential, RequestedModelId,
    },
    policies::{http_limits::MAX_NON_STREAM_RESPONSE_BYTES, model_routing::MODEL_CREATED_AT},
};

use super::{
    MessagesRequest,
    transport::{CLAUDE_SESSION_HEADER, query, request_headers},
};

const GPT_MODEL_PREFIX: &str = "gpt-";
const CLAUDE_MODEL_PREFIX: &str = "claude-";

#[derive(Deserialize)]
struct ModelEnvelope {
    model: String,
}

pub enum ModelDispatch {
    Local(Vec<u8>),
    Anthropic(AnthropicRequest),
}

#[must_use]
pub fn presented_credential(value: String) -> PresentedCredential {
    PresentedCredential::new(value)
}

/// Decodes and validates one Anthropic-compatible messages request.
///
/// # Errors
///
/// Returns an error when the body, headers, model route, or request invariants are invalid.
pub fn decode_message(
    body: &[u8],
    headers: &HeaderMap,
    uri: &Uri,
    catalogue: &ModelCatalogue,
) -> Result<ModelRequest, BridgeError> {
    let envelope: ModelEnvelope = serde_json::from_slice(body)
        .map_err(|error| BridgeError::invalid_request(format!("invalid JSON body: {error}")))?;
    if let Some(route) = catalogue.route_for(&envelope.model) {
        let request: MessagesRequest = serde_json::from_slice(body).map_err(|error| {
            BridgeError::invalid_request(format!("invalid request body: {error}"))
        })?;
        if !request.stream && u64::from(request.max_tokens) > MAX_NON_STREAM_RESPONSE_BYTES as u64 {
            return Err(BridgeError::invalid_request(format!(
                "non-stream max_tokens exceeds {MAX_NON_STREAM_RESPONSE_BYTES}-byte response limit"
            )));
        }
        let session = headers
            .get(CLAUDE_SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BridgeError::invalid_request("Claude Code session id is required"))?;
        let execute = request.execute_message(ClaudeSessionId::from(session), route.clone())?;
        return Ok(ModelRequest::Assistant {
            model: RequestedModelId::from(envelope.model),
            streaming: request.stream,
            execution: execute,
        });
    }
    if envelope.model.starts_with(GPT_MODEL_PREFIX) {
        return Err(BridgeError::unsupported(format!(
            "GPT model {} is not configured",
            envelope.model
        )));
    }
    if envelope.model.starts_with(CLAUDE_MODEL_PREFIX) {
        let document = std::str::from_utf8(body).map_err(|error| {
            BridgeError::invalid_request(format!("request body is not UTF-8: {error}"))
        })?;
        let document = json_contract::document(document.to_owned()).map_err(|error| {
            BridgeError::invalid_request(format!("request body is not valid JSON: {error}"))
        })?;
        return AnthropicRequest::new(
            AnthropicOperation::Messages,
            query(uri),
            request_headers(headers)?,
            AnthropicRequestBody::Json(document),
        )
        .map(ModelRequest::Anthropic)
        .map_err(|error| BridgeError::invalid_request(error.to_string()));
    }
    Err(BridgeError::unsupported(format!(
        "unsupported model {}",
        envelope.model
    )))
}

/// Decodes and validates one Anthropic-compatible model-list request.
///
/// # Errors
///
/// Returns an error when authentication or transport headers are invalid.
pub fn decode_models(headers: &HeaderMap, uri: &Uri) -> Result<AnthropicRequest, BridgeError> {
    AnthropicRequest::new(
        AnthropicOperation::Models,
        query(uri),
        request_headers(headers)?,
        AnthropicRequestBody::Empty,
    )
    .map_err(|error| BridgeError::invalid_request(error.to_string()))
}

/// Decodes and validates one Anthropic-compatible model-detail request.
///
/// # Errors
///
/// Returns an error when the model is unsupported or request headers are invalid.
pub fn decode_model(
    model: &str,
    headers: &HeaderMap,
    uri: &Uri,
    catalogue: &ModelCatalogue,
) -> Result<ModelDispatch, BridgeError> {
    if let Some(route) = catalogue.route_for(model) {
        return serde_json::to_vec(&json!({
            "id": route.claude_model.as_str(),
            "type": "model",
            "display_name": route.display_name.as_ref(),
            "created_at": MODEL_CREATED_AT
        }))
        .map(ModelDispatch::Local)
        .map_err(|error| {
            BridgeError::anthropic_unavailable(format!(
                "cannot encode local model response: {error}"
            ))
        });
    }
    if !model.starts_with(CLAUDE_MODEL_PREFIX) {
        return Err(BridgeError::unsupported(format!(
            "unsupported model {model}"
        )));
    }
    AnthropicRequest::new(
        AnthropicOperation::Model(RequestedModelId::from(model)),
        query(uri),
        request_headers(headers)?,
        AnthropicRequestBody::Empty,
    )
    .map(ModelDispatch::Anthropic)
    .map_err(|error| BridgeError::invalid_request(error.to_string()))
}
