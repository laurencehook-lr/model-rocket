use std::{
    env, fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::mpsc,
    time::timeout,
};

use crate::{anthropic::Tool, error::BridgeError};

const RPC_TIMEOUT: Duration = Duration::from_secs(20);
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_APP_SERVER_FRAME_BYTES: usize = 8 * 1024 * 1024;
const REQUIRED_CODEX_VERSION: &str = "codex-cli 0.146.0";
const ISOLATED_CONFIG: &str = r#"check_for_update_on_startup = false
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
apps = false
browser_use = false
code_mode = false
code_mode_only = false
computer_use = false
current_time_reminder = false
default_mode_request_user_input = false
deferred_executor = false
hooks = false
image_generation = false
multi_agent = false
multi_agent_v2 = false
plugins = false
skill_search = false
standalone_web_search = false
token_budget = false
tool_suggest = false
unified_exec = false
"#;

#[derive(Debug, Serialize)]
pub struct PreflightReport {
    pub account_type: &'static str,
    pub model: String,
}

#[derive(Debug)]
pub enum TurnOutcome {
    Text {
        usage: TokenUsage,
        limit_reached: bool,
    },
    ToolCall(PendingToolCall),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub struct StartTurnRequest<'a> {
    pub model: &'a str,
    pub cwd: &'a Path,
    pub tools: &'a [Tool],
    pub prompt: &'a str,
    pub developer_instructions: &'a str,
    pub max_tokens: u32,
    pub effort: Option<&'a str>,
    pub output_schema: Option<&'a Value>,
}

struct OutputLimiter {
    max_bytes: usize,
    emitted_bytes: usize,
    reached: bool,
}

#[derive(Debug)]
pub struct PendingToolCall {
    pub rpc_id: Value,
    pub call_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub tool: String,
    pub arguments: Value,
    pub usage: Option<TokenUsage>,
}

pub struct AppServer {
    _child: Child,
    _isolated_home: IsolatedCodexHome,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

struct IsolatedCodexHome {
    path: PathBuf,
}

impl IsolatedCodexHome {
    fn create() -> Result<Self, BridgeError> {
        let source_home = env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
            .ok_or_else(|| BridgeError::configuration("Codex authentication home is unknown"))?;
        let source_auth = source_home.join("auth.json");
        if !source_auth.is_file() {
            return Err(BridgeError::configuration(format!(
                "managed ChatGPT authentication is missing at {}",
                source_auth.display()
            )));
        }
        let path = env::temp_dir().join(format!(
            "model-rocket-codex-home-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        fs::create_dir(&path).map_err(|error| {
            BridgeError::configuration(format!("cannot create isolated Codex home: {error}"))
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            BridgeError::configuration(format!("cannot secure isolated Codex home: {error}"))
        })?;
        if let Err(error) = symlink(&source_auth, path.join("auth.json")) {
            let _cleanup = fs::remove_dir(&path);
            return Err(BridgeError::configuration(format!(
                "cannot expose managed ChatGPT authentication to isolated Codex: {error}"
            )));
        }
        if let Err(error) = fs::write(path.join("config.toml"), ISOLATED_CONFIG) {
            let _cleanup = fs::remove_dir_all(&path);
            return Err(BridgeError::configuration(format!(
                "cannot write isolated Codex configuration: {error}"
            )));
        }
        Ok(Self { path })
    }
}

impl Drop for IsolatedCodexHome {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            tracing::warn!(path = %self.path.display(), %error, "cannot remove isolated Codex home");
        }
    }
}

