use std::{collections::VecDeque, sync::Arc};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot};

use crate::{
    anthropic::MessagesRequest,
    bridge::{AssistantOutcome, Bridge},
    error::BridgeError,
};

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_NON_STREAM_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const HEALTH_ROUTE: &str = "/healthz";
const MESSAGES_ROUTE: &str = "/{token}/v1/messages";
const MODELS_ROUTE: &str = "/{token}/v1/models";
const MODEL_ROUTE: &str = "/{token}/v1/models/{model}";
const LEGACY_LOCAL_AUTH_HEADER: &str = "x-model-rocket-token";
const CLAUDE_SESSION_HEADER: &str = "x-claude-code-session-id";
const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/";

#[derive(Clone)]
struct HttpState {
    bridge: Bridge,
    anthropic: reqwest::Client,
    #[cfg(test)]
    anthropic_base_url: reqwest::Url,
}

#[derive(Deserialize)]
struct ModelEnvelope {
    model: String,
}

/// Builds the loopback model router.
///
/// # Errors
///
/// Returns an error if the hardened Anthropic HTTP client cannot be built.
pub fn router(bridge: Bridge) -> Result<Router, BridgeError> {
    let anthropic = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            BridgeError::configuration(format!("cannot build Anthropic HTTP client: {error}"))
        })?;
    Ok(routes(HttpState {
        bridge,
        anthropic,
        #[cfg(test)]
        anthropic_base_url: reqwest::Url::parse(ANTHROPIC_BASE_URL).map_err(|error| {
            BridgeError::configuration(format!("invalid Anthropic base URL: {error}"))
        })?,
    }))
}

