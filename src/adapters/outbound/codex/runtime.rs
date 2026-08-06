use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    env, fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use serde_json::Value;
use tiktoken_rs::CoreBPE;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{Mutex, RwLock, mpsc, oneshot},
    time::timeout,
};

use crate::{
    contracts::codex::{
        self, diagnostics as d,
        messages::{
            self as m, JsonRpcRequestId, RpcMessage, TurnStatus, agent_message_delta,
            app_server_error, parse_tool_call, token_usage, turn_status, validate_item_lifecycle,
        },
        tool_names::DynamicToolNames,
    },
    domain::BridgeError,
    domain::{
        AccountMode, AssistantOutcome, AssistantTextDelta, CompletionCause, ContinueModelTurn,
        PreflightReport, StartModelTurn, TokenUsage, ValidatedCodexExecutable,
    },
    policies::codex_limits::{
        CONNECTION_ALIVE_AT_START, CONNECTION_FAILED_STATE, INITIAL_RPC_REQUEST_ID,
        ISOLATED_HOME_MODE, MAX_APP_SERVER_FRAME_BYTES, MAX_MODEL_LIST_PAGES,
        MODEL_LIST_PAGE_DECREMENT, MODEL_LIST_PAGE_SIZE, RPC_REQUEST_ID_INCREMENT, RPC_TIMEOUT,
        TERMINATE_CHILD_ON_DROP, TOKEN_BOUNDARY_BACKOFF, TURN_TIMEOUT,
    },
    ports::ModelOutput,
    product::{CODEX_CLI_VERSION_OUTPUT, TRUSTED_CHILD_PATH},
};

static OUTPUT_TOKENIZER: OnceLock<Result<CoreBPE, String>> = OnceLock::new();

