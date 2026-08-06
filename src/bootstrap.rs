use std::sync::Arc;

use axum::Router;

use crate::{
    adapters::{
        inbound::http,
        outbound::{
            anthropic::ReqwestAnthropicGateway,
            codex::{self, AppServer, AppServerFactory},
        },
    },
    application::ModelRouterService,
    config::{Config, PreflightConfig},
    domain::BridgeError,
    domain::{ExpectedCredential, PreflightReport, WorkingDirectory},
    ports::{AnthropicGateway, ModelRouter, ModelSessionFactory},
};

/// Wires the production model router and Codex session factory.
///
/// # Errors
///
/// Returns an error when the configured working directory is not valid UTF-8.
pub fn model_router(config: &Config) -> Result<Arc<dyn ModelRouter>, BridgeError> {
    let anthropic: Arc<dyn AnthropicGateway> = Arc::new(ReqwestAnthropicGateway::new()?);
    model_router_with_anthropic(config, anthropic)
}

/// Wires a model router with an explicitly supplied Anthropic gateway.
///
/// # Errors
///
/// Returns an error when the configured working directory is not valid UTF-8.
pub fn model_router_with_anthropic(
    config: &Config,
    anthropic: Arc<dyn AnthropicGateway>,
) -> Result<Arc<dyn ModelRouter>, BridgeError> {
    let working_directory = config
        .working_directory()
        .to_str()
        .ok_or_else(|| BridgeError::configuration("MODEL_ROCKET_CWD must contain valid UTF-8"))?;
    let session_factory: Arc<dyn ModelSessionFactory> = Arc::new(AppServerFactory::new(
        config.codex_executable().clone(),
        Arc::clone(config.catalogue()),
    ));
    Ok(Arc::new(ModelRouterService::new(
        ExpectedCredential::new(config.bearer()),
        WorkingDirectory::new(working_directory),
        anthropic,
        session_factory,
    )))
}

/// Verifies the configured Codex subscription and required model.
///
/// # Errors
///
/// Returns an error when Codex cannot start, authenticate, or expose the required model.
pub async fn preflight(config: &PreflightConfig) -> Result<Vec<PreflightReport>, BridgeError> {
    let mut app_server = AppServer::launch_discovery(config.codex_executable()).await?;
    app_server.preflight(config.catalogue()).await
}

/// Wires the production HTTP router and Anthropic gateway.
///
/// # Errors
///
/// Returns an error when the tokenizer or Anthropic gateway cannot be initialized.
pub fn http_router(
    model_router: Arc<dyn ModelRouter>,
    catalogue: Arc<crate::domain::ModelCatalogue>,
) -> Result<Router, BridgeError> {
    codex::initialize_output_tokenizer()?;
    Ok(http::router(model_router, catalogue))
}