impl AppServer {
    /// Starts and initializes the official Codex App Server process.
    ///
    /// # Errors
    ///
    /// Returns an error when the process or initialization handshake fails.
    pub async fn launch(codex_bin: &Path) -> Result<Self, BridgeError> {
        verify_codex_version(codex_bin).await?;
        let isolated_home = IsolatedCodexHome::create()?;
        let model_catalog = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/codex-models.json");
        if !model_catalog.is_file() {
            return Err(BridgeError::configuration(format!(
                "restricted Codex model catalog is missing at {}",
                model_catalog.display()
            )));
        }
        let model_catalog_override = format!(
            "model_catalog_json={}",
            serde_json::to_string(&model_catalog).map_err(|error| {
                BridgeError::configuration(format!("cannot encode model catalog path: {error}"))
            })?
        );
        let mut command = Command::new(codex_bin);
        command
            .args([
                "app-server",
                "--strict-config",
                "--disable",
                "multi_agent_v2",
                "-c",
                &model_catalog_override,
                "-c",
                "agents.enabled=false",
                "-c",
                "web_search=\"disabled\"",
                "--listen",
                "stdio://",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let allowed_environment = ["USER", "PATH", "TMPDIR", "LANG", "LC_ALL", "SHELL"]
            .into_iter()
            .filter_map(|name| env::var_os(name).map(|value| (name, value)))
            .collect::<Vec<_>>();
        command.env_clear();
        command.env("HOME", &isolated_home.path);
        command.env("CODEX_HOME", &isolated_home.path);
        for (name, value) in allowed_environment {
            command.env(name, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| BridgeError::unavailable(format!("cannot start codex: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BridgeError::unavailable("codex stdin was not created"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::unavailable("codex stdout was not created"))?;

        let mut server = Self {
            _child: child,
            _isolated_home: isolated_home,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        server.initialize().await?;
        Ok(server)
    }

    /// Verifies managed `ChatGPT` authentication and exact model availability.
    ///
    /// # Errors
    ///
    /// Returns an error for non-ChatGPT auth, a missing model, or malformed protocol data.
    pub async fn preflight(&mut self, model: &str) -> Result<PreflightReport, BridgeError> {
        let account = self
            .request("account/read", json!({"refreshToken": false}))
            .await?;
        let account_type = account
            .get("account")
            .and_then(|value| value.as_object())
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::protocol("account/read returned no account type"))?;
        if account_type != "chatgpt" {
            return Err(BridgeError::unavailable(format!(
                "managed ChatGPT authentication is required, found {account_type}"
            )));
        }

        let mut cursor: Option<String> = None;
        let mut found = false;
        loop {
            let mut params = json!({"includeHidden": true, "limit": 100});
            if let Some(value) = &cursor {
                params
                    .as_object_mut()
                    .ok_or_else(|| BridgeError::protocol("model/list params are not an object"))?
                    .insert("cursor".to_owned(), Value::String(value.clone()));
            }
            let result = self.request("model/list", params).await?;
            let models = result
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| BridgeError::protocol("model/list returned no data array"))?;
            found |= models.iter().any(|entry| {
                entry.get("id").and_then(Value::as_str) == Some(model)
                    && entry.get("model").and_then(Value::as_str) == Some(model)
            });
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        if !found {
            return Err(BridgeError::unavailable(format!(
                "model {model} is not available to the managed ChatGPT account"
            )));
        }

        Ok(PreflightReport {
            account_type: "chatgpt",
            model: model.to_owned(),
        })
    }

    /// Starts one ephemeral App Server thread and its first turn.
    ///
    /// # Errors
    ///
    /// Returns an error when translation, thread startup, turn startup, or generation fails.
    pub async fn start_turn(
        &mut self,
        request: StartTurnRequest<'_>,
        deltas: &mpsc::Sender<String>,
    ) -> Result<TurnOutcome, BridgeError> {
        let cwd = request
            .cwd
            .to_str()
            .ok_or_else(|| BridgeError::configuration("MODEL_ROCKET_CWD must be valid UTF-8"))?;
        let dynamic_tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        let thread = self
            .request(
                "thread/start",
                json!({
                    "model": request.model,
                    "cwd": cwd,
                    "ephemeral": true,
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                    "environments": [],
                    "runtimeWorkspaceRoots": [],
                    "dynamicTools": dynamic_tools,
                    "baseInstructions": "Act as the model inside Claude Code. Respond with assistant text or the supplied dynamic tools only. Never invoke Codex built-in shell, file, web, MCP, collaboration, or user-input tools.",
                    "developerInstructions": request.developer_instructions,
                }),
            )
            .await?;
        let thread_id = thread
            .get("thread")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::protocol("thread/start returned no thread id"))?
            .to_owned();
        let mut turn_params = json!({
            "threadId": thread_id,
            "model": request.model,
            "input": [{"type": "text", "text": request.prompt}],
        });
        if let Some(value) = request.effort {
            turn_params
                .as_object_mut()
                .ok_or_else(|| BridgeError::protocol("turn/start params are not an object"))?
                .insert("effort".to_owned(), Value::String(value.to_owned()));
        }
        if let Some(schema) = request.output_schema {
            turn_params
                .as_object_mut()
                .ok_or_else(|| BridgeError::protocol("turn/start params are not an object"))?
                .insert("outputSchema".to_owned(), schema.clone());
        }
        let turn = self.request("turn/start", turn_params).await?;
        let turn_id = turn
            .get("turn")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::protocol("turn/start returned no turn id"))?
            .to_owned();
        self.read_turn(&thread_id, &turn_id, request.max_tokens, deltas)
            .await
    }

    /// Resolves a pending dynamic tool request and reads the continuing turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the response cannot be sent or the continuing turn fails.
    pub async fn continue_tool(
        &mut self,
        pending: PendingToolCall,
        text: &str,
        success: bool,
        max_tokens: u32,
        deltas: &mpsc::Sender<String>,
    ) -> Result<TurnOutcome, BridgeError> {
        let expected_thread_id = pending.thread_id.clone();
        let expected_turn_id = pending.turn_id.clone();
        self.send(json!({
            "id": pending.rpc_id,
            "result": {
                "contentItems": [{"type": "inputText", "text": text}],
                "success": success,
            }
        }))
        .await?;
        self.read_turn(&expected_thread_id, &expected_turn_id, max_tokens, deltas)
            .await
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "model_rocket",
                    "title": "Model Rocket",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {"experimentalApi": true},
            }),
        )
        .await?;
        self.send(json!({"method": "initialized", "params": {}}))
            .await
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| BridgeError::protocol("JSON-RPC request id exhausted"))?;
        self.send(json!({"method": method, "id": id, "params": params}))
            .await?;

        loop {
            let message = self.read_message(RPC_TIMEOUT).await?;
            if message.id.as_ref().and_then(Value::as_u64) != Some(id) {
                if message.method.is_some() {
                    continue;
                }
                return Err(BridgeError::protocol(format!(
                    "received response for unexpected request id while waiting for {method}"
                )));
            }
            if let Some(error) = message.error {
                return Err(BridgeError::protocol(format!(
                    "{method} failed with {}: {}",
                    error.code, error.message
                )));
            }
            return message
                .result
                .ok_or_else(|| BridgeError::protocol(format!("{method} returned no result")));
        }
    }