struct OutputLimiter {
    max_tokens: usize,
    emitted_tokens: usize,
    pending_text: String,
    state: OutputLimitState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputLimitState {
    Available,
    Reached,
}

impl OutputLimitState {
    fn is_reached(self) -> bool {
        self == Self::Reached
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UsageEpoch {
    Closed,
    Open,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TurnReadPhase {
    Initial,
    ToolContinuation,
}

struct AppServerContinuation {
    pub rpc: JsonRpcRequestId,
    pub thread: String,
    pub turn: String,
}

struct TurnProgress {
    usage: Option<TokenUsage>,
    usage_epoch: UsageEpoch,
    limiter: OutputLimiter,
    latest_upstream_error: Option<String>,
    fatal_upstream_error: Option<String>,
}

impl TurnProgress {
    fn new(max_tokens: u32, phase: TurnReadPhase) -> Result<Self, BridgeError> {
        Ok(Self {
            usage: None,
            usage_epoch: if phase == TurnReadPhase::Initial {
                UsageEpoch::Open
            } else {
                UsageEpoch::Closed
            },
            limiter: OutputLimiter {
                max_tokens: usize::try_from(max_tokens)
                    .map_err(|_| BridgeError::invalid_request(d::OUTPUT_LIMIT_INVALID))?,
                emitted_tokens: usize::default(),
                pending_text: String::new(),
                state: OutputLimitState::Available,
            },
            latest_upstream_error: None,
            fatal_upstream_error: None,
        })
    }
}

pub(super) struct CodexRuntime {
    connection: Arc<AppServerConnection>,
    dynamic_tool_names: DynamicToolNames,
    output_tokenizer: &'static CoreBPE,
    pending_tool: Option<AppServerContinuation>,
    thread_id: Option<String>,
    thread_events: Option<mpsc::UnboundedReceiver<Result<RpcMessage, BridgeError>>>,
}

#[derive(Clone)]
pub(super) struct CodexRuntimeFactory {
    executable: ValidatedCodexExecutable,
    catalogue: Arc<crate::domain::ModelCatalogue>,
    connection: Arc<Mutex<Option<Arc<AppServerConnection>>>>,
}

struct AppServerConnection {
    child: Mutex<Child>,
    _isolated_home: IsolatedCodexHome,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending_requests: Mutex<HashMap<u64, oneshot::Sender<Result<RpcMessage, BridgeError>>>>,
    thread_senders: RwLock<HashMap<String, mpsc::UnboundedSender<Result<RpcMessage, BridgeError>>>>,
    alive: AtomicBool,
    failure: OnceLock<BridgeError>,
    provider_override: Option<String>,
}

impl CodexRuntimeFactory {
    #[must_use]
    pub(super) fn new(
        executable: ValidatedCodexExecutable,
        catalogue: Arc<crate::domain::ModelCatalogue>,
    ) -> Self {
        Self {
            executable,
            catalogue,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    async fn shared_connection(&self) -> Result<Arc<AppServerConnection>, BridgeError> {
        let mut connection = self.connection.lock().await;
        if let Some(active) = connection.as_ref().filter(|active| active.is_alive()) {
            return Ok(Arc::clone(active));
        }
        let replacement =
            AppServerConnection::launch(&self.executable, None, Some(Arc::clone(&self.catalogue)))
                .await?;
        *connection = Some(Arc::clone(&replacement));
        Ok(replacement)
    }

    /// Opens one logical model session on the shared App Server process.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot be launched or initialized.
    pub(super) async fn launch_session(&self) -> Result<CodexRuntime, BridgeError> {
        let connection = self.shared_connection().await?;
        CodexRuntime::from_connection(connection)
    }
}

struct IsolatedCodexHome {
    path: PathBuf,
}

impl IsolatedCodexHome {
    fn create(catalogue: Option<&crate::domain::ModelCatalogue>) -> Result<Self, BridgeError> {
        let source_home = env::var_os(d::CODEX_HOME_ENV)
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os(d::HOME_ENV)
                    .map(|home| PathBuf::from(home).join(d::CODEX_HOME_DIRECTORY))
            })
            .ok_or_else(|| BridgeError::configuration(d::CODEX_HOME_UNKNOWN))?;
        let source_auth = source_home.join(d::AUTH_FILE);
        if !source_auth.is_file() {
            return Err(d::missing_auth(&source_auth));
        }
        let path = env::temp_dir().join(d::isolated_home_name(
            std::process::id(),
            uuid::Uuid::now_v7(),
        ));
        fs::create_dir(&path).map_err(|error| d::configuration(d::CREATE_HOME_CONTEXT, error))?;
        let isolated_home = Self { path };
        fs::set_permissions(
            &isolated_home.path,
            fs::Permissions::from_mode(ISOLATED_HOME_MODE),
        )
        .map_err(|error| d::configuration(d::SECURE_HOME_CONTEXT, error))?;
        if let Err(error) = symlink(&source_auth, isolated_home.path.join(d::AUTH_FILE)) {
            return Err(d::configuration(d::EXPOSE_AUTH_CONTEXT, error));
        }
        if let Err(error) = fs::write(
            isolated_home.path.join(d::CONFIG_FILE),
            codex::ISOLATED_CONFIG,
        ) {
            return Err(d::configuration(d::WRITE_CONFIG_CONTEXT, error));
        }
        if let Some(catalogue) = catalogue {
            let model_catalog = m::restricted_model_catalog(catalogue)?;
            let encoded_catalog = serde_json::to_vec(&model_catalog)
                .map_err(|error| d::configuration(d::ENCODE_CATALOG_CONTEXT, error))?;
            if let Err(error) = fs::write(
                isolated_home.path.join(d::MODEL_CATALOG_FILE),
                encoded_catalog,
            ) {
                return Err(d::configuration(d::WRITE_CATALOG_CONTEXT, error));
            }
        }
        Ok(isolated_home)
    }
}

impl Drop for IsolatedCodexHome {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            d::warn_home_cleanup(&self.path, error);
        }
    }
}

impl CodexRuntime {
    /// Starts and initializes the official Codex App Server process.
    ///
    /// # Errors
    ///
    /// Returns an error when the process or initialization handshake fails.
    pub(super) async fn launch(
        executable: &ValidatedCodexExecutable,
        provider_endpoint: Option<&reqwest::Url>,
        catalogue: Arc<crate::domain::ModelCatalogue>,
    ) -> Result<Self, BridgeError> {
        if let Some(endpoint) = provider_endpoint {
            let host = endpoint
                .host_str()
                .and_then(|value| value.parse::<std::net::IpAddr>().ok())
                .ok_or_else(|| BridgeError::configuration(d::CAPTURE_HOST_INVALID))?;
            if endpoint.scheme() != d::HTTP_SCHEME || !host.is_loopback() {
                return Err(BridgeError::configuration(d::CAPTURE_ENDPOINT_INVALID));
            }
        }
        let connection =
            AppServerConnection::launch(executable, provider_endpoint, Some(catalogue)).await?;
        Self::from_connection(connection)
    }

