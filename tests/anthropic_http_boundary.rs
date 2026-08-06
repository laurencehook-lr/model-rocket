use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use model_rocket::{
    adapters::inbound::http,
    bootstrap,
    config::Config,
    contracts::anthropic::{MessageAccumulator, ResponseSequence},
    domain::BridgeError,
    domain::{
        AnthropicHeader, AnthropicHeaders, AnthropicRequest, AnthropicResponseChunk,
        AnthropicResponseHead, AnthropicStatus, AssistantOutcome, CompletionCause, TokenCount,
        TokenUsage,
    },
    policies::http_limits::MAX_NON_STREAM_RESPONSE_BYTES,
    ports::{AnthropicGateway, AnthropicResponseSink, ModelRouter, PortFuture},
};
use tower::ServiceExt;

const LOCAL_TOKEN: &str = "01234567890123456789012345678901";
const LOCAL_MODEL: &str = "anthropic-model-rocket-gpt-5.6-sol-normal-high";
const CLAUDE_MODEL: &str = "claude-fable-5";
const OAUTH: &str = "Bearer anthropic-subscription";

#[derive(Clone)]
enum GatewayPlan {
    Response {
        status: u16,
        headers: Vec<(&'static str, &'static [u8])>,
        chunks: Vec<&'static [u8]>,
    },
    Failure(&'static str),
}

struct RecordingGateway {
    requests: Mutex<Vec<AnthropicRequest>>,
    plan: GatewayPlan,
}

impl RecordingGateway {
    fn responding(
        status: u16,
        headers: Vec<(&'static str, &'static [u8])>,
        chunks: Vec<&'static [u8]>,
    ) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            plan: GatewayPlan::Response {
                status,
                headers,
                chunks,
            },
        }
    }

    fn failing(message: &'static str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            plan: GatewayPlan::Failure(message),
        }
    }

    fn requests(&self) -> Result<Vec<AnthropicRequest>, BridgeError> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| BridgeError::anthropic_unavailable("request lock poisoned"))
    }
}

impl AnthropicGateway for RecordingGateway {
    fn exchange<'a>(
        &'a self,
        request: AnthropicRequest,
        output: &'a dyn AnthropicResponseSink,
    ) -> PortFuture<'a, ()> {
        Box::pin(async move {
            self.requests
                .lock()
                .map_err(|_| BridgeError::anthropic_unavailable("request lock poisoned"))?
                .push(request);
            match &self.plan {
                GatewayPlan::Response {
                    status,
                    headers,
                    chunks,
                } => {
                    let fields = headers
                        .iter()
                        .map(|(name, value)| AnthropicHeader::new(*name, *value))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| BridgeError::anthropic_unavailable(error.to_string()))?;
                    output
                        .start(AnthropicResponseHead::new(
                            AnthropicStatus::new(*status).map_err(|error| {
                                BridgeError::anthropic_unavailable(error.to_string())
                            })?,
                            AnthropicHeaders::new(fields),
                        ))
                        .await?;
                    for chunk in chunks {
                        output.emit(AnthropicResponseChunk::new(*chunk)).await?;
                    }
                    Ok(())
                }
                GatewayPlan::Failure(message) => Err(BridgeError::anthropic_unavailable(*message)),
            }
        })
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn model_router(
    gateway: Arc<dyn AnthropicGateway>,
) -> Result<Arc<dyn ModelRouter>, Box<dyn std::error::Error>> {
    let config = Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &fixture("fake_codex.py"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        LOCAL_TOKEN.to_owned(),
    )?;
    Ok(bootstrap::model_router_with_anthropic(&config, gateway)?)
}

fn app(gateway: Arc<dyn AnthropicGateway>) -> Result<axum::Router, Box<dyn std::error::Error>> {
    let config = Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &fixture("fake_codex.py"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        LOCAL_TOKEN.to_owned(),
    )?;
    Ok(http::router(
        model_router(gateway)?,
        Arc::clone(config.catalogue()),
    ))
}

