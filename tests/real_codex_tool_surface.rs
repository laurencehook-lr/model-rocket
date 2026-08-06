use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri, header},
    routing::post,
};
use model_rocket::{
    config::PreflightConfig,
    contracts::json as json_contract,
    domain::ModelRoute,
    domain::{
        AssistantTextDelta, BridgeError, ClaudeToolName, DeveloperInstructions, ModelPrompt,
        OutputTokenLimit, StartModelTurn, ToolDefinition, ToolDescription, ToolSet,
        WorkingDirectory,
    },
    ports::{ModelOutput, PortFuture},
    test_support,
};

struct CaptureOutput(mpsc::Sender<String>);

impl ModelOutput for CaptureOutput {
    fn emit(&self, delta: AssistantTextDelta) -> PortFuture<'_, ()> {
        Box::pin(async move {
            self.0
                .send(delta.as_str().to_owned())
                .await
                .map_err(|_| BridgeError::unavailable("capture output receiver closed"))
        })
    }
}
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    time::{Duration, timeout},
};

#[derive(Debug)]
struct CapturedRequest {
    uri: Uri,
    content_encoding: Option<String>,
    body: Vec<u8>,
}

type Capture = Arc<Mutex<Option<oneshot::Sender<CapturedRequest>>>>;

async fn capture_request(
    State(capture): State<Capture>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    if let Some(sender) = capture.lock().await.take() {
        let _sent = sender.send(CapturedRequest {
            uri: uri.clone(),
            content_encoding: headers
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: body.to_vec(),
        });
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": {"message": "capture complete"}})),
    )
}

async fn capture_outbound_request(
    route: ModelRoute,
) -> Result<CapturedRequest, Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let endpoint = format!("http://{address}/").parse::<reqwest::Url>()?;
    let capture = Arc::new(Mutex::new(None));
    let capture_for_server = Arc::clone(&capture);
    let capture_server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .fallback(post(capture_request))
                .with_state(capture_for_server),
        )
        .await
    });

    let config = PreflightConfig::test_fixture_from_env()?;
    let mut app_server = timeout(
        Duration::from_secs(600),
        test_support::launch_for_wire_capture(
            config.codex_executable(),
            &endpoint,
            Arc::clone(config.catalogue()),
        ),
    )
    .await
    .map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "Codex App Server launch timed out for route {}",
                route.claude_model.as_str()
            ),
        )
    })??;
    let (capture_tx, capture_rx) = oneshot::channel();
    *capture.lock().await = Some(capture_tx);
    let cwd = std::env::current_dir()?;
    let mut task = tokio::spawn(async move {
        let tools = ToolSet::new(vec![ToolDefinition::new(
            ClaudeToolName::from("weather"),
            ToolDescription::new("Get weather"),
            json_contract::object(&json!({"type": "object"})).map_err(|error| {
                BridgeError::protocol(format!("test tool schema is invalid: {error}"))
            })?,
        )]);
        let (delta_tx, _delta_rx) = mpsc::channel(8);
        let output = CaptureOutput(delta_tx);
        app_server
            .start_turn(
                StartModelTurn::new(
                    route.codex_model.clone(),
                    WorkingDirectory::new(cwd.to_string_lossy().into_owned()),
                    tools,
                    ModelPrompt::new("Use the weather tool."),
                    DeveloperInstructions::new("Wire-capture contract test."),
                    OutputTokenLimit::new(100).map_err(|error| {
                        BridgeError::invalid_request(format!(
                            "test output limit is invalid: {error}"
                        ))
                    })?,
                    route.reasoning_effort,
                    route.service_tier,
                    None,
                ),
                &output,
            )
            .await
    });

    let captured = tokio::select! {
        captured = timeout(Duration::from_secs(20), capture_rx) => captured??,
        outcome = &mut task => {
            return Err(std::io::Error::other(format!(
                "production adapter turn ended before request capture: {outcome:?}"
            )).into());
        }
    };
    task.abort();
    let _aborted = task.await;
    capture_server.abort();
    let _aborted = capture_server.await;
    Ok(captured)
}

fn request_json(captured: &CapturedRequest) -> Result<Value, Box<dyn std::error::Error>> {
    let body = match captured.content_encoding.as_deref() {
        None => captured.body.clone(),
        Some("zstd") => zstd::stream::decode_all(captured.body.as_slice())?,
        Some(encoding) => {
            return Err(std::io::Error::other(format!(
                "production request uses unsupported test decoding {encoding}; {} raw bytes",
                captured.body.len()
            ))
            .into());
        }
    };
    Ok(serde_json::from_slice(&body)?)
}

#[tokio::test]
async fn real_codex_production_adapter_exposes_only_the_supplied_dynamic_tool()
-> Result<(), Box<dyn std::error::Error>> {
    let config = PreflightConfig::test_fixture_from_env()?;
    let route = config
        .catalogue()
        .routes()
        .iter()
        .find(|route| model_rocket::contracts::codex::service_tier(route.service_tier).is_none())
        .ok_or_else(|| std::io::Error::other("standard route missing"))?;
    let captured = capture_outbound_request(route.clone()).await?;
    assert!(
        captured.uri.path().contains("responses"),
        "unexpected production request URI: {}",
        captured.uri
    );
    let body = request_json(&captured)?;
    let tools = body
        .pointer("/input/0/tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "subscription Responses request at {} has no developer tools",
                captured.uri
            ))
        })?;
    assert_eq!(tools.len(), 1, "unexpected model-visible tools: {tools:?}");
    assert_eq!(
        tools
            .first()
            .and_then(|tool| tool.get("name"))
            .and_then(Value::as_str),
        Some("model_rocket_tool_0")
    );
    Ok(())
}

#[tokio::test]
async fn real_codex_production_adapter_preserves_every_route_policy_on_the_wire()
-> Result<(), Box<dyn std::error::Error>> {
    let config = PreflightConfig::test_fixture_from_env()?;
    for route in config.catalogue().routes() {
        let captured = capture_outbound_request(route.clone()).await?;
        let body = request_json(&captured)?;
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some(route.codex_model.as_str()),
            "wrong outbound model for {}",
            route.claude_model.as_str()
        );
        assert_eq!(
            body.get("service_tier").and_then(Value::as_str),
            model_rocket::contracts::codex::service_tier(route.service_tier),
            "wrong outbound service tier for {}",
            route.claude_model.as_str()
        );
        assert_eq!(
            body.pointer("/reasoning/effort").and_then(Value::as_str),
            Some(model_rocket::contracts::codex::reasoning_effort(
                route.reasoning_effort,
            )),
            "wrong outbound effort for {}",
            route.claude_model.as_str()
        );
    }
    Ok(())
}
