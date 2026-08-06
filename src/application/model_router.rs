use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    sync::{RwLock, Semaphore},
    time::sleep,
};
use uuid::Uuid;

use crate::{
    domain::BridgeError,
    domain::{
        AnthropicResponseChunk, AnthropicResponseHead, AssistantOutcome, AssistantTextDelta,
        AuthorizedRequest, ClaudeModelId, ClaudeSessionId, ContinueModelTurn, ExecuteMessage,
        ExpectedCredential, ModelRequest, ModelResponseChunk, ModelResponseEnd, ModelResponseHead,
        PresentedCredential, StartModelTurn, ToolUseId, WorkingDirectory,
    },
    policies::{
        model_prompt::{developer_instructions, model_prompt},
        router_limits::{MAX_CONCURRENT_GPT_TURNS, TOOL_SESSION_TTL},
    },
    ports::{
        AnthropicGateway, AnthropicResponseSink, ModelOutput, ModelResponseSink, ModelRouter,
        ModelSession, ModelSessionFactory, PortFuture,
    },
};

#[derive(Clone)]
pub(crate) struct ModelRouterService {
    expected_credential: ExpectedCredential,
    working_directory: WorkingDirectory,
    anthropic: Arc<dyn AnthropicGateway>,
    session_factory: Arc<dyn ModelSessionFactory>,
    pending_calls: Arc<RwLock<HashMap<SessionKey, Session>>>,
    admission: Arc<Semaphore>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct SessionKey {
    claude_session_id: ClaudeSessionId,
    tool_use_id: ToolUseId,
}

struct Session {
    execution: Box<dyn ModelSession>,
    route: ClaudeModelId,
    expiry_id: Uuid,
}

impl ModelRouterService {
    #[must_use]
    pub(crate) fn new(
        expected_credential: ExpectedCredential,
        working_directory: WorkingDirectory,
        anthropic: Arc<dyn AnthropicGateway>,
        session_factory: Arc<dyn ModelSessionFactory>,
    ) -> Self {
        Self {
            expected_credential,
            working_directory,
            anthropic,
            session_factory,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            admission: Arc::new(Semaphore::new(MAX_CONCURRENT_GPT_TURNS)),
        }
    }

    /// Authorizes one loopback request against the launch-scoped credential.
    ///
    /// # Errors
    ///
    /// Returns an authentication error without revealing credential contents.
    pub(crate) fn authorize(
        &self,
        credential: &PresentedCredential,
    ) -> Result<AuthorizedRequest, BridgeError> {
        let expected = Sha256::digest(self.expected_credential.expose().as_bytes());
        let candidate = Sha256::digest(credential.expose().as_bytes());
        let matches: bool = expected.ct_eq(&candidate).into();
        if matches {
            Ok(AuthorizedRequest::granted())
        } else {
            Err(BridgeError::Authentication)
        }
    }

    /// Executes one initial or continuing model request.
    ///
    /// # Errors
    ///
    /// Returns an explicit application, model, or protocol error without fallback.
    pub(crate) async fn handle(
        &self,
        _authorization: AuthorizedRequest,
        request: ExecuteMessage,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        let route = request.requested_model().clone();
        if let Some(tool_result) = request.pending_tool_result() {
            let key = SessionKey {
                claude_session_id: request.session().clone(),
                tool_use_id: tool_result.id().clone(),
            };
            let session = {
                let mut sessions = self.pending_calls.write().await;
                let active = sessions.get(&key).ok_or_else(|| {
                    BridgeError::invalid_request("tool result has no matching pending tool call")
                })?;
                if active.route != route.claude_model {
                    return Err(BridgeError::invalid_request(
                        "tool result model does not match its pending tool call",
                    ));
                }
                sessions.remove(&key).ok_or_else(|| {
                    BridgeError::protocol("pending tool call disappeared while it was locked")
                })?
            };
            let Session {
                execution: mut model_session,
                route,
                ..
            } = session;
            let _turn_permit = self
                .admission
                .acquire()
                .await
                .map_err(|_| BridgeError::unavailable("GPT turn scheduler is unavailable"))?;
            let outcome = model_session
                .continue_tool(
                    ContinueModelTurn::new(
                        tool_result.text().clone(),
                        tool_result.disposition(),
                        request.output_limit(),
                    ),
                    output,
                )
                .await?;
            return self
                .finish_outcome(model_session, outcome, request.session().clone(), route)
                .await;
        }

        let _turn_permit = self
            .admission
            .acquire()
            .await
            .map_err(|_| BridgeError::unavailable("GPT turn scheduler is unavailable"))?;
        let mut model_session = self.session_factory.launch().await?;
        let prompt = model_prompt(request.conversation());
        let developer_instructions =
            developer_instructions(request.system(), request.output_limit());
        let outcome = model_session
            .start_turn(
                StartModelTurn::new(
                    route.codex_model.clone(),
                    self.working_directory.clone(),
                    request.tools().clone(),
                    prompt,
                    developer_instructions,
                    request.output_limit(),
                    route.reasoning_effort,
                    route.service_tier,
                    request.output_schema().cloned(),
                ),
                output,
            )
            .await?;
        self.finish_outcome(
            model_session,
            outcome,
            request.session().clone(),
            route.claude_model,
        )
        .await
    }