fn claude_message_request() -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(format!("/{LOCAL_TOKEN}/v1/messages?beta=true"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-model-rocket-token", "must-not-reach-upstream")
        .header("x-claude-code-session-id", "claude-session-1")
        .header(header::AUTHORIZATION, OAUTH)
        .body(Body::from(
            serde_json::json!({
                "model": CLAUDE_MODEL,
                "stream": true,
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "hello"}]
            })
            .to_string(),
        ))
}

fn gpt_message_request() -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(format!("/{LOCAL_TOKEN}/v1/messages"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-claude-code-session-id", "provider-switch-session")
        .body(Body::from(
            serde_json::json!({
                "model": LOCAL_MODEL,
                "stream": false,
                "max_tokens": 100,
                "messages": [{"role": "user", "content": "hello"}]
            })
            .to_string(),
        ))
}

#[tokio::test]
async fn local_authentication_rejects_wrong_path_token() -> Result<(), Box<dyn std::error::Error>> {
    let gateway = Arc::new(RecordingGateway::responding(200, Vec::new(), Vec::new()));
    let request = Request::builder()
        .method("GET")
        .uri("/wrong/v1/models")
        .header(header::AUTHORIZATION, OAUTH)
        .body(Body::empty())?;

    let response = app(gateway)?.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn anthropic_oauth_rejects_conflicting_api_key() -> Result<(), Box<dyn std::error::Error>> {
    let gateway = Arc::new(RecordingGateway::responding(200, Vec::new(), Vec::new()));
    let request = Request::builder()
        .method("GET")
        .uri(format!("/{LOCAL_TOKEN}/v1/models"))
        .header(header::AUTHORIZATION, OAUTH)
        .header("x-api-key", "must-not-be-accepted")
        .body(Body::empty())?;

    let response = app(gateway.clone())?.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(gateway.requests()?.is_empty());
    Ok(())
}

#[tokio::test]
async fn claude_message_routes_with_isolated_oauth_and_exact_body()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = Arc::new(RecordingGateway::responding(
        200,
        vec![("content-type", b"text/event-stream")],
        vec![b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"],
    ));
    let request = claude_message_request()?;

    let response = app(gateway.clone())?.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("message_stop"));
    let requests = gateway.requests()?;
    assert_eq!(requests.len(), 1);
    let captured = requests
        .first()
        .ok_or_else(|| std::io::Error::other("missing captured request"))?;
    assert_eq!(captured.query().as_deref(), Some("beta=true"));
    let names = captured
        .headers()
        .as_slice()
        .iter()
        .map(model_rocket::domain::AnthropicHeader::name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"authorization"));
    assert!(!names.contains(&"x-model-rocket-token"));
    assert!(!names.contains(&"x-api-key"));
    assert!(matches!(
        captured.body(),
        model_rocket::domain::AnthropicRequestBody::Json(document)
            if document.as_str().contains(CLAUDE_MODEL)
    ));
    Ok(())
}

#[tokio::test]
async fn one_router_switches_claude_then_gpt_then_claude_per_request()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = Arc::new(RecordingGateway::responding(
        200,
        vec![("content-type", b"text/event-stream")],
        vec![b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"],
    ));
    let app = app(gateway.clone())?;

    let first_claude = app.clone().oneshot(claude_message_request()?).await?;
    assert_eq!(first_claude.status(), StatusCode::OK);
    let _first_body = to_bytes(first_claude.into_body(), 1024 * 1024).await?;
    assert_eq!(gateway.requests()?.len(), 1);

    let gpt = app.clone().oneshot(gpt_message_request()?).await?;
    assert_eq!(gpt.status(), StatusCode::OK);
    let gpt_body: serde_json::Value =
        serde_json::from_slice(&to_bytes(gpt.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        gpt_body
            .pointer("/content/0/text")
            .and_then(serde_json::Value::as_str),
        Some("hello from gpt")
    );
    assert_eq!(gateway.requests()?.len(), 1);

    let second_claude = app.oneshot(claude_message_request()?).await?;
    assert_eq!(second_claude.status(), StatusCode::OK);
    let _second_body = to_bytes(second_claude.into_body(), 1024 * 1024).await?;
    assert_eq!(gateway.requests()?.len(), 2);
    Ok(())
}

#[tokio::test]
async fn model_list_preserves_cursor_query_and_response_body()
-> Result<(), Box<dyn std::error::Error>> {
    let upstream_body = br#"{"data":[{"id":"claude-fable-5","type":"model"}],"has_more":false}"#;
    let gateway = Arc::new(RecordingGateway::responding(
        200,
        vec![("content-type", b"application/json")],
        vec![upstream_body],
    ));
    let request = Request::builder()
        .method("GET")
        .uri(format!(
            "/{LOCAL_TOKEN}/v1/models?after_id=claude-fable-5&limit=1"
        ))
        .header(header::AUTHORIZATION, OAUTH)
        .body(Body::empty())?;

    let response = app(gateway.clone())?.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024 * 1024).await?.as_ref(),
        upstream_body
    );
    let requests = gateway.requests()?;
    let captured = requests
        .first()
        .ok_or_else(|| std::io::Error::other("missing captured request"))?;
    assert_eq!(
        captured.query().as_deref(),
        Some("after_id=claude-fable-5&limit=1")
    );
    Ok(())
}

