//! Application ports for model execution.

use std::{future::Future, pin::Pin};

use crate::{
    domain::BridgeError,
    domain::{
        AnthropicRequest, AnthropicResponseChunk, AnthropicResponseHead, AssistantOutcome,
        AssistantTextDelta, AuthorizedRequest, ContinueModelTurn, ModelRequest, ModelResponseChunk,
        ModelResponseEnd, ModelResponseHead, PresentedCredential, StartModelTurn,
    },
};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, BridgeError>> + Send + 'a>>;

pub trait ModelOutput: Send + Sync {
    fn emit(&self, delta: AssistantTextDelta) -> PortFuture<'_, ()>;
}

pub trait ModelRouter: Send + Sync {
    /// Authorizes a request using a launch-scoped credential.
    ///
    /// # Errors
    ///
    /// Returns an authentication error when the credential does not match.
    fn authorize(&self, credential: &PresentedCredential)
    -> Result<AuthorizedRequest, BridgeError>;

    fn dispatch<'a>(
        &'a self,
        authorization: AuthorizedRequest,
        request: ModelRequest,
        output: &'a dyn ModelResponseSink,
    ) -> PortFuture<'a, ModelResponseEnd>;
}

pub trait ModelResponseSink: Send + Sync {
    fn start(&self, head: ModelResponseHead) -> PortFuture<'_, ()>;

    fn emit(&self, chunk: ModelResponseChunk) -> PortFuture<'_, ()>;
}

pub trait ModelSession: Send + Sync {
    fn start_turn<'a>(
        &'a mut self,
        request: StartModelTurn,
        output: &'a dyn ModelOutput,
    ) -> PortFuture<'a, AssistantOutcome>;

    fn continue_tool<'a>(
        &'a mut self,
        resolution: ContinueModelTurn,
        output: &'a dyn ModelOutput,
    ) -> PortFuture<'a, AssistantOutcome>;
}

pub trait ModelSessionFactory: Send + Sync {
    fn launch(&self) -> PortFuture<'_, Box<dyn ModelSession>>;
}

pub trait AnthropicResponseSink: Send + Sync {
    fn start(&self, head: AnthropicResponseHead) -> PortFuture<'_, ()>;

    fn emit(&self, chunk: AnthropicResponseChunk) -> PortFuture<'_, ()>;
}

pub trait AnthropicGateway: Send + Sync {
    fn exchange<'a>(
        &'a self,
        request: AnthropicRequest,
        output: &'a dyn AnthropicResponseSink,
    ) -> PortFuture<'a, ()>;
}