    async fn read_turn(
        &mut self,
        expected_thread_id: &str,
        expected_turn_id: &str,
        max_tokens: u32,
        deltas: &mpsc::Sender<String>,
    ) -> Result<TurnOutcome, BridgeError> {
        let expected = (expected_thread_id, expected_turn_id);
        let mut usage: Option<TokenUsage> = None;
        let mut limiter = OutputLimiter {
            max_bytes: usize::try_from(max_tokens).map_err(|_| {
                BridgeError::invalid_request("max_tokens does not fit this platform")
            })?,
            emitted_bytes: 0,
            reached: false,
        };
        let mut upstream_error: Option<String> = None;
        loop {
            let message = self.read_message(TURN_TIMEOUT).await?;
            match message.method.as_deref() {
                Some("item/agentMessage/delta") => {
                    if let Some(delta) = message
                        .params
                        .as_ref()
                        .and_then(|params| params.get("delta"))
                        .and_then(Value::as_str)
                    {
                        self.emit_delta(delta, expected, &mut limiter, deltas)
                            .await?;
                    }
                }
                Some("thread/tokenUsage/updated") => {
                    usage = Some(token_usage(&message, Some(expected))?);
                }
                Some("item/tool/call") => {
                    if !limiter.reached {
                        return parse_tool_call(message, expected, usage)
                            .map(TurnOutcome::ToolCall);
                    }
                }
                Some("item/started" | "item/completed") => {
                    validate_item_lifecycle(&message)?;
                }
                Some("error") => {
                    upstream_error = app_server_error(&message);
                }
                Some("turn/completed") => {
                    let status = turn_status(&message)?;
                    if status != "completed" && !limiter.reached {
                        return Err(BridgeError::unavailable(
                            upstream_error
                                .unwrap_or_else(|| format!("turn completed with status {status}")),
                        ));
                    }
                    let exact_usage = usage.ok_or_else(|| {
                        BridgeError::protocol("turn completed without final token usage")
                    })?;
                    return Ok(TurnOutcome::Text {
                        usage: exact_usage,
                        limit_reached: limiter.reached,
                    });
                }
                Some(method) if message.id.is_some() => {
                    return Err(BridgeError::unsupported(format!(
                        "App Server requested unsupported client method {method}"
                    )));
                }
                Some(_) | None => {}
            }
        }
    }

