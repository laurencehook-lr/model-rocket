use std::{fs, net::SocketAddr, path::Path, path::PathBuf, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use futures_util::{StreamExt, future::join_all};
use model_rocket::{
    bootstrap, config::Config, domain::BridgeError,
    policies::router_limits::MAX_CONCURRENT_GPT_TURNS, ports::ModelRouter,
};
use tokio::time::{Duration, timeout};
use tower::ServiceExt;

const BEARER: &str = "01234567890123456789012345678901";
const ROUTE_MODEL: &str = "anthropic-model-rocket-gpt-5.6-sol-normal-high";

fn test_model_router(config: &Config) -> Result<Arc<dyn ModelRouter>, BridgeError> {
    bootstrap::model_router(config)
}

mod test_http {
    use std::sync::Arc;

    use axum::Router;
    use model_rocket::{bootstrap, config::Config, domain::BridgeError, ports::ModelRouter};

    pub(super) fn router(
        model_router: Result<Arc<dyn ModelRouter>, BridgeError>,
    ) -> Result<Router, BridgeError> {
        let config = Config::test_fixture(
            "127.0.0.1:0".parse().map_err(|error| {
                BridgeError::configuration(format!("invalid test listener: {error}"))
            })?,
            None,
            std::path::Path::new("/usr/bin/true"),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            "01234567890123456789012345678901".to_owned(),
        )?;
        bootstrap::http_router(model_router?, Arc::clone(config.catalogue()))
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn config() -> Result<Config, Box<dyn std::error::Error>> {
    config_with_fixture("fake_codex.py")
}

fn config_with_fixture(name: &str) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &fixture(name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        BEARER.to_owned(),
    )?)
}

fn config_with_catalogue_fixture(
    fixture_name: &str,
    catalogue_path: &Path,
) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config::test_fixture_with_catalogue(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &fixture(fixture_name),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        BEARER.to_owned(),
        catalogue_path,
    )?)
}

fn request(body: &serde_json::Value) -> Result<Request<Body>, axum::http::Error> {
    request_for_session(body, "claude-session-1")
}

fn request_for_session(
    body: &serde_json::Value,
    session_id: &str,
) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(format!("/{BEARER}/v1/messages"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-claude-code-session-id", session_id)
        .body(Body::from(body.to_string()))
}

fn streamed_text(body: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    for line in body.lines().filter_map(|line| line.strip_prefix("data: ")) {
        let event: serde_json::Value = serde_json::from_str(line)?;
        if event.get("type").and_then(serde_json::Value::as_str) == Some("content_block_delta")
            && let Some(text) = event
                .pointer("/delta/text")
                .and_then(serde_json::Value::as_str)
        {
            output.push_str(text);
        }
    }
    Ok(output)
}

#[tokio::test]
async fn text_request_returns_anthropic_sse() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("event: message_start"));
    assert_eq!(streamed_text(&body)?, "hello from gpt");
    assert!(body.contains("\"stop_reason\":\"end_turn\""));
    assert!(body.contains("\"input_tokens\":12"));
    assert!(body.contains("\"output_tokens\":3"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}

#[tokio::test]
async fn non_stream_text_request_returns_anthropic_json() -> Result<(), Box<dyn std::error::Error>>
{
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static("application/json"))
    );
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.pointer("/content/0/text")
            .and_then(serde_json::Value::as_str),
        Some("hello from gpt")
    );
    assert_eq!(
        body.get("stop_reason").and_then(serde_json::Value::as_str),
        Some("end_turn")
    );
    assert_eq!(
        body.pointer("/usage/input_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(12)
    );
    assert_eq!(
        body.pointer("/usage/output_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    Ok(())
}

#[tokio::test]
async fn production_router_builds_prompt_and_developer_instructions_for_app_server()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 37,
            "stream": false,
            "system": "system policy",
            "messages": [
                {"role": "user", "content": "ASSERT_PROMPT_POLICY"},
                {"role": "assistant", "content": [
                    {"type": "future_valid_block", "new_field": {"keep": true}}
                ]}
            ]
        }))?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.pointer("/content/0/text")
            .and_then(serde_json::Value::as_str),
        Some("hello from gpt")
    );
    Ok(())
}