#[tokio::test]
async fn anthropic_redirect_failure_has_no_location_header()
-> Result<(), Box<dyn std::error::Error>> {
    let gateway = Arc::new(RecordingGateway::failing("redirects are not permitted"));
    let request = Request::builder()
        .method("GET")
        .uri(format!("/{LOCAL_TOKEN}/v1/models/claude-redirect"))
        .header(header::AUTHORIZATION, OAUTH)
        .body(Body::empty())?;

    let response = app(gateway)?.oneshot(request).await?;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!response.headers().contains_key(header::LOCATION));
    Ok(())
}

#[test]
fn text_outcome_has_complete_stream_shape() -> Result<(), Box<dyn std::error::Error>> {
    let mut sequence = ResponseSequence::new(LOCAL_MODEL)?;
    let _message_start = sequence
        .pop()
        .ok_or_else(|| std::io::Error::other("missing message_start"))?;
    sequence.accept_delta("hello")?;
    while sequence.pop().is_some() {}
    sequence.finish(AssistantOutcome::Text {
        usage: Some(TokenUsage::new(TokenCount::new(12), TokenCount::new(3))),
        cause: CompletionCause::EndTurn,
    })?;

    assert_eq!(std::iter::from_fn(|| sequence.pop()).count(), 3);
    Ok(())
}

#[test]
fn non_stream_aggregate_limit_rejects_repeated_small_deltas()
-> Result<(), Box<dyn std::error::Error>> {
    let mut accumulator = MessageAccumulator::new(LOCAL_MODEL.to_owned());
    let chunk = "x".repeat(64 * 1024);
    let mut error = None;
    for _ in 0..=(MAX_NON_STREAM_RESPONSE_BYTES / chunk.len()) {
        if let Err(found) = accumulator.push(&chunk) {
            error = Some(found);
            break;
        }
    }

    let error = error.ok_or_else(|| std::io::Error::other("aggregate response limit must fail"))?;
    assert!(
        error
            .to_string()
            .contains("non-stream response exceeds 8388608 bytes")
    );
    Ok(())
}

#[test]
fn non_stream_wire_limit_rejects_escape_expansion() -> Result<(), Box<dyn std::error::Error>> {
    let mut accumulator = MessageAccumulator::new(LOCAL_MODEL.to_owned());
    let chunk = "\n".repeat(64 * 1024);
    for _ in 0..80 {
        accumulator.push(&chunk)?;
    }
    let result = accumulator.finish(AssistantOutcome::Text {
        usage: Some(TokenUsage::new(TokenCount::new(1), TokenCount::new(1))),
        cause: CompletionCause::EndTurn,
    });
    let Err(error) = result else {
        return Err(std::io::Error::other("encoded response limit must fail").into());
    };

    assert!(
        error
            .to_string()
            .contains("non-stream encoded response exceeds 8388608 bytes")
    );
    Ok(())
}