    async fn emit_delta(
        &mut self,
        delta: &str,
        expected: (&str, &str),
        limiter: &mut OutputLimiter,
        deltas: &mpsc::Sender<String>,
    ) -> Result<(), BridgeError> {
        if limiter.reached {
            return Ok(());
        }
        let remaining = limiter.max_bytes.saturating_sub(limiter.emitted_bytes);
        let mut end = remaining.min(delta.len());
        while !delta.is_char_boundary(end) {
            end = end
                .checked_sub(1)
                .ok_or_else(|| BridgeError::protocol("cannot find UTF-8 output boundary"))?;
        }
        if let Some(prefix) = delta.get(..end)
            && !prefix.is_empty()
        {
            deltas.send(prefix.to_owned()).await.map_err(|_| {
                BridgeError::unavailable("Claude Code disconnected during generation")
            })?;
            limiter.emitted_bytes = limiter
                .emitted_bytes
                .checked_add(prefix.len())
                .ok_or_else(|| BridgeError::protocol("streamed output byte count overflowed"))?;
        }
        if limiter.emitted_bytes == limiter.max_bytes || end < delta.len() {
            self.send_without_wait(
                "turn/interrupt",
                json!({"threadId": expected.0, "turnId": expected.1}),
            )
            .await?;
            limiter.reached = true;
        }
        Ok(())
    }

    async fn send(&mut self, value: Value) -> Result<(), BridgeError> {
        let mut bytes = serde_json::to_vec(&value)
            .map_err(|error| BridgeError::protocol(format!("cannot encode JSON-RPC: {error}")))?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|error| BridgeError::unavailable(format!("cannot write to codex: {error}")))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| BridgeError::unavailable(format!("cannot flush codex stdin: {error}")))
    }

    async fn send_without_wait(&mut self, method: &str, params: Value) -> Result<(), BridgeError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| BridgeError::protocol("JSON-RPC request id exhausted"))?;
        self.send(json!({"method": method, "id": id, "params": params}))
            .await
    }

    async fn read_message(&mut self, deadline: Duration) -> Result<RpcMessage, BridgeError> {
        let line = timeout(deadline, self.read_capped_line())
            .await
            .map_err(|_| BridgeError::unavailable("timed out waiting for Codex App Server"))??;
        serde_json::from_str::<RpcMessage>(&line)
            .map_err(|error| BridgeError::protocol(format!("invalid JSON-RPC from codex: {error}")))
    }

    async fn read_capped_line(&mut self) -> Result<String, BridgeError> {
        let mut frame = Vec::new();
        loop {
            let available = self.stdout.fill_buf().await.map_err(|error| {
                BridgeError::unavailable(format!("cannot read codex stdout: {error}"))
            })?;
            if available.is_empty() {
                return Err(BridgeError::unavailable("Codex App Server closed stdout"));
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.unwrap_or(available.len());
            let next_len = frame
                .len()
                .checked_add(take)
                .ok_or_else(|| BridgeError::protocol("App Server frame length overflowed"))?;
            if next_len > MAX_APP_SERVER_FRAME_BYTES {
                return Err(BridgeError::protocol(format!(
                    "App Server frame exceeds {MAX_APP_SERVER_FRAME_BYTES} bytes"
                )));
            }
            frame.extend_from_slice(
                available
                    .get(..take)
                    .ok_or_else(|| BridgeError::protocol("App Server frame boundary is invalid"))?,
            );
            self.stdout.consume(take + usize::from(newline.is_some()));
            if newline.is_some() {
                return String::from_utf8(frame).map_err(|error| {
                    BridgeError::protocol(format!("App Server frame is not UTF-8: {error}"))
                });
            }
        }
    }
}