    pub(super) async fn launch_discovery(
        executable: &ValidatedCodexExecutable,
    ) -> Result<Self, BridgeError> {
        let connection = AppServerConnection::launch(executable, None, None).await?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Arc<AppServerConnection>) -> Result<Self, BridgeError> {
        Ok(Self {
            connection,
            dynamic_tool_names: DynamicToolNames::default(),
            output_tokenizer: output_tokenizer()?,
            pending_tool: None,
            thread_id: None,
            thread_events: None,
        })
    }

    /// Verifies managed `ChatGPT` authentication and exact model availability.
    ///
    /// # Errors
    ///
    /// Returns an error for non-ChatGPT auth, a missing model, or malformed protocol data.
    pub(super) async fn preflight(
        &mut self,
        catalogue: &crate::domain::ModelCatalogue,
    ) -> Result<Vec<PreflightReport>, BridgeError> {
        let account = self
            .connection
            .request(m::ACCOUNT_READ_METHOD, m::account_read_params())
            .await?;
        let account_type = m::account_type(&account)?;
        if !m::is_managed_chatgpt(account_type) {
            return Err(d::wrong_account(account_type));
        }

        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut available_models = HashSet::new();
        let mut remaining_pages = MAX_MODEL_LIST_PAGES;
        loop {
            if remaining_pages == usize::default() {
                return Err(d::model_list_page_limit());
            }
            remaining_pages = remaining_pages
                .checked_sub(MODEL_LIST_PAGE_DECREMENT)
                .ok_or_else(d::model_list_page_limit)?;
            let params = m::model_list_params(cursor.as_deref(), MODEL_LIST_PAGE_SIZE);
            let result = self
                .connection
                .request(m::MODEL_LIST_METHOD, params)
                .await?;
            let (page_models, next_cursor) = m::model_page(&result)?;
            available_models.extend(page_models);
            let Some(next_cursor) = next_cursor else {
                break;
            };
            if !seen_cursors.insert(next_cursor.clone()) {
                return Err(d::repeated_model_list_cursor(&next_cursor));
            }
            cursor = Some(next_cursor);
        }

        let mut reports = Vec::with_capacity(catalogue.models().len());
        for model in catalogue.models() {
            if !available_models.contains(model.id()) {
                return Err(d::unavailable_model(model.id().as_str()));
            }
            reports.push(PreflightReport::new(
                AccountMode::ManagedChatGpt,
                model.id().clone(),
            ));
        }
        Ok(reports)
    }
}

impl AppServerConnection {
    async fn launch(
        executable: &ValidatedCodexExecutable,
        provider_endpoint: Option<&reqwest::Url>,
        catalogue: Option<Arc<crate::domain::ModelCatalogue>>,
    ) -> Result<Arc<Self>, BridgeError> {
        let codex_bin = codex::executable::revalidate(executable)?;
        verify_codex_version(&codex_bin).await?;
        let isolated_home = IsolatedCodexHome::create(catalogue.as_deref())?;
        let mut command = Command::new(&codex_bin);
        command
            .args(d::PROCESS_ARGUMENTS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(TERMINATE_CHILD_ON_DROP);
        if catalogue.is_some() {
            let model_catalog = isolated_home.path.join(d::MODEL_CATALOG_FILE);
            let model_catalog_override = d::model_catalog_override(&model_catalog)?;
            command.args([d::CONFIG_FLAG, &model_catalog_override]);
        }
        command.args(d::PROCESS_ARGUMENTS_AFTER_CATALOGUE);
        let provider_override = if let Some(endpoint) = provider_endpoint {
            let endpoint_override = d::capture_endpoint_override(endpoint.as_str())?;
            command.args([
                d::CONFIG_FLAG,
                d::CAPTURE_NAME_CONFIG,
                d::CONFIG_FLAG,
                &endpoint_override,
                d::CONFIG_FLAG,
                d::CAPTURE_WIRE_CONFIG,
                d::CONFIG_FLAG,
                d::CAPTURE_AUTH_CONFIG,
                d::CONFIG_FLAG,
                d::CAPTURE_WEBSOCKET_CONFIG,
            ]);
            Some(d::CAPTURE_PROVIDER.to_owned())
        } else {
            None
        };
        let allowed_environment = d::ALLOWED_ENVIRONMENT
            .into_iter()
            .filter_map(|name| env::var_os(name).map(|value| (name, value)))
            .collect::<Vec<_>>();
        command.env_clear();
        command.env(d::HOME_ENV, &isolated_home.path);
        command.env(d::CODEX_HOME_ENV, &isolated_home.path);
        command.env(d::PATH_ENV, TRUSTED_CHILD_PATH);
        for (name, value) in allowed_environment {
            command.env(name, value);
        }

        std::mem::drop(codex::executable::revalidate(executable)?);
        let mut child = command
            .spawn()
            .map_err(|error| d::unavailable(d::START_CODEX_CONTEXT, error))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BridgeError::unavailable(d::CODEX_STDIN_MISSING))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::unavailable(d::CODEX_STDOUT_MISSING))?;