#[tokio::test]
async fn non_stream_tool_request_returns_anthropic_json_without_fake_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": false,
            "messages": [{"role": "user", "content": "CALL_TOOL"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.pointer("/content/0/type")
            .and_then(serde_json::Value::as_str),
        Some("tool_use")
    );
    assert_eq!(
        body.pointer("/content/0/id")
            .and_then(serde_json::Value::as_str),
        Some("call_1")
    );
    assert_eq!(
        body.get("stop_reason").and_then(serde_json::Value::as_str),
        Some("tool_use")
    );
    assert!(body.get("usage").is_none());
    Ok(())
}

#[tokio::test]
async fn reserved_mcp_tool_name_round_trips_through_safe_codex_alias()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": false,
            "messages": [{"role": "user", "content": "CALL_TOOL"}],
            "tools": [{
                "name": "mcp__plugin_context7_context7__query-docs",
                "description": "Query documentation",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.pointer("/content/0/name")
            .and_then(serde_json::Value::as_str),
        Some("mcp__plugin_context7_context7__query-docs")
    );
    Ok(())
}

#[tokio::test]
async fn non_stream_request_rejects_output_limit_above_server_cap()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": u32::MAX,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("non-stream max_tokens exceeds 8388608-byte response limit"));
    Ok(())
}

#[tokio::test]
async fn non_stream_structured_output_maps_schema_to_app_server()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_output_schema.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": false,
            "messages": [{"role": "user", "content": "Create a title"}],
            "output_config": {
                "effort": "low",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": {"title": {"type": "string"}},
                        "required": ["title"],
                        "additionalProperties": false
                    }
                }
            }
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.pointer("/content/0/text")
            .and_then(serde_json::Value::as_str),
        Some("{\"title\":\"bridge\"}")
    );
    Ok(())
}

#[tokio::test]
async fn tool_result_continues_original_turn() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let first = app
        .clone()
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "CALL_TOOL"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let first_body = String::from_utf8(to_bytes(first.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(first_body.contains("\"type\":\"tool_use\""));
    assert!(first_body.contains("\"id\":\"call_1\""));
    assert!(first_body.contains("\"stop_reason\":\"tool_use\""));
    assert!(!first_body.contains("\"input_tokens\":20"));
    assert!(!first_body.contains("\"output_tokens\":4"));

    let second = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [
                {"role": "user", "content": "CALL_TOOL"},
                {"role": "assistant", "content": [{
                    "type": "tool_use", "id": "call_1", "name": "weather", "input": {"city": "London"}
                }]},
                {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "call_1", "content": "sunny"
                }]}
            ],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let second_body = String::from_utf8(to_bytes(second.into_body(), 1024 * 1024).await?.to_vec())?;
    assert_eq!(streamed_text(&second_body)?, "tool returned: sunny");
    assert!(second_body.contains("\"stop_reason\":\"end_turn\""));
    assert!(second_body.contains("\"input_tokens\":8"));
    assert!(second_body.contains("\"output_tokens\":5"));
    Ok(())
}

#[tokio::test]
async fn malformed_tool_continuation_envelopes_fail_without_discarding_content()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    for messages in [
        serde_json::json!([{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"},
            {"type": "text", "text": "must not be discarded"}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "text", "text": "must not be discarded"},
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
        ]}]),
        serde_json::json!([{"role": "assistant", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
        ]}]),
    ] {
        let response = app
            .clone()
            .oneshot(request(&serde_json::json!({
                "model": ROUTE_MODEL,
                "max_tokens": 100,
                "stream": true,
                "messages": messages
            }))?)
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
        assert!(body.contains("exactly one tool_result block"));
    }
    Ok(())
}

