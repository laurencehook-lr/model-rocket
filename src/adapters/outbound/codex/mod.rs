//! Codex App Server adapter.

use std::sync::Arc;

use crate::{
    domain::BridgeError,
    domain::{
        AssistantOutcome, ContinueModelTurn, ModelCatalogue, PreflightReport, StartModelTurn,
        ValidatedCodexExecutable,
    },
    ports::{ModelOutput, ModelSession, ModelSessionFactory, PortFuture},
};

mod runtime;

use runtime::{CodexRuntime, CodexRuntimeFactory};

pub(crate) fn initialize_output_tokenizer() -> Result<(), BridgeError> {
    runtime::initialize_output_tokenizer()
}

pub struct AppServer {
    runtime: CodexRuntime,
}

impl AppServer {
    /// Launches the Codex runtime using its standard subscription transport.
    ///
    /// # Errors
    ///
    /// Returns an error when the runtime cannot be started.
    pub async fn launch(
        executable: &ValidatedCodexExecutable,
        catalogue: Arc<ModelCatalogue>,
    ) -> Result<Self, BridgeError> {
        Self::launch_with_provider(executable, None, catalogue).await
    }

    pub(crate) async fn launch_with_provider(
        executable: &ValidatedCodexExecutable,
        provider_endpoint: Option<&reqwest::Url>,
        catalogue: Arc<ModelCatalogue>,
    ) -> Result<Self, BridgeError> {
        let runtime = CodexRuntime::launch(executable, provider_endpoint, catalogue).await?;
        Ok(Self { runtime })
    }

    /// Launches Codex against its account-backed remote model catalogue.
    ///
    /// This intentionally omits Model Rocket's generated `model_catalog_json` so availability
    /// cannot be confirmed by configuration that Model Rocket supplied itself.
    ///
    /// # Errors
    ///
    /// Returns an error when the discovery runtime cannot be started.
    pub async fn launch_discovery(
        executable: &ValidatedCodexExecutable,
    ) -> Result<Self, BridgeError> {
        let runtime = CodexRuntime::launch_discovery(executable).await?;
        Ok(Self { runtime })
    }

    /// Verifies the authenticated account and requested model.
    ///
    /// # Errors
    ///
    /// Returns an error when preflight fails.
    pub async fn preflight(
        &mut self,
        catalogue: &ModelCatalogue,
    ) -> Result<Vec<PreflightReport>, BridgeError> {
        self.runtime.preflight(catalogue).await
    }

    /// Starts a model turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the turn cannot be completed.
    pub async fn start_turn(
        &mut self,
        request: StartModelTurn,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        self.runtime.start_turn(request, output).await
    }

    /// Resolves a tool call and continues the active turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the turn cannot be continued.
    pub async fn continue_tool(
        &mut self,
        resolution: ContinueModelTurn,
        output: &dyn ModelOutput,
    ) -> Result<AssistantOutcome, BridgeError> {
        self.runtime.continue_tool(resolution, output).await
    }
}

#[derive(Clone)]
pub struct AppServerFactory {
    runtime: CodexRuntimeFactory,
}

impl AppServerFactory {
    #[must_use]
    pub(crate) fn new(
        executable: ValidatedCodexExecutable,
        catalogue: Arc<ModelCatalogue>,
    ) -> Self {
        Self {
            runtime: CodexRuntimeFactory::new(executable, catalogue),
        }
    }
}

impl ModelSession for AppServer {
    fn start_turn<'a>(
        &'a mut self,
        request: StartModelTurn,
        output: &'a dyn ModelOutput,
    ) -> PortFuture<'a, AssistantOutcome> {
        Box::pin(AppServer::start_turn(self, request, output))
    }

    fn continue_tool<'a>(
        &'a mut self,
        resolution: ContinueModelTurn,
        output: &'a dyn ModelOutput,
    ) -> PortFuture<'a, AssistantOutcome> {
        Box::pin(AppServer::continue_tool(self, resolution, output))
    }
}

impl ModelSessionFactory for AppServerFactory {
    fn launch(&self) -> PortFuture<'_, Box<dyn ModelSession>> {
        Box::pin(async {
            let runtime = self.runtime.launch_session().await?;
            Ok(Box::new(AppServer { runtime }) as Box<dyn ModelSession>)
        })
    }
}
