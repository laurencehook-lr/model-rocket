use std::ffi::OsStr;
use std::io::{self, Write};

use model_rocket::{
    bootstrap, claude_settings,
    config::{Config, PreflightConfig, model_catalogue_from_env},
    domain::BridgeError,
    domain::{AccountMode, PreflightReport},
    product,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Serialize)]
struct PreflightOutput {
    account_type: &'static str,
    models: Vec<String>,
}

impl TryFrom<Vec<PreflightReport>> for PreflightOutput {
    type Error = BridgeError;

    fn try_from(reports: Vec<PreflightReport>) -> Result<Self, Self::Error> {
        let account_type = match reports
            .first()
            .ok_or_else(|| BridgeError::protocol("preflight returned no configured models"))?
            .account()
        {
            AccountMode::ManagedChatGpt => product::MANAGED_ACCOUNT_TYPE,
        };
        let models = reports
            .iter()
            .map(|report| report.model().as_str().to_owned())
            .collect();
        Ok(Self {
            account_type,
            models,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), BridgeError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        Some(command) if command == OsStr::new("preflight") => {
            reject_trailing_arguments(&mut arguments, "preflight")?;
            preflight().await
        }
        Some(command) if command == OsStr::new("serve") => {
            reject_trailing_arguments(&mut arguments, "serve")?;
            serve().await
        }
        Some(command) if command == OsStr::new("validate-settings") => {
            claude_settings::validate(arguments)
        }
        Some(command) if command == OsStr::new("validate-claude") => {
            let path = arguments.next().ok_or_else(|| {
                BridgeError::configuration("validate-claude requires one executable path")
            })?;
            if arguments.next().is_some() {
                return Err(BridgeError::configuration(
                    "validate-claude accepts exactly one executable path",
                ));
            }
            model_rocket::contracts::claude_executable::validate(std::path::Path::new(&path))?;
            Ok(())
        }
        Some(command) if command == OsStr::new("guard-settings-change") => {
            reject_trailing_arguments(&mut arguments, "guard-settings-change")?;
            claude_settings::guard_config_change(io::stdin().lock(), io::stdout().lock())
        }
        Some(command) if command == OsStr::new("launcher-contract") => {
            reject_trailing_arguments(&mut arguments, "launcher-contract")?;
            let catalogue = model_catalogue_from_env()?;
            product::write_launcher_contract(&catalogue, io::stdout().lock()).map_err(|error| {
                BridgeError::unavailable(format!("cannot write launcher contract: {error}"))
            })
        }
        Some(command) if command == OsStr::new("worker-config") => {
            reject_trailing_arguments(&mut arguments, "worker-config")?;
            let catalogue = model_catalogue_from_env()?;
            product::write_worker_config(&catalogue, io::stdout().lock()).map_err(|error| {
                BridgeError::unavailable(format!("cannot write worker configuration: {error}"))
            })
        }
        Some(command) if command == OsStr::new("canonical-route") => {
            reject_trailing_arguments(&mut arguments, "canonical-route")?;
            let catalogue = model_catalogue_from_env()?;
            writeln!(
                io::stdout().lock(),
                "{}",
                catalogue.canonical_route().claude_model.as_str()
            )
            .map_err(|error| {
                BridgeError::unavailable(format!("cannot write canonical route: {error}"))
            })
        }
        Some(command) if command == OsStr::new("launcher-settings") => {
            let bridge_bin = required_argument(&mut arguments, "bridge executable")?;
            let base_url = required_argument(&mut arguments, "bridge base URL")?;
            let proxy_bypass = required_argument(&mut arguments, "proxy bypass")?;
            if arguments.next().is_some() {
                return Err(BridgeError::configuration(
                    "launcher-settings accepts exactly three arguments",
                ));
            }
            let catalogue = model_catalogue_from_env()?;
            claude_settings::write_routing_settings(
                &catalogue,
                &bridge_bin,
                &base_url,
                &proxy_bypass,
                io::stdout().lock(),
            )
        }
        Some(command) => Err(BridgeError::configuration(format!(
            "unknown command {}; expected preflight, serve, validate-settings, validate-claude, guard-settings-change, launcher-contract, launcher-settings, worker-config, or canonical-route",
            command.to_string_lossy()
        ))),
        None => Err(BridgeError::configuration(
            "missing command; expected preflight, serve, validate-settings, validate-claude, guard-settings-change, launcher-contract, launcher-settings, worker-config, or canonical-route",
        )),
    }
}

async fn preflight() -> Result<(), BridgeError> {
    let config = PreflightConfig::from_env()?;
    let report = bootstrap::preflight(&config).await?;
    let output = PreflightOutput::try_from(report)?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, &output).map_err(|error| {
        BridgeError::protocol(format!("cannot encode preflight report: {error}"))
    })?;
    writeln!(stdout).map_err(|error| {
        BridgeError::unavailable(format!("cannot write preflight report: {error}"))
    })
}

async fn serve() -> Result<(), BridgeError> {
    let config = Config::from_env()?;
    let model_router = bootstrap::model_router(&config)?;
    let app = bootstrap::http_router(model_router, std::sync::Arc::clone(config.catalogue()))?;
    let listener = TcpListener::bind(config.listen())
        .await
        .map_err(|error| BridgeError::configuration(format!("cannot bind listener: {error}")))?;
    let local_addr = listener.local_addr().map_err(|error| {
        BridgeError::configuration(format!("cannot read listener address: {error}"))
    })?;
    if let Some(path) = config.ready_file() {
        std::fs::write(path, local_addr.to_string()).map_err(|error| {
            BridgeError::configuration(format!("cannot write MODEL_ROCKET_READY_FILE: {error}"))
        })?;
    }
    info!(address = %local_addr, "bridge listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| BridgeError::unavailable(format!("HTTP server failed: {error}")))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _result = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _result = tokio::signal::ctrl_c().await;
    }
}

fn reject_trailing_arguments(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    command: &str,
) -> Result<(), BridgeError> {
    if arguments.next().is_some() {
        return Err(BridgeError::configuration(format!(
            "{command} does not accept arguments"
        )));
    }
    Ok(())
}

fn required_argument(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    name: &str,
) -> Result<String, BridgeError> {
    arguments
        .next()
        .ok_or_else(|| BridgeError::configuration(format!("launcher-settings requires {name}")))?
        .into_string()
        .map_err(|_| BridgeError::configuration(format!("{name} must be valid UTF-8")))
}
