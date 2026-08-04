use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
    time::Duration,
};

use tokio::{
    sync::{OwnedSemaphorePermit, RwLock, Semaphore, mpsc},
    time::sleep,
};
use uuid::Uuid;

use crate::{
    anthropic::MessagesRequest,
    app_server::{AppServer, PendingToolCall, StartTurnRequest, TokenUsage, TurnOutcome},
    config::Config,
    error::BridgeError,
};

#[derive(Clone)]
pub struct Bridge {
    config: Config,
    pending_calls: Arc<RwLock<HashMap<SessionKey, Session>>>,
    admission: Arc<Semaphore>,
}

const TOOL_SESSION_TTL: Duration = Duration::from_secs(600);
const MAX_ACTIVE_APP_SERVERS: usize = 4;

#[derive(Debug)]
pub enum AssistantOutcome {
    Text {
        usage: TokenUsage,
        limit_reached: bool,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        usage: Option<TokenUsage>,
    },
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SessionKey {
    claude_session_id: String,
    tool_use_id: String,
}

struct Session {
    app_server: AppServer,
    pending: PendingToolCall,
    expiry_id: Uuid,
    _permit: OwnedSemaphorePermit,
}

impl Bridge {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            admission: Arc::new(Semaphore::new(MAX_ACTIVE_APP_SERVERS)),
        }
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Translates one initial or continuing Claude Messages request.
    ///
    /// # Errors
    ///
    /// Returns an explicit request, App Server, or translation error without fallback.
    pub async fn handle(
        &self,
        request: &MessagesRequest,
        claude_session_id: &str,
        deltas: &mpsc::Sender<String>,
    ) -> Result<AssistantOutcome, BridgeError> {
        request.validate(&self.config.model)?;

        if let Some(tool_result) = request.final_tool_result()? {
            let key = SessionKey {
                claude_session_id: claude_session_id.to_owned(),
                tool_use_id: tool_result.tool_use_id.clone(),
            };
            let session = self
                .pending_calls
                .write()
                .await
                .remove(&key)
                .ok_or_else(|| {
                    BridgeError::invalid_request("tool result has no matching pending tool call")
                })?;
            let Session {
                mut app_server,
                pending,
                _permit: permit,
                ..
            } = session;
            let outcome = app_server
                .continue_tool(
                    pending,
                    &tool_result.text,
                    !tool_result.is_error,
                    request.max_tokens,
                    deltas,
                )
                .await?;
            return self
                .finish_outcome(app_server, outcome, claude_session_id, permit)
                .await;
        }

        let permit = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| BridgeError::unavailable("GPT route is at its four-session limit"))?;
        let mut app_server = AppServer::launch(&self.config.codex_bin).await?;
        app_server.preflight(&self.config.model).await?;
        let prompt = request.transcript()?;
        let developer_instructions = request.developer_instructions();
        let effort = request.reasoning_effort()?;
        let output_schema = request.output_schema()?;
        let outcome = app_server
            .start_turn(
                StartTurnRequest {
                    model: &self.config.model,
                    cwd: &self.config.cwd,
                    tools: &request.tools,
                    prompt: &prompt,
                    developer_instructions: &developer_instructions,
                    max_tokens: request.max_tokens,
                    effort,
                    output_schema,
                },
                deltas,
            )
            .await?;
        self.finish_outcome(app_server, outcome, claude_session_id, permit)
            .await
    }

    async fn finish_outcome(
        &self,
        app_server: AppServer,
        outcome: TurnOutcome,
        claude_session_id: &str,
        permit: OwnedSemaphorePermit,
    ) -> Result<AssistantOutcome, BridgeError> {
        match outcome {
            TurnOutcome::Text {
                usage,
                limit_reached,
            } => Ok(AssistantOutcome::Text {
                usage,
                limit_reached,
            }),
            TurnOutcome::ToolCall(pending) => {
                let call_id = pending.call_id.clone();
                let key = SessionKey {
                    claude_session_id: claude_session_id.to_owned(),
                    tool_use_id: call_id.clone(),
                };
                let outcome = AssistantOutcome::ToolCall {
                    id: call_id.clone(),
                    name: pending.tool.clone(),
                    input: pending.arguments.clone(),
                    usage: pending.usage,
                };
                let expiry_id = Uuid::now_v7();
                match self.pending_calls.write().await.entry(key.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(Session {
                            app_server,
                            pending,
                            expiry_id,
                            _permit: permit,
                        });
                    }
                    Entry::Occupied(_) => {
                        return Err(BridgeError::protocol(
                            "Codex reused an active dynamic tool call identifier",
                        ));
                    }
                }
                let pending_calls = Arc::clone(&self.pending_calls);
                tokio::spawn(async move {
                    sleep(TOOL_SESSION_TTL).await;
                    let mut sessions = pending_calls.write().await;
                    if sessions
                        .get(&key)
                        .is_some_and(|session| session.expiry_id == expiry_id)
                    {
                        sessions.remove(&key);
                    }
                });
                Ok(outcome)
            }
        }
    }
}