        let connection = Arc::new(Self {
            child: Mutex::new(child),
            _isolated_home: isolated_home,
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(INITIAL_RPC_REQUEST_ID),
            pending_requests: Mutex::new(HashMap::new()),
            thread_senders: RwLock::new(HashMap::new()),
            alive: AtomicBool::new(CONNECTION_ALIVE_AT_START),
            failure: OnceLock::new(),
            provider_override,
        });
        let weak = Arc::downgrade(&connection);
        tokio::spawn(async move {
            Self::read_loop(BufReader::new(stdout), weak).await;
        });
        connection.initialize().await?;
        Ok(connection)
    }
}

impl CodexRuntime {
    /// Starts one ephemeral App Server thread and its first turn.
    ///
    /// # Errors
    ///
    /// Returns an error when translation, thread startup, turn startup, or generation fails.
    pub(super) async fn start_turn(
        &mut self,
        request: StartModelTurn,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        let cwd = request.working_directory().as_str();
        self.dynamic_tool_names = DynamicToolNames::from_claude_tools(request.tools())?;
        let thread_params = m::thread_start_params(
            &request,
            cwd,
            &self.dynamic_tool_names,
            self.connection.provider_override.as_deref(),
        );
        let thread = self
            .connection
            .request(m::THREAD_START_METHOD, thread_params)
            .await?;
        let thread_id = m::started_thread_id(&thread)?;
        let events = self.connection.register_thread(&thread_id).await?;
        self.thread_id = Some(thread_id.clone());
        self.thread_events = Some(events);
        let turn_params = m::turn_start_params(&request, &thread_id)?;
        let turn = match self
            .connection
            .request(m::TURN_START_METHOD, turn_params)
            .await
        {
            Ok(turn) => turn,
            Err(error) => {
                self.release_thread().await;
                return Err(error);
            }
        };
        let turn_id = m::started_turn_id(&turn)?;
        self.read_turn(
            &thread_id,
            &turn_id,
            request.output_limit().get(),
            output,
            TurnReadPhase::Initial,
        )
        .await
    }

    /// Resolves a pending dynamic tool request and reads the continuing turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the response cannot be sent or the continuing turn fails.
    pub(super) async fn continue_tool(
        &mut self,
        resolution: ContinueModelTurn,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        let pending = self
            .pending_tool
            .take()
            .ok_or_else(|| BridgeError::protocol(d::TOOL_CONTINUATION_MISSING))?;
        let expected_thread_id = pending.thread.clone();
        let expected_turn_id = pending.turn.clone();
        self.connection
            .send(m::tool_resolution(&pending.rpc, &resolution))
            .await?;
        self.read_turn(
            &expected_thread_id,
            &expected_turn_id,
            resolution.output_limit().get(),
            output,
            TurnReadPhase::ToolContinuation,
        )
        .await
    }

    async fn read_turn(
        &mut self,
        expected_thread_id: &str,
        expected_turn_id: &str,
        max_tokens: u32,
        output: &dyn ModelOutput,
        phase: TurnReadPhase,
    ) -> Result<AssistantOutcome, BridgeError> {
        let outcome = self
            .read_turn_inner(
                expected_thread_id,
                expected_turn_id,
                max_tokens,
                output,
                phase,
            )
            .await;
        match &outcome {
            Ok(AssistantOutcome::ToolCall(_)) => {}
            _ => self.release_thread().await,
        }
        outcome
    }