fn routes(state: HttpState) -> Router {
    Router::new()
        .route(HEALTH_ROUTE, get(health))
        .route(MESSAGES_ROUTE, post(messages))
        .route(MODELS_ROUTE, get(models))
        .route(MODEL_ROUTE, get(model))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(Arc::new(state))
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn messages(
    State(state): State<Arc<HttpState>>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    match messages_inner(&state, &token, request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn messages_inner(
    state: &HttpState,
    token: &str,
    request: Request,
) -> Result<Response, BridgeError> {
    authenticate(token, &state.bridge.config().bearer)?;
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| BridgeError::PayloadTooLarge)?;
    let envelope: ModelEnvelope = serde_json::from_slice(&body)
        .map_err(|error| BridgeError::invalid_request(format!("invalid JSON body: {error}")))?;

    if envelope.model == state.bridge.config().model {
        return gpt_messages(state.bridge.clone(), &parts.headers, &body).await;
    }
    if envelope.model.starts_with("gpt-") {
        return Err(BridgeError::unsupported(format!(
            "GPT model {} is not configured; expected {}",
            envelope.model,
            state.bridge.config().model
        )));
    }
    if envelope.model.starts_with("claude-") {
        require_anthropic_oauth(&parts.headers)?;
        return proxy(
            state,
            Method::POST,
            &parts.uri,
            "v1/messages",
            &parts.headers,
            body,
        )
        .await;
    }
    Err(BridgeError::unsupported(format!(
        "unsupported model {}",
        envelope.model
    )))
}

async fn gpt_messages(
    bridge: Bridge,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, BridgeError> {
    let request: MessagesRequest = serde_json::from_slice(body)
        .map_err(|error| BridgeError::invalid_request(format!("invalid request body: {error}")))?;
    request.validate(&bridge.config().model)?;
    if !request.stream
        && u64::from(request.max_tokens)
            > u64::try_from(MAX_NON_STREAM_RESPONSE_BYTES)
                .map_err(|_| BridgeError::configuration("response limit is not representable"))?
    {
        return Err(BridgeError::invalid_request(format!(
            "non-stream max_tokens exceeds {MAX_NON_STREAM_RESPONSE_BYTES}-byte response limit"
        )));
    }
    let claude_session_id = headers
        .get(CLAUDE_SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BridgeError::invalid_request("Claude Code session id is required"))?
        .to_owned();
    let model = request.model.clone();
    let stream = request.stream;
    let (delta_tx, delta_rx) = mpsc::channel(32);
    let (outcome_tx, outcome_rx) = oneshot::channel();
    tokio::spawn(async move {
        let outcome = bridge.handle(&request, &claude_session_id, &delta_tx).await;
        drop(delta_tx);
        let _result = outcome_tx.send(outcome);
    });
    if stream {
        let body = assistant_stream(&model, delta_rx, outcome_rx)?;
        return Ok(Sse::new(body).into_response());
    }

    assistant_message(&model, delta_rx, outcome_rx).await
}

fn authenticate(candidate: &str, expected: &str) -> Result<(), BridgeError> {
    let expected_digest = Sha256::digest(expected.as_bytes());
    let candidate_digest = Sha256::digest(candidate.as_bytes());
    let matches: bool = expected_digest.ct_eq(&candidate_digest).into();
    if matches {
        Ok(())
    } else {
        Err(BridgeError::Authentication)
    }
}

async fn models(
    State(state): State<Arc<HttpState>>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    match models_inner(&state, &token, request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn models_inner(
    state: &HttpState,
    token: &str,
    request: Request,
) -> Result<Response, BridgeError> {
    authenticate(token, &state.bridge.config().bearer)?;
    require_anthropic_oauth(request.headers())?;
    proxy(
        state,
        Method::GET,
        request.uri(),
        "v1/models",
        request.headers(),
        Bytes::new(),
    )
    .await
}

async fn model(
    State(state): State<Arc<HttpState>>,
    Path((token, model)): Path<(String, String)>,
    request: Request,
) -> Response {
    match model_inner(&state, &token, &model, request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn model_inner(
    state: &HttpState,
    token: &str,
    model: &str,
    request: Request,
) -> Result<Response, BridgeError> {
    authenticate(token, &state.bridge.config().bearer)?;
    if model == state.bridge.config().model {
        return serde_json::to_vec(&local_model(model))
            .map(|body| json_response(StatusCode::OK, body))
            .map_err(|error| {
                BridgeError::anthropic_unavailable(format!(
                    "cannot encode local model response: {error}"
                ))
            });
    }
    if !model.starts_with("claude-") {
        return Err(BridgeError::unsupported(format!(
            "unsupported model {model}"
        )));
    }
    require_anthropic_oauth(request.headers())?;
    proxy(
        state,
        Method::GET,
        request.uri(),
        &format!("v1/models/{model}"),
        request.headers(),
        Bytes::new(),
    )
    .await
}

fn local_model(model: &str) -> Value {
    json!({
        "id": model,
        "type": "model",
        "display_name": "GPT-5.6 Sol through Codex subscription",
        "created_at": "1970-01-01T00:00:00Z"
    })
}

async fn proxy(
    state: &HttpState,
    method: Method,
    uri: &Uri,
    path: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, BridgeError> {
    let upstream = send_upstream(state, method, uri, path, headers, body).await?;
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    copy_response_headers(&headers, response.headers_mut());
    Ok(response)
}

async fn send_upstream(
    state: &HttpState,
    method: Method,
    uri: &Uri,
    path: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<reqwest::Response, BridgeError> {
    #[cfg(not(test))]
    let base = reqwest::Url::parse(ANTHROPIC_BASE_URL);
    #[cfg(test)]
    let base = Ok(state.anthropic_base_url.clone());
    let mut url = base.and_then(|base| base.join(path)).map_err(|error| {
        BridgeError::anthropic_unavailable(format!("cannot construct upstream URL: {error}"))
    })?;
    url.set_query(uri.query());
    let mut request = state.anthropic.request(method, url).body(body);
    for (name, value) in headers {
        if forward_request_header(name) {
            request = request.header(name, value);
        }
    }
    let response = request
        .send()
        .await
        .map_err(|error| BridgeError::anthropic_unavailable(format!("request failed: {error}")))?;
    if response.status().is_redirection() {
        return Err(BridgeError::anthropic_unavailable(
            "upstream redirect was rejected",
        ));
    }
    Ok(response)
}

fn require_anthropic_oauth(headers: &HeaderMap) -> Result<(), BridgeError> {
    let oauth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| !value.is_empty());
    if oauth && !headers.contains_key("x-api-key") {
        Ok(())
    } else {
        Err(BridgeError::Authentication)
    }
}

fn forward_request_header(name: &HeaderName) -> bool {
    name != header::HOST
        && name != header::CONTENT_LENGTH
        && name != header::CONNECTION
        && name != header::TRANSFER_ENCODING
        && name.as_str() != LEGACY_LOCAL_AUTH_HEADER
        && name.as_str() != "x-api-key"
        && name.as_str() != "proxy-authorization"
        && name.as_str() != "proxy-authenticate"
}

fn copy_response_headers(source: &HeaderMap, destination: &mut HeaderMap) {
    for (name, value) in source {
        if name != header::CONTENT_LENGTH
            && name != header::CONNECTION
            && name != header::LOCATION
            && name != header::TRANSFER_ENCODING
            && name.as_str() != "proxy-authorization"
            && name.as_str() != "proxy-authenticate"
        {
            destination.append(name, value.clone());
        }
    }
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn assistant_stream(
    model: &str,
    delta_rx: mpsc::Receiver<String>,
    outcome_rx: oneshot::Receiver<Result<AssistantOutcome, BridgeError>>,
) -> Result<impl futures_util::Stream<Item = Result<Event, BridgeError>> + use<>, BridgeError> {
    let queue = VecDeque::from([sse_event(
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": format!("msg_{}", uuid::Uuid::now_v7().simple()),
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": model,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        }),
    )?]);
    Ok(stream::unfold(
        AssistantStreamState {
            delta_rx,
            outcome_rx: Some(outcome_rx),
            queue,
            text_started: false,
            deltas_done: false,
        },
        |mut state| async move {
            loop {
                if let Some(event) = state.queue.pop_front() {
                    return Some((Ok(event), state));
                }
                if !state.deltas_done {
                    if let Some(delta) = state.delta_rx.recv().await {
                        if !state.text_started {
                            state.queue.push_back(text_block_start(0));
                            state.text_started = true;
                        }
                        state.queue.push_back(text_delta(0, &delta));
                        continue;
                    }
                    state.deltas_done = true;
                }
                let outcome_rx = state.outcome_rx.take()?;
                match outcome_rx.await {
                    Ok(Ok(outcome)) => {
                        state
                            .queue
                            .extend(terminal_events(outcome, state.text_started));
                    }
                    Ok(Err(error)) => state.queue.push_back(error_event(&error)),
                    Err(_) => state.queue.push_back(error_event(&BridgeError::unavailable(
                        "generation task ended without an outcome",
                    ))),
                }
            }
        },
    ))
}

async fn assistant_message(
    model: &str,
    mut delta_rx: mpsc::Receiver<String>,
    outcome_rx: oneshot::Receiver<Result<AssistantOutcome, BridgeError>>,
) -> Result<Response, BridgeError> {
    let mut text = String::new();
    while let Some(delta) = delta_rx.recv().await {
        let next_len = text
            .len()
            .checked_add(delta.len())
            .ok_or_else(|| BridgeError::protocol("non-stream response length overflowed"))?;
        if next_len > MAX_NON_STREAM_RESPONSE_BYTES {
            return Err(BridgeError::protocol(format!(
                "non-stream response exceeds {MAX_NON_STREAM_RESPONSE_BYTES} bytes"
            )));
        }
        text.push_str(&delta);
    }
    let outcome = outcome_rx
        .await
        .map_err(|_| BridgeError::unavailable("generation task ended without an outcome"))??;

    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    let (stop_reason, usage) = match outcome {
        AssistantOutcome::Text {
            usage,
            limit_reached,
        } => (
            if limit_reached {
                "max_tokens"
            } else {
                "end_turn"
            },
            Some(usage),
        ),
        AssistantOutcome::ToolCall {
            id,
            name,
            input,
            usage,
        } => {
            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
            ("tool_use", usage)
        }
    };
    let mut message = json!({
        "id": format!("msg_{}", uuid::Uuid::now_v7().simple()),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": null
    });
    if let Some(usage) = usage
        && let Some(object) = message.as_object_mut()
    {
        object.insert(
            "usage".to_owned(),
            json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens
            }),
        );
    }
    let body = serde_json::to_vec(&message)
        .map_err(|error| BridgeError::protocol(format!("cannot encode JSON response: {error}")))?;
    if body.len() > MAX_NON_STREAM_RESPONSE_BYTES {
        return Err(BridgeError::protocol(format!(
            "non-stream encoded response exceeds {MAX_NON_STREAM_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(json_response(StatusCode::OK, body))
}

struct AssistantStreamState {
    delta_rx: mpsc::Receiver<String>,
    outcome_rx: Option<oneshot::Receiver<Result<AssistantOutcome, BridgeError>>>,
    queue: VecDeque<Event>,
    text_started: bool,
    deltas_done: bool,
}

fn terminal_events(outcome: AssistantOutcome, text_started: bool) -> Vec<Event> {
    let mut events = Vec::new();
    let (stop_reason, usage) = match outcome {
        AssistantOutcome::Text {
            usage,
            limit_reached,
        } => {
            if !text_started {
                events.push(text_block_start(0));
            }
            events.push(content_block_stop(0));
            (
                if limit_reached {
                    "max_tokens"
                } else {
                    "end_turn"
                },
                Some(usage),
            )
        }
        AssistantOutcome::ToolCall {
            id,
            name,
            input,
            usage,
        } => {
            let index = usize::from(text_started);
            if text_started {
                events.push(content_block_stop(0));
            }
            events.push(
                sse_event(
                    "content_block_start",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}
                    }),
                )
                .unwrap_or_else(|error| error_as_sse(&error)),
            );
            let partial_json = match serde_json::to_string(&input) {
                Ok(value) => value,
                Err(error) => {
                    events.push(error_event(&BridgeError::protocol(format!(
                        "cannot encode tool arguments: {error}"
                    ))));
                    return events;
                }
            };
            events.push(
                sse_event(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "input_json_delta", "partial_json": partial_json}
                    }),
                )
                .unwrap_or_else(|error| error_as_sse(&error)),
            );
            events.push(content_block_stop(index));
            ("tool_use", usage)
        }
    };

    let mut terminal = json!({
        "type": "message_delta",
        "delta": {"stop_reason": stop_reason, "stop_sequence": null}
    });
    if let Some(usage) = usage
        && let Some(object) = terminal.as_object_mut()
    {
        object.insert(
            "usage".to_owned(),
            json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens
            }),
        );
    }
    events.push(sse_event("message_delta", terminal).unwrap_or_else(|error| error_as_sse(&error)));
    events.push(
        sse_event("message_stop", json!({"type": "message_stop"}))
            .unwrap_or_else(|error| error_as_sse(&error)),
    );
    events
}

fn text_block_start(index: usize) -> Event {
    sse_event(
        "content_block_start",
        json!({
            "type": "content_block_start",
            "index": index,
            "content_block": {"type": "text", "text": ""}
        }),
    )
    .unwrap_or_else(|error| error_as_sse(&error))
}

fn text_delta(index: usize, text: &str) -> Event {
    sse_event(
        "content_block_delta",
        json!({
            "type": "content_block_delta",
            "index": index,
            "delta": {"type": "text_delta", "text": text}
        }),
    )
    .unwrap_or_else(|error| error_as_sse(&error))
}

fn content_block_stop(index: usize) -> Event {
    sse_event(
        "content_block_stop",
        json!({"type": "content_block_stop", "index": index}),
    )
    .unwrap_or_else(|error| error_as_sse(&error))
}

fn error_event(error: &BridgeError) -> Event {
    sse_event(
        "error",
        json!({
            "type": "error",
            "error": {"type": "api_error", "message": error.to_string()}
        }),
    )
    .unwrap_or_else(|encoding_error| error_as_sse(&encoding_error))
}

fn error_as_sse(error: &BridgeError) -> Event {
    Event::default().event("error").data(error.to_string())
}

fn sse_event(name: &'static str, value: Value) -> Result<Event, BridgeError> {
    Event::default()
        .event(name)
        .json_data(value)
        .map_err(|error| BridgeError::protocol(format!("cannot encode SSE event: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{
        AssistantOutcome, CLAUDE_SESSION_HEADER, HttpState, LEGACY_LOCAL_AUTH_HEADER,
        MAX_NON_STREAM_RESPONSE_BYTES, assistant_message, authenticate, require_anthropic_oauth,
        routes, terminal_events,
    };
    use std::{net::SocketAddr, path::PathBuf};

    use axum::{
        Json, Router,
        body::{Body, Bytes, to_bytes},
        http::{HeaderMap, HeaderValue, Request, StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use tower::ServiceExt;

    use crate::{app_server::TokenUsage, bridge::Bridge, config::Config};

    const TEST_TOKEN: &str = "01234567890123456789012345678901";

    #[test]
    fn local_authentication_accepts_matching_path_token() -> Result<(), Box<dyn std::error::Error>>
    {
        authenticate(
            "01234567890123456789012345678901",
            "01234567890123456789012345678901",
        )?;
        Ok(())
    }

    #[test]
    fn anthropic_oauth_rejects_conflicting_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer 01234567890123456789012345678901"),
        );
        headers.insert(
            "x-api-key",
            HeaderValue::from_static("01234567890123456789012345678901"),
        );
        assert!(require_anthropic_oauth(&headers).is_err());
    }

    #[test]
    fn local_authentication_rejects_wrong_path_token() {
        assert!(authenticate("wrong", "01234567890123456789012345678901").is_err());
    }

    #[test]
    fn text_outcome_has_complete_stream_shape() {
        let events = terminal_events(
            AssistantOutcome::Text {
                usage: TokenUsage {
                    input_tokens: 12,
                    output_tokens: 3,
                },
                limit_reached: false,
            },
            true,
        );
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn non_stream_aggregate_limit_rejects_repeated_small_deltas()
    -> Result<(), Box<dyn std::error::Error>> {
        let (delta_tx, delta_rx) = tokio::sync::mpsc::channel(1);
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            let chunk = "x".repeat(64 * 1024);
            for _ in 0..=(MAX_NON_STREAM_RESPONSE_BYTES / chunk.len()) {
                if delta_tx.send(chunk.clone()).await.is_err() {
                    break;
                }
            }
            drop(delta_tx);
            let _sent = outcome_tx.send(Ok(AssistantOutcome::Text {
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                limit_reached: false,
            }));
        };
        let (result, ()) = tokio::join!(
            assistant_message("gpt-5.6-sol", delta_rx, outcome_rx),
            producer
        );
        let error = result
            .err()
            .ok_or_else(|| std::io::Error::other("aggregate response limit did not fail"))?;
        assert!(
            error
                .to_string()
                .contains("non-stream response exceeds 8388608 bytes")
        );
        Ok(())
    }

    #[tokio::test]
    async fn non_stream_wire_limit_rejects_escape_expansion()
    -> Result<(), Box<dyn std::error::Error>> {
        let (delta_tx, delta_rx) = tokio::sync::mpsc::channel(1);
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let producer = async move {
            let chunk = "\n".repeat(64 * 1024);
            for _ in 0..80 {
                if delta_tx.send(chunk.clone()).await.is_err() {
                    break;
                }
            }
            drop(delta_tx);
            let _sent = outcome_tx.send(Ok(AssistantOutcome::Text {
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                limit_reached: false,
            }));
        };
        let (result, ()) = tokio::join!(
            assistant_message("gpt-5.6-sol", delta_rx, outcome_rx),
            producer
        );
        let error = result
            .err()
            .ok_or_else(|| std::io::Error::other("encoded response limit did not fail"))?;
        assert!(
            error
                .to_string()
                .contains("non-stream encoded response exceeds 8388608 bytes")
        );
        Ok(())
    }

    async fn fake_messages(headers: HeaderMap, body: Bytes) -> Response {
        assert_eq!(
            headers.get(header::AUTHORIZATION),
            Some(&HeaderValue::from_static("Bearer anthropic-subscription"))
        );
        assert!(!headers.contains_key(LEGACY_LOCAL_AUTH_HEADER));
        assert!(!headers.contains_key("x-api-key"));
        let document: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(document) => document,
            Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        };
        assert_eq!(
            document.get("model").and_then(serde_json::Value::as_str),
            Some("claude-fable-5")
        );
        (
            [(header::CONTENT_TYPE, "text/event-stream")],
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        )
            .into_response()
    }

    async fn fake_models(headers: HeaderMap) -> Response {
        assert_eq!(
            headers.get(header::AUTHORIZATION),
            Some(&HeaderValue::from_static("Bearer anthropic-subscription"))
        );
        assert!(!headers.contains_key(LEGACY_LOCAL_AUTH_HEADER));
        Json(serde_json::json!({
            "data": [{
                "id": "claude-fable-5",
                "type": "model",
                "display_name": "Claude Fable 5",
                "created_at": "2026-01-01T00:00:00Z"
            }],
            "has_more": false,
            "first_id": "claude-fable-5",
            "last_id": "claude-fable-5"
        }))
        .into_response()
    }

    async fn fake_redirect() -> Response {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, "https://attacker.invalid/capture")],
        )
            .into_response()
    }

    async fn test_router()
    -> Result<(Router, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
        let fake = Router::new()
            .route("/v1/messages", post(fake_messages))
            .route("/v1/models", get(fake_models))
            .route("/v1/models/{model}", get(fake_redirect));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, fake).await;
            assert!(result.is_ok());
        });
        let config = Config {
            listen: "127.0.0.1:0".parse::<SocketAddr>()?,
            ready_file: None,
            model: "gpt-5.6-sol".to_owned(),
            codex_bin: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/fake_codex.py"),
            cwd: std::env::current_dir()?,
            bearer: TEST_TOKEN.to_owned(),
        };
        let anthropic = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let anthropic_base_url = format!("http://{address}/").parse::<reqwest::Url>()?;
        Ok((
            routes(HttpState {
                bridge: Bridge::new(config),
                anthropic,
                anthropic_base_url,
            }),
            task,
        ))
    }

    #[tokio::test]
    async fn claude_model_routes_to_anthropic_with_isolated_auth()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, task) = test_router().await?;
        let request = Request::builder()
            .method("POST")
            .uri(format!("/{TEST_TOKEN}/v1/messages?beta=true"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(LEGACY_LOCAL_AUTH_HEADER, "must-not-reach-upstream")
            .header(CLAUDE_SESSION_HEADER, "claude-session-1")
            .header(header::AUTHORIZATION, "Bearer anthropic-subscription")
            .body(Body::from(
                serde_json::json!({
                    "model": "claude-fable-5",
                    "stream": true,
                    "max_tokens": 100,
                    "messages": [{"role": "user", "content": "hello"}]
                })
                .to_string(),
            ))?;
        let response = app.oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await?;
        assert!(String::from_utf8(body.to_vec())?.contains("message_stop"));
        task.abort();
        Ok(())
    }

    #[tokio::test]
    async fn model_list_preserves_anthropic_models() -> Result<(), Box<dyn std::error::Error>> {
        let (app, task) = test_router().await?;
        let request = Request::builder()
            .method("GET")
            .uri(format!("/{TEST_TOKEN}/v1/models"))
            .header(LEGACY_LOCAL_AUTH_HEADER, "must-not-reach-upstream")
            .header(CLAUDE_SESSION_HEADER, "claude-session-1")
            .header(header::AUTHORIZATION, "Bearer anthropic-subscription")
            .body(Body::empty())?;
        let response = app.oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
        let ids = body
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| std::io::Error::other("missing model data"))?
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["claude-fable-5"]);
        task.abort();
        Ok(())
    }

    #[tokio::test]
    async fn one_router_switches_claude_then_gpt_then_claude()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, task) = test_router().await?;
        for model in ["claude-fable-5", "gpt-5.6-sol", "claude-fable-5"] {
            let request = Request::builder()
                .method("POST")
                .uri(format!("/{TEST_TOKEN}/v1/messages"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(LEGACY_LOCAL_AUTH_HEADER, "must-not-reach-upstream")
                .header(CLAUDE_SESSION_HEADER, "claude-session-1")
                .header(header::AUTHORIZATION, "Bearer anthropic-subscription")
                .body(Body::from(
                    serde_json::json!({
                        "model": model,
                        "stream": true,
                        "max_tokens": 100,
                        "messages": [{"role": "user", "content": "hello"}]
                    })
                    .to_string(),
                ))?;
            let response = app.clone().oneshot(request).await?;
            assert_eq!(response.status(), StatusCode::OK);
            let body =
                String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
            if model.starts_with("claude-") {
                assert!(body.contains("message_stop"));
            } else {
                assert!(body.contains("hello from gpt"));
            }
        }
        task.abort();
        Ok(())
    }

    #[tokio::test]
    async fn anthropic_redirect_is_rejected_without_location()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, task) = test_router().await?;
        let request = Request::builder()
            .method("GET")
            .uri(format!("/{TEST_TOKEN}/v1/models/claude-redirect"))
            .header(LEGACY_LOCAL_AUTH_HEADER, "must-not-reach-upstream")
            .header(CLAUDE_SESSION_HEADER, "claude-session-1")
            .header(header::AUTHORIZATION, "Bearer anthropic-subscription")
            .body(Body::empty())?;
        let response = app.oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!response.headers().contains_key(header::LOCATION));
        task.abort();
        Ok(())
    }
}