#[tokio::test]
async fn wrong_bearer_is_rejected_before_app_server_work() -> Result<(), Box<dyn std::error::Error>>
{
    let app = test_http::router(test_model_router(&config()?))?;
    let request = Request::builder()
        .method("POST")
        .uri("/wrong/v1/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn body_above_eight_mib_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let request = Request::builder()
        .method("POST")
        .uri(format!("/{BEARER}/v1/messages"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(vec![b'a'; 8 * 1024 * 1024 + 1]))?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
}

#[tokio::test]
async fn unconfigured_gpt_model_fails_without_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-not-configured",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("is not configured"));
    Ok(())
}

#[tokio::test]
async fn claude_model_requires_subscription_oauth() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "claude-fable-5",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn configured_gpt_model_has_model_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let request = Request::builder()
        .method("GET")
        .uri(format!("/{BEARER}/v1/models/{ROUTE_MODEL}"))
        .body(Body::empty())?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.get("id").and_then(serde_json::Value::as_str),
        Some(ROUTE_MODEL)
    );
    Ok(())
}

#[tokio::test]
async fn every_route_enforces_its_service_tier_and_reasoning_effort()
-> Result<(), Box<dyn std::error::Error>> {
    let markers = [
        "ASSERT_ROUTE_NORMAL_LOW",
        "ASSERT_ROUTE_NORMAL_HIGH",
        "ASSERT_ROUTE_FAST_LOW",
        "ASSERT_ROUTE_FAST_HIGH",
    ];
    let configured = config()?;
    for (route, marker) in configured.catalogue().routes().iter().zip(markers) {
        let app = test_http::router(test_model_router(&configured))?;
        let response = app
            .oneshot(request(&serde_json::json!({
                "model": route.claude_model.as_str(),
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": marker}],
                "output_config": {"effort": "max"}
            }))?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
        assert_eq!(
            streamed_text(&body)?,
            "hello from gpt",
            "{marker} did not complete through its configured route: {body}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn tool_result_cannot_cross_claude_sessions() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let first = app
        .clone()
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": ROUTE_MODEL,
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "CALL_TOOL"}],
                "tools": [{
                    "name": "weather",
                    "description": "Get weather",
                    "input_schema": {"type": "object"}
                }]
            }),
            "claude-session-a",
        )?)
        .await?;
    let first_body = String::from_utf8(to_bytes(first.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(first_body.contains("\"id\":\"call_1\""));

    let continuation = serde_json::json!({
        "model": ROUTE_MODEL,
        "max_tokens": 100,
        "stream": true,
        "messages": [{"role": "user", "content": [{
            "type": "tool_result",
            "tool_use_id": "call_1",
            "content": "sunny"
        }]}]
    });
    let wrong_session = app
        .clone()
        .oneshot(request_for_session(&continuation, "claude-session-b")?)
        .await?;
    let wrong_body = String::from_utf8(
        to_bytes(wrong_session.into_body(), 1024 * 1024)
            .await?
            .to_vec(),
    )?;
    assert!(wrong_body.contains("no matching pending tool call"));

    let mut wrong_model = continuation.clone();
    let configured = config()?;
    let alternate_model = configured
        .catalogue()
        .routes()
        .iter()
        .map(|route| route.claude_model.as_str())
        .find(|model| *model != ROUTE_MODEL)
        .ok_or_else(|| std::io::Error::other("alternate route missing"))?;
    wrong_model
        .as_object_mut()
        .ok_or_else(|| std::io::Error::other("continuation is not an object"))?
        .insert(
            "model".to_owned(),
            serde_json::Value::String(alternate_model.to_owned()),
        );
    let wrong_model_response = app
        .clone()
        .oneshot(request_for_session(&wrong_model, "claude-session-a")?)
        .await?;
    let wrong_model_body = String::from_utf8(
        to_bytes(wrong_model_response.into_body(), 1024 * 1024)
            .await?
            .to_vec(),
    )?;
    assert!(wrong_model_body.contains("model does not match"));

    let correct_session = app
        .oneshot(request_for_session(&continuation, "claude-session-a")?)
        .await?;
    let correct_body = String::from_utf8(
        to_bytes(correct_session.into_body(), 1024 * 1024)
            .await?
            .to_vec(),
    )?;
    assert_eq!(streamed_text(&correct_body)?, "tool returned: sunny");
    Ok(())
}

#[tokio::test]
async fn gpt_delta_arrives_before_turn_completion() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_stream.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let early = timeout(Duration::from_secs(5), async {
        let mut captured = String::new();
        while let Some(chunk) = stream.next().await {
            captured.push_str(&String::from_utf8(chunk?.to_vec())?);
            if captured.contains("streamed") {
                return Ok::<String, Box<dyn std::error::Error>>(captured);
            }
        }
        Err::<String, Box<dyn std::error::Error>>(
            std::io::Error::other("stream ended before first GPT delta").into(),
        )
    })
    .await??;
    assert!(early.contains("streamed"));
    Ok(())
}

