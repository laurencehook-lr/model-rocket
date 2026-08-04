use std::{env, net::SocketAddr, path::PathBuf};

use crate::error::BridgeError;

pub const DEFAULT_MODEL: &str = "gpt-5.6-sol";

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub ready_file: Option<PathBuf>,
    pub model: String,
    pub codex_bin: PathBuf,
    pub cwd: PathBuf,
    pub bearer: String,
}

impl Config {
    /// Loads and validates the serving configuration from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing bearer, invalid address, or non-loopback listener.
    pub fn from_env() -> Result<Self, BridgeError> {
        let listen = env::var("MODEL_ROCKET_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:0".to_owned())
            .parse::<SocketAddr>()
            .map_err(|error| {
                BridgeError::configuration(format!("invalid MODEL_ROCKET_LISTEN: {error}"))
            })?;

        let bearer = env::var("MODEL_ROCKET_BEARER")
            .map_err(|_| BridgeError::configuration("MODEL_ROCKET_BEARER is required"))?;

        let cwd = env::var_os("MODEL_ROCKET_CWD")
            .map(PathBuf::from)
            .ok_or_else(|| BridgeError::configuration("MODEL_ROCKET_CWD is required"))?;

        Self {
            listen,
            ready_file: env::var_os("MODEL_ROCKET_READY_FILE").map(PathBuf::from),
            model: DEFAULT_MODEL.to_owned(),
            codex_bin: env::var_os("MODEL_ROCKET_CODEX_BIN")
                .map_or_else(|| PathBuf::from("codex"), PathBuf::from),
            cwd,
            bearer,
        }
        .validated()
    }

    /// Validates security-sensitive configuration assembled by another caller.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-loopback listener or short local bearer.
    pub fn validated(self) -> Result<Self, BridgeError> {
        if !self.listen.ip().is_loopback() {
            return Err(BridgeError::configuration(
                "MODEL_ROCKET_LISTEN must use a loopback address",
            ));
        }
        if self.bearer.len() < 32 {
            return Err(BridgeError::configuration(
                "MODEL_ROCKET_BEARER must contain at least 32 bytes",
            ));
        }
        if self.model != DEFAULT_MODEL {
            return Err(BridgeError::configuration(format!(
                "only {DEFAULT_MODEL} is supported"
            )));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct PreflightConfig {
    pub model: String,
    pub codex_bin: PathBuf,
}

impl PreflightConfig {
    pub fn from_env() -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            codex_bin: env::var_os("MODEL_ROCKET_CODEX_BIN")
                .map_or_else(|| PathBuf::from("codex"), PathBuf::from),
        }
    }
}
