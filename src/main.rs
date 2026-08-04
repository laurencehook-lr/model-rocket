use std::io::{self, Write};

use model_rocket::{
    app_server::AppServer,
    bridge::Bridge,
    config::{Config, PreflightConfig},
    error::BridgeError,
    http,
};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), BridgeError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("preflight") => preflight().await,
        Some("serve") => serve().await,
        Some(command) => Err(BridgeError::configuration(format!(
            "unknown command {command}; expected preflight or serve"
        ))),
        None => Err(BridgeError::configuration(
            "missing command; expected preflight or serve",
        )),
    }
}

async fn preflight() -> Result<(), BridgeError> {
    let config = PreflightConfig::from_env();
    let mut app_server = AppServer::launch(&config.codex_bin).await?;
    let report = app_server.preflight(&config.model).await?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &report).map_err(|error| {
        BridgeError::protocol(format!("cannot encode preflight report: {error}"))
    })?;
    writeln!(stdout).map_err(|error| {
        BridgeError::unavailable(format!("cannot write preflight report: {error}"))
    })
}

async fn serve() -> Result<(), BridgeError> {
    let config = Config::from_env()?;
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(|error| BridgeError::configuration(format!("cannot bind listener: {error}")))?;
    let local_addr = listener.local_addr().map_err(|error| {
        BridgeError::configuration(format!("cannot read listener address: {error}"))
    })?;
    if let Some(path) = &config.ready_file {
        std::fs::write(path, local_addr.to_string()).map_err(|error| {
            BridgeError::configuration(format!("cannot write MODEL_ROCKET_READY_FILE: {error}"))
        })?;
    }
    info!(address = %local_addr, model = %config.model, "bridge listening");
    axum::serve(listener, http::router(Bridge::new(config))?)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| BridgeError::unavailable(format!("HTTP server failed: {error}")))
}

async fn shutdown_signal() {
    let _result = tokio::signal::ctrl_c().await;
}