async fn verify_codex_version(codex_bin: &Path) -> Result<(), BridgeError> {
    let output = Command::new(codex_bin)
        .arg("--version")
        .env_clear()
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .output()
        .await
        .map_err(|error| BridgeError::configuration(format!("cannot inspect codex: {error}")))?;
    let version = String::from_utf8(output.stdout).map_err(|error| {
        BridgeError::configuration(format!("codex version is not UTF-8: {error}"))
    })?;
    if !output.status.success() || version.trim() != REQUIRED_CODEX_VERSION {
        return Err(BridgeError::configuration(format!(
            "Codex version must be exactly {REQUIRED_CODEX_VERSION}, found {}",
            version.trim()
        )));
    }
    Ok(())
}

fn validate_item_lifecycle(message: &RpcMessage) -> Result<(), BridgeError> {
    let item_type = message
        .params
        .as_ref()
        .and_then(|params| params.get("item"))
        .and_then(|item| item.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("item lifecycle event has no item type"))?;
    if matches!(
        item_type,
        "agentMessage" | "reasoning" | "userMessage" | "dynamicToolCall"
    ) {
        return Ok(());
    }
    Err(BridgeError::protocol(format!(
        "Codex attempted forbidden built-in item type {item_type}"
    )))
}

#[derive(Debug, Deserialize)]
struct RpcMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
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

fn parse_tool_call(
    message: RpcMessage,
    expected: (&str, &str),
    usage: Option<TokenUsage>,
) -> Result<PendingToolCall, BridgeError> {
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
    Ok(PendingToolCall {
        rpc_id,
        call_id: call.call_id,
        thread_id: call.thread_id,
        turn_id: call.turn_id,
        tool: call.tool,
        arguments: call.arguments,
        usage,
    })
}

fn app_server_error(message: &RpcMessage) -> Option<String> {
    message
        .params
        .as_ref()
        .and_then(|params| params.get("error"))
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn turn_status(message: &RpcMessage) -> Result<&str, BridgeError> {
    message
        .params
        .as_ref()
        .and_then(|params| params.get("turn"))
        .and_then(|turn| turn.get("status"))
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("turn/completed has no status"))
}

fn token_usage(
    message: &RpcMessage,
    expected: Option<(&str, &str)>,
) -> Result<TokenUsage, BridgeError> {
    let params = message
        .params
        .as_ref()
        .ok_or_else(|| BridgeError::protocol("token usage update has no params"))?;
    let thread_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("token usage update has no thread id"))?;
    let turn_id = params
        .get("turnId")
        .and_then(Value::as_str)
        .ok_or_else(|| BridgeError::protocol("token usage update has no turn id"))?;
    if let Some((expected_thread_id, expected_turn_id)) = expected
        && (thread_id != expected_thread_id || turn_id != expected_turn_id)
    {
        return Err(BridgeError::protocol(
            "token usage update does not match the active turn",
        ));
    }
    let last = params
        .get("tokenUsage")
        .and_then(|value| value.get("last"))
        .ok_or_else(|| BridgeError::protocol("token usage update has no last usage"))?;
    let input_tokens = last
        .get("inputTokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| BridgeError::protocol("token usage update has no input token count"))?;
    let output_tokens = last
        .get("outputTokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| BridgeError::protocol("token usage update has no output token count"))?;
    Ok(TokenUsage {
        input_tokens,
        output_tokens,
    })
}