    async fn read_turn_inner(
        &mut self,
        expected_thread_id: &str,
        expected_turn_id: &str,
        max_tokens: u32,
        output: &dyn ModelOutput,
        phase: TurnReadPhase,
    ) -> Result<AssistantOutcome, BridgeError> {
        let expected = (expected_thread_id, expected_turn_id);
        let mut progress = TurnProgress::new(max_tokens, phase)?;
        loop {
            let message = self.next_turn_event().await?;
            match message.method.as_deref() {
                Some(m::EVENT_AGENT_DELTA) => {
                    let delta = agent_message_delta(&message, expected)?;
                    progress.usage = None;
                    progress.usage_epoch = UsageEpoch::Open;
                    self.emit_delta(delta, expected, &mut progress.limiter, output)
                        .await?;
                }
                Some(m::EVENT_TOKEN_USAGE) => {
                    let reported_usage = token_usage(&message, expected)?;
                    if progress.usage_epoch == UsageEpoch::Open {
                        progress.usage = Some(reported_usage);
                    }
                }
                Some(m::EVENT_TOOL_CALL) => {
                    let parsed_tool = parse_tool_call(message, expected, &self.dynamic_tool_names)?;
                    progress.usage = None;
                    progress.usage_epoch = UsageEpoch::Open;
                    if !progress.limiter.state.is_reached() {
                        self.resolve_pending(&mut progress.limiter, output).await?;
                        if progress.limiter.state.is_reached() {
                            self.interrupt_turn(expected).await?;
                            continue;
                        }
                        self.pending_tool = Some(AppServerContinuation {
                            rpc: parsed_tool.rpc_id,
                            thread: parsed_tool.thread_id,
                            turn: parsed_tool.turn_id,
                        });
                        return Ok(AssistantOutcome::ToolCall(parsed_tool.tool_call));
                    }
                }
                Some(m::EVENT_ITEM_STARTED) => {
                    validate_item_lifecycle(&message, expected, m::STARTED_TIMESTAMP_FIELD)?;
                }
                Some(m::EVENT_ITEM_COMPLETED) => {
                    validate_item_lifecycle(&message, expected, m::COMPLETED_TIMESTAMP_FIELD)?;
                }
                Some(m::EVENT_ERROR) => {
                    let error = app_server_error(&message, expected)?;
                    progress.latest_upstream_error = Some(error.message.clone());
                    if !error.will_retry {
                        progress.fatal_upstream_error = Some(error.message);
                    }
                }
                Some(m::EVENT_TURN_COMPLETED) => {
                    let status = turn_status(&message, expected)?;
                    return self.complete_turn(status, &mut progress, output).await;
                }
                Some(method) if message.id.is_some() => return Err(d::unsupported_method(method)),
                Some(method) => return Err(m::unsupported_notification(method)),
                None => return Err(m::missing_message_method()),
            }
        }
    }

    async fn complete_turn(
        &mut self,
        status: TurnStatus,
        progress: &mut TurnProgress,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        if let Some(error) = progress.fatal_upstream_error.take() {
            return Err(BridgeError::unavailable(error));
        }
        if status == TurnStatus::Completed {
            self.resolve_pending(&mut progress.limiter, output).await?;
        }
        let expected_limit_interrupt =
            status == TurnStatus::Interrupted && progress.limiter.state.is_reached();
        if status != TurnStatus::Completed && !expected_limit_interrupt {
            let error = progress
                .latest_upstream_error
                .take()
                .map_or_else(|| d::turn_status(status.as_str()), BridgeError::unavailable);
            return Err(error);
        }
        if progress.usage.is_none()
            && progress.limiter.emitted_tokens != usize::default()
            && !expected_limit_interrupt
        {
            return Err(BridgeError::protocol(d::TOKEN_USAGE_MISSING));
        }
        Ok(AssistantOutcome::Text {
            usage: progress.usage,
            cause: if progress.limiter.state.is_reached() {
                CompletionCause::OutputLimit
            } else {
                CompletionCause::EndTurn
            },
        })
    }

    async fn next_turn_event(&mut self) -> Result<RpcMessage, BridgeError> {
        let events = self
            .thread_events
            .as_mut()
            .ok_or_else(|| BridgeError::protocol(d::TURN_SUBSCRIPTION_MISSING))?;
        timeout(TURN_TIMEOUT, events.recv())
            .await
            .map_err(|_| BridgeError::unavailable(d::TURN_TIMEOUT))?
            .ok_or_else(|| BridgeError::unavailable(d::TURN_STREAM_CLOSED))?
    }