    async fn dispatch_request(
        &self,
        authorization: AuthorizedRequest,
        request: ModelRequest,
        output: &dyn ModelResponseSink,
    ) -> Result<ModelResponseEnd, BridgeError> {
        match request {
            ModelRequest::Assistant {
                model,
                streaming,
                execution,
            } => {
                output
                    .start(ModelResponseHead::Assistant { model, streaming })
                    .await?;
                let model_output = RoutedModelOutput(output);
                self.handle(authorization, execution, &model_output)
                    .await
                    .map(ModelResponseEnd::Assistant)
            }
            ModelRequest::Anthropic(request) => {
                let anthropic_output = RoutedAnthropicOutput(output);
                self.anthropic.exchange(request, &anthropic_output).await?;
                Ok(ModelResponseEnd::Anthropic)
            }
        }
    }

    async fn finish_outcome(
        &self,
        model_session: Box<dyn ModelSession>,
        outcome: AssistantOutcome,
        claude_session_id: ClaudeSessionId,
        route: ClaudeModelId,
    ) -> Result<AssistantOutcome, BridgeError> {
        let AssistantOutcome::ToolCall(tool_call) = outcome else {
            return Ok(outcome);
        };
        let key = SessionKey {
            claude_session_id,
            tool_use_id: tool_call.id().clone(),
        };
        self.retain_pending_session(key, model_session, route)
            .await?;
        Ok(AssistantOutcome::ToolCall(tool_call))
    }

    async fn retain_pending_session(
        &self,
        key: SessionKey,
        model_session: Box<dyn ModelSession>,
        route: ClaudeModelId,
    ) -> Result<(), BridgeError> {
        let expiry_id = Uuid::now_v7();
        match self.pending_calls.write().await.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(Session {
                    execution: model_session,
                    route,
                    expiry_id,
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
        Ok(())
    }
}

impl ModelRouter for ModelRouterService {
    fn authorize(
        &self,
        credential: &PresentedCredential,
    ) -> Result<AuthorizedRequest, BridgeError> {
        ModelRouterService::authorize(self, credential)
    }

    fn dispatch<'a>(
        &'a self,
        authorization: AuthorizedRequest,
        request: ModelRequest,
        output: &'a dyn ModelResponseSink,
    ) -> PortFuture<'a, ModelResponseEnd> {
        Box::pin(self.dispatch_request(authorization, request, output))
    }
}

struct RoutedModelOutput<'a>(&'a dyn ModelResponseSink);

impl ModelOutput for RoutedModelOutput<'_> {
    fn emit(&self, delta: AssistantTextDelta) -> PortFuture<'_, ()> {
        self.0.emit(ModelResponseChunk::Assistant(delta))
    }
}

struct RoutedAnthropicOutput<'a>(&'a dyn ModelResponseSink);

impl AnthropicResponseSink for RoutedAnthropicOutput<'_> {
    fn start(&self, head: AnthropicResponseHead) -> PortFuture<'_, ()> {
        self.0.start(ModelResponseHead::Anthropic(head))
    }

    fn emit(&self, chunk: AnthropicResponseChunk) -> PortFuture<'_, ()> {
        self.0.emit(ModelResponseChunk::Anthropic(chunk))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ModelRouterService, SessionKey};
    use crate::{
        domain::{
            AnthropicRequest, AssistantOutcome, ClaudeModelId, ClaudeSessionId, ContinueModelTurn,
            ExpectedCredential, StartModelTurn, ToolUseId, WorkingDirectory,
        },
        policies::router_limits::TOOL_SESSION_TTL,
        ports::{
            AnthropicGateway, AnthropicResponseSink, ModelOutput, ModelSession,
            ModelSessionFactory, PortFuture,
        },
    };

    struct UnusedGateway;

    impl AnthropicGateway for UnusedGateway {
        fn exchange<'a>(
            &'a self,
            _request: AnthropicRequest,
            _output: &'a dyn AnthropicResponseSink,
        ) -> PortFuture<'a, ()> {
            Box::pin(async { Err(crate::domain::BridgeError::protocol("unused gateway")) })
        }
    }

    struct UnusedSession;

    impl ModelSession for UnusedSession {
        fn start_turn<'a>(
            &'a mut self,
            _request: StartModelTurn,
            _output: &'a dyn ModelOutput,
        ) -> PortFuture<'a, AssistantOutcome> {
            Box::pin(async { Err(crate::domain::BridgeError::protocol("unused turn")) })
        }

        fn continue_tool<'a>(
            &'a mut self,
            _resolution: ContinueModelTurn,
            _output: &'a dyn ModelOutput,
        ) -> PortFuture<'a, AssistantOutcome> {
            Box::pin(async { Err(crate::domain::BridgeError::protocol("unused continuation")) })
        }
    }

    struct UnusedFactory;

    impl ModelSessionFactory for UnusedFactory {
        fn launch(&self) -> PortFuture<'_, Box<dyn ModelSession>> {
            Box::pin(async { Err(crate::domain::BridgeError::protocol("unused factory")) })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn abandoned_tool_session_expires_after_exact_ttl()
    -> Result<(), crate::domain::BridgeError> {
        let router = ModelRouterService::new(
            ExpectedCredential::new("test-credential"),
            WorkingDirectory::new("/tmp"),
            Arc::new(UnusedGateway),
            Arc::new(UnusedFactory),
        );
        let key = SessionKey {
            claude_session_id: ClaudeSessionId::from("ttl-session"),
            tool_use_id: ToolUseId::from("ttl-tool"),
        };
        router
            .retain_pending_session(
                key,
                Box::new(UnusedSession),
                ClaudeModelId::new("anthropic-model-rocket-test"),
            )
            .await?;
        assert_eq!(router.pending_calls.read().await.len(), 1);
        assert_eq!(TOOL_SESSION_TTL, std::time::Duration::from_secs(600));

        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(599)).await;
        tokio::task::yield_now().await;
        assert_eq!(router.pending_calls.read().await.len(), 1);

        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;

        assert!(router.pending_calls.read().await.is_empty());
        Ok(())
    }
}