#[tokio::test]
async fn max_tokens_is_enforced_as_a_strict_gpt_token_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_overflow_interrupted.py",
    )?))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        app.oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 2,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?),
    )
    .await??;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    let tokenizer = tiktoken_rs::o200k_base()?;
    let source = "éstreamed beyond limit";
    let source_tokens = tokenizer.encode_ordinary(source);
    assert!(source_tokens.len() > 2);
    let expected_prefix = tokenizer.decode(
        source_tokens
            .get(..2)
            .ok_or_else(|| std::io::Error::other("missing expected GPT token prefix"))?,
    )?;
    assert_eq!(tokenizer.encode_ordinary(&expected_prefix).len(), 2);
    let expected_event = format!("\"text\":{}", serde_json::to_string(&expected_prefix)?);
    assert!(body.contains(&expected_event));
    assert!(!body.contains("streamed beyond limit"));
    assert!(body.contains("\"stop_reason\":\"max_tokens\""));
    assert_eq!(body.matches("\"usage\"").count(), 1);
    Ok(())
}

#[tokio::test]
async fn failed_turn_after_output_limit_still_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_overflow_failed.py",
    )?))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        app.oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 2,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?),
    )
    .await??;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("provider failed after interrupt"));
    assert!(!body.contains("\"stop_reason\":\"max_tokens\""));
    Ok(())
}

#[tokio::test]
async fn interrupt_rpc_error_fails_instead_of_waiting_for_a_terminal_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_interrupt_error.py",
    )?))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        app.oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 1,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?),
    )
    .await??;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("turn/interrupt failed with -32000: interrupt rejected"));
    assert!(!body.contains("\"stop_reason\":\"max_tokens\""));
    Ok(())
}

#[tokio::test]
async fn fatal_error_after_output_interrupt_cannot_be_masked_as_max_tokens()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_overflow_error_interrupted.py",
    )?))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        app.oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 2,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?),
    )
    .await??;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("fatal provider error after interrupt"));
    assert!(!body.contains("\"stop_reason\":\"max_tokens\""));
    Ok(())
}

#[tokio::test]
async fn foreign_tool_call_after_output_interrupt_still_fails_scope_validation()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_overflow_foreign_tool.py",
    )?))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        app.oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 2,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?),
    )
    .await??;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("dynamic tool call does not match the active turn"));
    assert!(!body.contains("\"stop_reason\":\"max_tokens\""));
    Ok(())
}