    async fn emit_delta(
        &mut self,
        delta: &str,
        expected: (&str, &str),
        limiter: &mut OutputLimiter,
        output: &dyn ModelOutput,
    ) -> Result<(), BridgeError> {
        if limiter.state.is_reached() {
            return Ok(());
        }
        limiter.pending_text.push_str(delta);
        let (stable_tokens, _completions) = self
            .output_tokenizer
            ._encode_unstable_native(&limiter.pending_text, &HashSet::new());
        if stable_tokens.is_empty() {
            return Ok(());
        }
        let remaining = limiter.max_tokens.saturating_sub(limiter.emitted_tokens);
        if stable_tokens.len() > remaining {
            let (prefix, emitted_token_count) =
                self.decode_utf8_prefix(&stable_tokens, remaining)?;
            Self::send_delta(prefix, output).await?;
            limiter.emitted_tokens = limiter
                .emitted_tokens
                .checked_add(emitted_token_count)
                .ok_or_else(|| BridgeError::protocol(d::TOKEN_COUNT_OVERFLOW))?;
            limiter.pending_text.clear();
            limiter.state = OutputLimitState::Reached;
            self.interrupt_turn(expected).await?;
            return Ok(());
        }

        let stable_text = self.decode_utf8(&stable_tokens)?;
        let stable_bytes = stable_text.len();
        Self::send_delta(stable_text, output).await?;
        limiter.pending_text.drain(..stable_bytes);
        limiter.emitted_tokens = limiter
            .emitted_tokens
            .checked_add(stable_tokens.len())
            .ok_or_else(|| BridgeError::protocol(d::TOKEN_COUNT_OVERFLOW))?;
        Ok(())
    }

    async fn resolve_pending(
        &self,
        limiter: &mut OutputLimiter,
        output: &dyn ModelOutput,
    ) -> Result<(), BridgeError> {
        if limiter.pending_text.is_empty() {
            return Ok(());
        }
        let tokens = self.output_tokenizer.encode_ordinary(&limiter.pending_text);
        let remaining = limiter.max_tokens.saturating_sub(limiter.emitted_tokens);
        if tokens.len() > remaining {
            let (prefix, emitted_token_count) = self.decode_utf8_prefix(&tokens, remaining)?;
            Self::send_delta(prefix, output).await?;
            limiter.emitted_tokens = limiter
                .emitted_tokens
                .checked_add(emitted_token_count)
                .ok_or_else(|| BridgeError::protocol(d::TOKEN_COUNT_OVERFLOW))?;
            limiter.pending_text.clear();
            limiter.state = OutputLimitState::Reached;
            return Ok(());
        }
        let text = std::mem::take(&mut limiter.pending_text);
        Self::send_delta(text, output).await?;
        limiter.emitted_tokens = limiter
            .emitted_tokens
            .checked_add(tokens.len())
            .ok_or_else(|| BridgeError::protocol(d::TOKEN_COUNT_OVERFLOW))?;
        Ok(())
    }

    async fn interrupt_turn(&mut self, expected: (&str, &str)) -> Result<(), BridgeError> {
        self.connection
            .request(
                m::TURN_INTERRUPT_METHOD,
                m::interrupt_params(expected.0, expected.1),
            )
            .await
            .map(|_| ())
    }

    async fn release_thread(&mut self) {
        self.thread_events = None;
        if let Some(thread_id) = self.thread_id.take() {
            self.connection.unregister_thread(&thread_id).await;
        }
    }

    fn decode_utf8_prefix(
        &self,
        tokens: &[tiktoken_rs::Rank],
        maximum_tokens: usize,
    ) -> Result<(String, usize), BridgeError> {
        let mut token_count = maximum_tokens.min(tokens.len());
        loop {
            let prefix = tokens
                .get(..token_count)
                .ok_or_else(|| BridgeError::protocol(d::TOKEN_BOUNDARY_INVALID))?;
            let bytes = self
                .output_tokenizer
                .decode_bytes(prefix)
                .map_err(|error| d::protocol(d::DECODE_TOKEN_BOUNDARY_CONTEXT, error))?;
            match String::from_utf8(bytes) {
                Ok(text) => return Ok((text, token_count)),
                Err(_) if token_count != usize::default() => {
                    token_count = token_count
                        .checked_sub(TOKEN_BOUNDARY_BACKOFF)
                        .ok_or_else(|| BridgeError::protocol(d::TOKEN_COUNT_UNDERFLOW))?;
                }
                Err(error) => {
                    return Err(d::protocol(d::DECODE_OUTPUT_CONTEXT, error));
                }
            }
        }
    }

    fn decode_utf8(&self, tokens: &[tiktoken_rs::Rank]) -> Result<String, BridgeError> {
        self.output_tokenizer
            .decode(tokens)
            .map_err(|error| d::protocol(d::DECODE_STABLE_TOKENS_CONTEXT, error))
    }

