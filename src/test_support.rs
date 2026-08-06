use std::sync::Arc;

use crate::{
    adapters::outbound::codex::AppServer,
    domain::{BridgeError, ModelCatalogue, ValidatedCodexExecutable},
};

/// Launches the production adapter against a loopback wire-capture endpoint.
///
/// # Errors
///
/// Returns an error when the endpoint is not loopback HTTP or startup fails.
pub async fn launch_for_wire_capture(
    executable: &ValidatedCodexExecutable,
    provider_endpoint: &reqwest::Url,
    catalogue: Arc<ModelCatalogue>,
) -> Result<AppServer, BridgeError> {
    AppServer::launch_with_provider(executable, Some(provider_endpoint), catalogue).await
}