#[tokio::test]
async fn tool_continuation_rejects_usage_that_predates_new_text()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_stale_tool_usage.py",
    )?))?;
    let first = app
        .clone()
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "CALL_TOOL"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let first_body = String::from_utf8(to_bytes(first.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(first_body.contains("\"id\":\"call_stale\""));

    let second = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [
                {"role": "user", "content": "CALL_TOOL"},
                {"role": "assistant", "content": [{
                    "type": "tool_use", "id": "call_stale", "name": "weather", "input": {"city": "London"}
                }]},
                {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "call_stale", "content": "sunny"
                }]}
            ],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let second_body = String::from_utf8(to_bytes(second.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(second_body.contains("text-producing turn completed without final token usage"));
    assert!(!second_body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn empty_tool_continuation_omits_usage_from_the_prior_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_stale_tool_usage_empty.py",
    )?))?;
    let first = app
        .clone()
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "CALL_TOOL"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let first_body = String::from_utf8(to_bytes(first.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(first_body.contains("\"id\":\"call_stale_empty\""));

    let second = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": false,
            "messages": [
                {"role": "user", "content": "CALL_TOOL"},
                {"role": "assistant", "content": [{
                    "type": "tool_use", "id": "call_stale_empty", "name": "weather", "input": {"city": "London"}
                }]},
                {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "call_stale_empty", "content": "sunny"
                }]}
            ],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(second.into_body(), 1024 * 1024).await?)?;
    assert_eq!(body.get("content"), Some(&serde_json::json!([])));
    assert!(body.get("usage").is_none());
    Ok(())
}

#[tokio::test]
async fn forbidden_builtin_item_in_terminal_snapshot_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_completion_forbidden.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("Codex attempted forbidden built-in item type commandExecution"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn foreign_turn_delta_fails_before_text_is_emitted() -> Result<(), Box<dyn std::error::Error>>
{
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_mismatched_delta.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("agent message delta does not match the active turn"));
    assert!(!body.contains("must not escape"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn turn_delta_without_thread_id_fails_immediately_in_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_missing_thread_delta.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("item/agentMessage/delta has no thread id"));
    assert!(!body.contains("must not escape"));
    assert!(!body.contains("timed out"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn recognized_notification_without_an_active_thread_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_orphan_notification.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("received event for unknown active thread thread_orphan"));
    assert!(!body.contains("normal response"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn foreign_turn_completion_cannot_terminate_the_active_turn()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_mismatched_completion.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("turn/completed does not match the active turn"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn missing_final_usage_fails_instead_of_reporting_zero()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_missing_usage.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("text-producing turn completed without final token usage"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn empty_auxiliary_turn_without_usage_completes_without_fabrication()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_empty_missing_usage.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 1,
            "stream": false,
            "messages": [{"role": "user", "content": "model availability probe"}]
        }))?)
        .await?;
    let status = response.status();
    let encoded = to_bytes(response.into_body(), 1024 * 1024).await?;
    if status != StatusCode::OK {
        return Err(std::io::Error::other(format!(
            "empty auxiliary request returned {status}: {}",
            String::from_utf8_lossy(&encoded)
        ))
        .into());
    }
    let body: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(body.get("content"), Some(&serde_json::json!([])));
    assert!(body.get("usage").is_none());
    Ok(())
}

#[tokio::test]
async fn exact_output_ceiling_waits_for_natural_completion_and_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_exact_ceiling.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 1,
            "stream": false,
            "messages": [{"role": "user", "content": "configuration"}]
        }))?)
        .await?;
    let status = response.status();
    let encoded = to_bytes(response.into_body(), 1024 * 1024).await?;
    if status != StatusCode::OK {
        return Err(std::io::Error::other(format!(
            "exact-ceiling request returned {status}: {}",
            String::from_utf8_lossy(&encoded)
        ))
        .into());
    }
    let body: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert_eq!(
        body.get("content"),
        Some(&serde_json::json!([{"type": "text", "text": " configuration"}]))
    );
    assert_eq!(
        body.get("stop_reason"),
        Some(&serde_json::json!("end_turn"))
    );
    assert_eq!(
        body.get("usage"),
        Some(&serde_json::json!({"input_tokens": 4, "output_tokens": 1}))
    );
    Ok(())
}