    async fn send_delta(text: String, output: &dyn ModelOutput) -> Result<(), BridgeError> {
        if text.is_empty() {
            return Ok(());
        }
        output.emit(AssistantTextDelta::new(text)).await
    }
}

impl Drop for CodexRuntime {
    fn drop(&mut self) {
        let Some(thread_id) = self.thread_id.take() else {
            return;
        };
        let connection = Arc::clone(&self.connection);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                connection.unregister_thread(&thread_id).await;
            });
        }
    }
}

impl AppServerConnection {
    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn connection_error(&self) -> BridgeError {
        self.failure
            .get()
            .cloned()
            .unwrap_or_else(|| BridgeError::unavailable(d::CONNECTION_STOPPED))
    }

    async fn initialize(&self) -> Result<(), BridgeError> {
        self.request(m::INITIALIZE_METHOD, m::initialize_params())
            .await?;
        self.send(m::initialized_notification()).await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        let id = self
            .next_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(RPC_REQUEST_ID_INCREMENT)
            })
            .map_err(|_| BridgeError::protocol(d::REQUEST_ID_EXHAUSTED))?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending_requests.lock().await;
            if !self.is_alive() {
                return Err(self.connection_error());
            }
            if pending.insert(id, sender).is_some() {
                return Err(BridgeError::protocol(d::REQUEST_ID_REUSED));
            }
        }
        if let Err(error) = self.send(m::rpc_request(method, id, &params)).await {
            self.pending_requests.lock().await.remove(&id);
            return Err(error);
        }
        let message = match timeout(RPC_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => {
                return Err(if self.is_alive() {
                    BridgeError::unavailable(d::RESPONSE_CHANNEL_CLOSED)
                } else {
                    self.connection_error()
                });
            }
            Err(_) => {
                let error = d::request_timeout(method);
                self.fail(error.clone()).await;
                return Err(error);
            }
        };
        if let Some(error) = message.error {
            return Err(d::rpc_failure(method, error.code, &error.message));
        }
        message.result.ok_or_else(|| d::missing_result(method))
    }

    async fn send(&self, value: Value) -> Result<(), BridgeError> {
        if !self.is_alive() {
            return Err(self.connection_error());
        }
        let bytes = m::encode_message(&value)?;
        let result = {
            let mut stdin = self.stdin.lock().await;
            match stdin.write_all(&bytes).await {
                Ok(()) => stdin.flush().await,
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            let failure = d::unavailable(d::WRITE_CODEX_CONTEXT, error);
            self.fail(failure.clone()).await;
            return Err(failure);
        }
        Ok(())
    }

    async fn register_thread(
        &self,
        thread_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<Result<RpcMessage, BridgeError>>, BridgeError> {
        if !self.is_alive() {
            return Err(self.connection_error());
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut threads = self.thread_senders.write().await;
        match threads.entry(thread_id.to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(sender);
            }
            Entry::Occupied(_) => return Err(BridgeError::protocol(d::THREAD_ID_REUSED)),
        }
        drop(threads);
        Ok(receiver)
    }

    async fn unregister_thread(&self, thread_id: &str) {
        self.thread_senders.write().await.remove(thread_id);
    }

    async fn read_loop(mut stdout: BufReader<ChildStdout>, connection: Weak<Self>) {
        loop {
            let message = match read_wire_message(&mut stdout).await {
                Ok(message) => message,
                Err(error) => {
                    if let Some(active) = connection.upgrade() {
                        active.fail(error).await;
                    }
                    return;
                }
            };
            let Some(active) = connection.upgrade() else {
                return;
            };
            if let Err(error) = active.dispatch(message).await {
                active.fail(error).await;
                return;
            }
        }
    }

    async fn dispatch(&self, message: RpcMessage) -> Result<(), BridgeError> {
        if message.method.is_none() {
            let id = message
                .id
                .as_ref()
                .and_then(JsonRpcRequestId::as_u64)
                .ok_or_else(|| BridgeError::protocol(d::RESPONSE_ID_MISSING))?;
            let sender = self
                .pending_requests
                .lock()
                .await
                .remove(&id)
                .ok_or_else(|| d::unexpected_request_id(id))?;
            let _result = sender.send(Ok(message));
            return Ok(());
        }

        if m::validate_ignored_notification(&message)? {
            return Ok(());
        }

        let thread_id = m::event_thread_id(&message);
        if thread_id.is_none()
            && let Some(method) = message
                .method
                .as_deref()
                .filter(|method| m::is_turn_scoped_event(method))
        {
            return Err(d::missing_event_thread(method));
        }
        if let Some(thread_id) = thread_id {
            let (destination, has_active_threads) = {
                let threads = self.thread_senders.read().await;
                (threads.get(&thread_id).cloned(), !threads.is_empty())
            };
            if let Some(sender) = destination {
                if sender.send(Ok(message)).is_err() {
                    self.unregister_thread(&thread_id).await;
                    return Err(d::closed_thread_receiver(&thread_id));
                }
                return Ok(());
            }
            if has_active_threads {
                return Err(d::mismatched_event_scope(
                    message.method.as_deref(),
                    &thread_id,
                ));
            }
            return Err(d::unknown_thread(&thread_id));
        }

        if let Some(method) = message.method.as_deref().filter(|_| message.id.is_some()) {
            return Err(d::unsupported_method(method));
        }
        match message.method.as_deref() {
            Some(method) => Err(m::unsupported_notification(method)),
            None => Err(m::missing_message_method()),
        }
    }

    async fn fail(&self, error: BridgeError) {
        if !self.alive.swap(CONNECTION_FAILED_STATE, Ordering::AcqRel) {
            return;
        }
        let _stored = self.failure.set(error.clone());
        if let Err(kill_error) = self.child.lock().await.start_kill() {
            d::warn_process_kill(kill_error);
        }
        let pending = std::mem::take(&mut *self.pending_requests.lock().await);
        for sender in pending.into_values() {
            let _result = sender.send(Err(error.clone()));
        }
        let threads = std::mem::take(&mut *self.thread_senders.write().await);
        for sender in threads.into_values() {
            let _result = sender.send(Err(error.clone()));
        }
    }
}

async fn read_wire_message(stdout: &mut BufReader<ChildStdout>) -> Result<RpcMessage, BridgeError> {
    let line = read_capped_line(stdout).await?;
    m::decode_message(&line)
}

async fn read_capped_line(stdout: &mut BufReader<ChildStdout>) -> Result<String, BridgeError> {
    let mut frame = Vec::new();
    loop {
        let available = stdout
            .fill_buf()
            .await
            .map_err(|error| d::unavailable(d::READ_STDOUT_CONTEXT, error))?;
        if available.is_empty() {
            return Err(BridgeError::unavailable(d::STDOUT_CLOSED));
        }
        let newline = available
            .iter()
            .position(|byte| *byte == d::FRAME_DELIMITER);
        let take = newline.unwrap_or(available.len());
        let next_len = frame
            .len()
            .checked_add(take)
            .ok_or_else(|| BridgeError::protocol(d::FRAME_LENGTH_OVERFLOW))?;
        if next_len > MAX_APP_SERVER_FRAME_BYTES {
            return Err(d::frame_too_large(MAX_APP_SERVER_FRAME_BYTES));
        }
        frame.extend_from_slice(
            available
                .get(..take)
                .ok_or_else(|| BridgeError::protocol(d::FRAME_BOUNDARY_INVALID))?,
        );
        stdout.consume(take + usize::from(newline.is_some()));
        if newline.is_some() {
            return String::from_utf8(frame)
                .map_err(|error| d::protocol(d::FRAME_UTF8_CONTEXT, error));
        }
    }
}

/// Builds the exact restricted model catalogue supplied to Codex App Server.
///
/// # Errors
///
/// Returns an error when the checked-in protocol template is malformed.
pub(crate) fn initialize_output_tokenizer() -> Result<(), BridgeError> {
    output_tokenizer().map(|_| ())
}

fn output_tokenizer() -> Result<&'static CoreBPE, BridgeError> {
    OUTPUT_TOKENIZER
        .get_or_init(|| tiktoken_rs::o200k_base().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| d::configuration(d::TOKENIZER_CONTEXT, error))
}

async fn verify_codex_version(codex_bin: &Path) -> Result<(), BridgeError> {
    let output = Command::new(codex_bin)
        .arg(d::CODEX_VERSION_ARGUMENT)
        .env_clear()
        .env(d::PATH_ENV, TRUSTED_CHILD_PATH)
        .output()
        .await
        .map_err(|error| d::configuration(d::INSPECT_CODEX_CONTEXT, error))?;
    let version = String::from_utf8(output.stdout)
        .map_err(|error| d::configuration(d::VERSION_UTF8_CONTEXT, error))?;
    if !output.status.success() || version.trim() != CODEX_CLI_VERSION_OUTPUT {
        return Err(d::wrong_version(version.trim()));
    }
    Ok(())
}
