use std::{net::SocketAddr, path::Path, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use futures_util::StreamExt;
use model_rocket::{bridge::Bridge, config::Config, http};
use tokio::time::{Duration, timeout};
use tower::ServiceExt;

const BEARER: &str = "01234567890123456789012345678901";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn config() -> Result<Config, Box<dyn std::error::Error>> {
    config_with_fixture("fake_codex.py")
}

fn config_with_fixture(name: &str) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config {
        listen: "127.0.0.1:0".parse::<SocketAddr>()?,
        ready_file: None,
        model: "gpt-5.6-sol".to_owned(),
        codex_bin: fixture(name),
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        bearer: BEARER.to_owned(),
    })
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
        .uri("/v1/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-model-rocket-token", BEARER)
        .header("x-claude-code-session-id", session_id)
        .body(Body::from(body.to_string()))
}

#[tokio::test]
async fn text_request_returns_anthropic_sse() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("hello from gpt"));
    assert!(body.contains("\"stop_reason\":\"end_turn\""));
    assert!(body.contains("\"input_tokens\":12"));
    assert!(body.contains("\"output_tokens\":3"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}

#[tokio::test]
async fn non_stream_text_request_returns_anthropic_json() -> Result<(), Box<dyn std::error::Error>>
{
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
async fn non_stream_tool_request_returns_anthropic_json_without_fake_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
    let app = http::router(Bridge::new(config_with_fixture(
        "fake_codex_output_schema.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
    let app = http::router(Bridge::new(config()?))?;
    let first = app
        .clone()
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
            "model": "gpt-5.6-sol",
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
    assert!(second_body.contains("tool returned: sunny"));
    assert!(second_body.contains("\"stop_reason\":\"end_turn\""));
    assert!(second_body.contains("\"input_tokens\":8"));
    assert!(second_body.contains("\"output_tokens\":5"));
    Ok(())
}

#[tokio::test]
async fn wrong_bearer_is_rejected_before_app_server_work() -> Result<(), Box<dyn std::error::Error>>
{
    let app = http::router(Bridge::new(config()?))?;
    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-model-rocket-token", "wrong")
        .body(Body::from("{}"))?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn body_above_eight_mib_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
    let request = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-model-rocket-token", BEARER)
        .body(Body::from(vec![b'a'; 8 * 1024 * 1024 + 1]))?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    Ok(())
}

#[tokio::test]
async fn unconfigured_gpt_model_fails_without_fallback() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
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
    let app = http::router(Bridge::new(config()?))?;
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
    let app = http::router(Bridge::new(config()?))?;
    let request = Request::builder()
        .method("GET")
        .uri("/v1/models/gpt-5.6-sol")
        .header("x-model-rocket-token", BEARER)
        .body(Body::empty())?;
    let response = app.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(
        body.get("id").and_then(serde_json::Value::as_str),
        Some("gpt-5.6-sol")
    );
    Ok(())
}

#[tokio::test]
async fn tool_result_cannot_cross_claude_sessions() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
    let first = app
        .clone()
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": "gpt-5.6-sol",
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
        "model": "gpt-5.6-sol",
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

    let correct_session = app
        .oneshot(request_for_session(&continuation, "claude-session-a")?)
        .await?;
    let correct_body = String::from_utf8(
        to_bytes(correct_session.into_body(), 1024 * 1024)
            .await?
            .to_vec(),
    )?;
    assert!(correct_body.contains("tool returned: sunny"));
    Ok(())
}

#[tokio::test]
async fn gpt_delta_arrives_before_turn_completion() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config_with_fixture("fake_codex_stream.py")?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let early = timeout(Duration::from_millis(800), async {
        let mut captured = String::new();
        while let Some(chunk) = stream.next().await {
            captured.push_str(&String::from_utf8(chunk?.to_vec())?);
            if captured.contains("streamed early") {
                return Ok::<String, Box<dyn std::error::Error>>(captured);
            }
        }
        Err::<String, Box<dyn std::error::Error>>(
            std::io::Error::other("stream ended before first GPT delta").into(),
        )
    })
    .await??;
    assert!(early.contains("streamed early"));
    Ok(())
}

#[tokio::test]
async fn max_tokens_is_enforced_as_a_strict_utf8_byte_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config_with_fixture("fake_codex_stream.py")?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 8,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("\"text\":\"éstream\""));
    assert!(!body.contains("streamed early"));
    assert!(body.contains("\"stop_reason\":\"max_tokens\""));
    Ok(())
}

#[tokio::test]
async fn missing_final_usage_fails_instead_of_reporting_zero()
-> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config_with_fixture(
        "fake_codex_missing_usage.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("turn completed without final token usage"));
    assert!(!body.contains("\"stop_reason\":\"end_turn\""));
    Ok(())
}

#[tokio::test]
async fn tool_call_after_output_limit_does_not_override_max_tokens()
-> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config_with_fixture(
        "fake_codex_limit_tool.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 8,
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
    let app = http::router(Bridge::new(config_with_fixture(
        "fake_codex_oversized_frame.py",
    )?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
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
    let app = http::router(Bridge::new(config()?))?;
    let response = app
        .oneshot(request(&serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": [{"role": "user", "content": "FORBIDDEN_TOOL"}]
        }))?)
        .await?;
    let body = String::from_utf8(to_bytes(response.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("forbidden built-in item type commandExecution"));
    assert!(!body.contains("event: message_stop"));
    Ok(())
}

#[tokio::test]
async fn retained_tool_sessions_are_bounded_to_four() -> Result<(), Box<dyn std::error::Error>> {
    let app = http::router(Bridge::new(config()?))?;
    for session in 0..4 {
        let response = app
            .clone()
            .oneshot(request_for_session(
                &serde_json::json!({
                    "model": "gpt-5.6-sol",
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

    let rejected = app
        .oneshot(request_for_session(
            &serde_json::json!({
                "model": "gpt-5.6-sol",
                "max_tokens": 100,
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}]
            }),
            "fifth-session",
        )?)
        .await?;
    let body = String::from_utf8(to_bytes(rejected.into_body(), 1024 * 1024).await?.to_vec())?;
    assert!(body.contains("GPT route is at its four-session limit"));
    assert!(!body.contains("hello from gpt"));
    Ok(())
}