#[tokio::test]
async fn terminal_unstable_overflow_is_truncated_without_protocol_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_terminal_overflow.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 1,
            "stream": false,
            "messages": [{"role": "user", "content": "configuration"}]
        }))?)
        .await?;
    let status = response.status();
    let encoded = to_bytes(response.into_body(), 1024 * 1024).await?;
    if status != StatusCode::OK {
        return Err(std::io::Error::other(format!(
            "terminal-overflow request returned {status}: {}",
            String::from_utf8_lossy(&encoded)
        ))
        .into());
    }
    let body: serde_json::Value = serde_json::from_slice(&encoded)?;
    let content = body
        .pointer("/content/0/text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| std::io::Error::other("terminal-overflow response has no text"))?;
    assert_eq!(tiktoken_rs::o200k_base()?.encode_ordinary(content).len(), 1);
    assert_eq!(
        body.get("stop_reason"),
        Some(&serde_json::json!("max_tokens"))
    );
    assert_eq!(
        body.get("usage"),
        Some(&serde_json::json!({"input_tokens": 4, "output_tokens": 2}))
    );
    Ok(())
}

#[tokio::test]
async fn tool_call_after_output_limit_does_not_override_max_tokens()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_limit_tool.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 2,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{
                "name": "weather",
                "description": "Get weather",
                "input_schema": {"type": "object"}
            }]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("\"stop_reason\":\"max_tokens\""));
    assert!(!body.contains("\"type\":\"tool_use\""));
    Ok(())
}

#[tokio::test]
async fn oversized_app_server_frame_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_oversized_frame.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("App Server frame exceeds 8388608 bytes"));
    assert!(!body.contains("event: message_stop"));
    Ok(())
}

#[tokio::test]
async fn forbidden_builtin_tool_lifecycle_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": ROUTE_MODEL,
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "FORBIDDEN_TOOL"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("forbidden built-in item type plan"), "{body}");
    assert!(!body.contains("event: message_stop"));
    Ok(())
}

#[tokio::test]
async fn requests_beyond_one_scheduler_window_are_queued_and_multiplexed()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_multiplex.py",
    )?))?;
    let request_count = MAX_CONCURRENT_GPT_TURNS * 2;
    let mut calls = Vec::with_capacity(request_count);
    for session in 0..request_count {
        let request = request_for_session(
            &serde_json::json!({
                "model": ROUTE_MODEL,
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            &format!("multiplexed-session-{session}"),
        )?;
        calls.push(app.clone().oneshot(request));
    }

    let responses = timeout(Duration::from_secs(10), join_all(calls)).await?;
    assert_eq!(responses.len(), request_count);
    for response in responses {
        let response = response?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
        assert!(body.contains("event: message_stop"));
        assert!(body.contains("hello from thread_"));
    }
    Ok(())
}

#[tokio::test]
async fn retained_tool_sessions_do_not_consume_turn_scheduler_capacity()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_retained_sessions.py",
    )?))?;
    let retained_count = MAX_CONCURRENT_GPT_TURNS * 2;
    for session in 0..retained_count {
        let response = app
            .clone()
            .oneshot(request_for_session(
                &serde_json::json!({
                    "model": ROUTE_MODEL,
                    "max_tokens": 100,
                    "stream": true,
                    "messages": [{"role": "user", "content": "CALL_TOOL"}],
                    "tools": [{
                        "name": "weather",
                        "description": "Get weather",
                        "input_schema": {"type": "object"}
                    }]
                }),
                &format!("retained-session-{session}"),
            )?)
            .await?;
        let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
        assert!(body.contains("\"stop_reason\":\"tool_use\""));
    }

    let response = app
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": ROUTE_MODEL,
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            "fresh-session-after-retained-tools",
        )?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert_eq!(streamed_text(&body)?, "fresh request succeeded");
    assert!(!body.contains("session safety limit"));
    Ok(())
}

