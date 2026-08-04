use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, oneshot},
    time::{Duration, timeout},
};

type Capture = Arc<Mutex<Option<oneshot::Sender<Value>>>>;

async fn capture_request(
    State(capture): State<Capture>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if let Some(sender) = capture.lock().await.take() {
        let _sent = sender.send(body);
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": {"message": "capture complete"}})),
    )
}

async fn send(stdin: &mut ChildStdin, value: Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoded = serde_json::to_vec(&value)?;
    encoded.push(b'\n');
    stdin.write_all(&encoded).await?;
    stdin.flush().await?;
    Ok(())
}

async fn response(
    stdout: &mut BufReader<ChildStdout>,
    expected_id: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    timeout(Duration::from_secs(10), async {
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).await? == 0 {
                return Err(std::io::Error::other("real App Server closed stdout").into());
            }
            let message: Value = serde_json::from_str(&line)?;
            if message.get("id").and_then(Value::as_u64) == Some(expected_id) {
                if let Some(error) = message.get("error") {
                    return Err(std::io::Error::other(format!(
                        "real App Server request failed: {error}"
                    ))
                    .into());
                }
                return Ok(message);
            }
        }
    })
    .await?
}

fn configure_codex_home(
    address: std::net::SocketAddr,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let codex_home = std::env::temp_dir().join(format!(
        "model-rocket-real-tool-surface-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7()
    ));
    std::fs::create_dir(&codex_home)?;
    let mut catalog: Value = serde_json::from_slice(include_bytes!("../config/codex-models.json"))?;
    let model = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .and_then(|models| models.first_mut())
        .and_then(Value::as_object_mut)
        .ok_or_else(|| std::io::Error::other("restricted model missing"))?;
    model.insert("multi_agent_version".to_owned(), json!("v2"));
    model.insert("use_responses_lite".to_owned(), json!(false));
    let catalog_path = codex_home.join("models.json");
    std::fs::write(&catalog_path, serde_json::to_vec(&catalog)?)?;
    let config = format!(
        r#"model = "gpt-5.6-sol"
model_provider = "capture"
model_catalog_json = {}
web_search = "disabled"

[orchestrator.skills]
enabled = false

[orchestrator.mcp]
enabled = false

[agents]
enabled = false

[tools.update_plan]
enabled = false

[tools.experimental_request_user_input]
enabled = false

[features]
multi_agent = false
multi_agent_v2 = false
apps = false
plugins = false
hooks = false
skill_search = false
tool_suggest = false

[model_providers.capture]
name = "capture"
base_url = "http://{address}/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
"#,
        serde_json::to_string(Path::new(&catalog_path))?
    );
    std::fs::write(codex_home.join("config.toml"), config)?;
    Ok(codex_home)
}

fn spawn_codex(
    codex_home: &Path,
) -> Result<(Child, ChildStdin, BufReader<ChildStdout>), Box<dyn std::error::Error>> {
    let mut child = Command::new("codex")
        .args([
            "app-server",
            "--strict-config",
            "--disable",
            "multi_agent_v2",
            "--listen",
            "stdio://",
        ])
        .env_clear()
        .env("HOME", codex_home)
        .env("CODEX_HOME", codex_home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("real App Server stdin missing"))?;
    let stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("real App Server stdout missing"))?,
    );
    Ok((child, stdin, stdout))
}

async fn begin_turn(
    stdin: &mut ChildStdin,
    stdout: &mut BufReader<ChildStdout>,
    codex_home: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    send(
        stdin,
        json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"tool-surface-test","title":"tool-surface-test","version":"1"},"capabilities":{"experimentalApi":true}}}),
    )
    .await?;
    let _initialized = response(stdout, 1).await?;
    send(stdin, json!({"method":"initialized","params":{}})).await?;
    send(
        stdin,
        json!({"method":"thread/start","id":2,"params":{
            "model":"gpt-5.6-sol",
            "modelProvider":"capture",
            "cwd":codex_home,
            "ephemeral":true,
            "approvalPolicy":"never",
            "sandbox":"read-only",
            "environments":[],
            "runtimeWorkspaceRoots":[],
            "dynamicTools":[{"type":"function","name":"weather","description":"Get weather","inputSchema":{"type":"object"}}]
        }}),
    )
    .await?;
    let thread = response(stdout, 2).await?;
    let thread_id = thread
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| std::io::Error::other("real App Server thread id missing"))?;
    send(
        stdin,
        json!({"method":"turn/start","id":3,"params":{
            "threadId":thread_id,
            "model":"gpt-5.6-sol",
            "input":[{"type":"text","text":"Use the weather tool."}]
        }}),
    )
    .await?;
    let _turn = response(stdout, 3).await?;
    Ok(())
}

#[tokio::test]
async fn real_codex_exposes_only_the_supplied_dynamic_tool()
-> Result<(), Box<dyn std::error::Error>> {
    let version = Command::new("codex").arg("--version").output().await?;
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout)?.trim(),
        "codex-cli 0.146.0"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (capture_tx, capture_rx) = oneshot::channel();
    let capture = Arc::new(Mutex::new(Some(capture_tx)));
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .fallback(post(capture_request))
                .with_state(capture),
        )
        .await
    });

    let codex_home = configure_codex_home(address)?;
    let (mut child, mut stdin, mut stdout) = spawn_codex(&codex_home)?;
    begin_turn(&mut stdin, &mut stdout, &codex_home).await?;

    let outbound = timeout(Duration::from_secs(10), capture_rx).await??;
    let tools = outbound
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| std::io::Error::other("Responses request has no tools"))?;
    assert_eq!(tools.len(), 1, "unexpected model-visible tools: {tools:?}");
    assert_eq!(
        tools
            .first()
            .and_then(|tool| tool.get("name"))
            .and_then(Value::as_str),
        Some("weather")
    );

    child.kill().await?;
    let _status = child.wait().await?;
    server.abort();
    std::fs::remove_dir_all(&codex_home)?;
    Ok(())
}