#[tokio::test]
async fn next_request_restarts_the_shared_process_after_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let app = test_http::router(test_model_router(&config_with_fixture(
        "fake_codex_recovery.py",
    )?))?;
    let message = serde_json::json!({
        "model": ROUTE_MODEL,
        "max_tokens": 100,
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });

    let failed = app
        .clone()
        .oneshot(request_for_session(&message, "failed-process-session")?)
        .await?;
    let failed_body = String::from_utf8(to_bytes(failed.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(failed_body.contains("Codex App Server closed stdout"));

    let recovered = app
        .oneshot(request_for_session(&message, "recovered-process-session")?)
        .await?;
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered_body =
        String::from_utf8(to_bytes(recovered.into_body(), 1024 * 1024).await?.to_vec())?;
    assert_eq!(
        streamed_text(&recovered_body)?,
        "recovered process succeeded"
    );
    Ok(())
}

#[tokio::test]
async fn two_model_routes_survive_shared_process_recovery_with_the_same_catalogue()
-> Result<(), Box<dyn std::error::Error>> {
    let catalogue_path = std::env::temp_dir().join(format!(
        "model-rocket-two-model-route-recovery-{}.json",
        uuid::Uuid::now_v7()
    ));
    fs::write(
        &catalogue_path,
        r#"{
          "schema_version": 1,
          "canonical_route": "anthropic-model-rocket-gpt-a-high",
          "models": [
            {"id":"gpt-a","display_name":"A","description":"A model","context_tokens":1000000},
            {"id":"gpt-b","display_name":"B","description":"B model","context_tokens":250000}
          ],
          "routes": [
            {"id":"anthropic-model-rocket-gpt-a-high","display_name":"A High","description":"A high","model":"gpt-a","delivery":"standard","reasoning":"high"},
            {"id":"anthropic-model-rocket-gpt-b-low","display_name":"B Low","description":"B low","model":"gpt-b","delivery":"fast","reasoning":"low"}
          ]
        }"#,
    )?;
    let recovery_state = std::env::temp_dir().join(format!(
        "model-rocket-two-model-recovery-{}.state",
        std::process::id()
    ));
    let _removed_stale_state = fs::remove_file(&recovery_state);
    let config =
        config_with_catalogue_fixture("fake_codex_two_model_recovery.py", &catalogue_path)?;
    let app = bootstrap::http_router(test_model_router(&config)?, Arc::clone(config.catalogue()))?;

    let first = app
        .clone()
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": "anthropic-model-rocket-gpt-a-high",
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "first model"}]
            }),
            "two-model-a",
        )?)
        .await?;
    let first_body = String::from_utf8(to_bytes(first.into_body(), 1024 * 1024).await?.to_vec())?;
    assert_eq!(streamed_text(&first_body)?, "gpt-a via launch 1");

    let failed = app
        .clone()
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": "anthropic-model-rocket-gpt-b-low",
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "force recovery"}]
            }),
            "two-model-b-failed",
        )?)
        .await?;
    let failed_body = String::from_utf8(to_bytes(failed.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(failed_body.contains("Codex App Server closed stdout"));

    let recovered = app
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": "anthropic-model-rocket-gpt-b-low",
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "second model"}]
            }),
            "two-model-b-recovered",
        )?)
        .await?;
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered_body =
        String::from_utf8(to_bytes(recovered.into_body(), 1024 * 1024).await?.to_vec())?;
    assert_eq!(streamed_text(&recovered_body)?, "gpt-b via launch 2");
    assert_eq!(fs::read_to_string(&recovery_state)?, "2");

    fs::remove_file(catalogue_path)?;
    fs::remove_file(recovery_state)?;
    Ok(())
}
