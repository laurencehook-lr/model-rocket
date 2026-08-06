use std::{
    env,
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use crate::contracts::codex::executable;
use crate::domain::{BridgeError, ModelCatalogue, ValidatedCodexExecutable};

mod model_catalogue;

#[derive(Clone, Debug)]
pub struct Config {
    listen: SocketAddr,
    ready_file: Option<PathBuf>,
    codex_executable: ValidatedCodexExecutable,
    cwd: PathBuf,
    bearer: String,
    catalogue: std::sync::Arc<ModelCatalogue>,
}

impl Config {
    /// Loads and validates the serving configuration from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing bearer, invalid address, or non-loopback listener.
    pub fn from_env() -> Result<Self, BridgeError> {
        let listen = match env::var("MODEL_ROCKET_LISTEN") {
            Ok(listen) => listen,
            Err(env::VarError::NotPresent) => "127.0.0.1:0".to_owned(),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(BridgeError::configuration(
                    "MODEL_ROCKET_LISTEN must contain valid UTF-8",
                ));
            }
        }
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
            codex_executable: required_codex_binary()?,
            cwd,
            bearer,
            catalogue: model_catalogue_from_env()?,
        }
        .validate()
    }

    fn validate(self) -> Result<Self, BridgeError> {
        if !self.listen.ip().is_loopback() {
            return Err(BridgeError::configuration(
                "MODEL_ROCKET_LISTEN must use a loopback address",
            ));
        }
        if self.bearer.len() < 32 || !self.bearer.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(BridgeError::configuration(
                "MODEL_ROCKET_BEARER must contain at least 32 bytes of printable ASCII",
            ));
        }
        if self.cwd.to_str().is_none() {
            return Err(BridgeError::configuration(
                "MODEL_ROCKET_CWD must contain valid UTF-8",
            ));
        }
        Ok(self)
    }

    #[must_use]
    pub const fn listen(&self) -> SocketAddr {
        self.listen
    }

    #[must_use]
    pub const fn ready_file(&self) -> Option<&PathBuf> {
        self.ready_file.as_ref()
    }

    #[must_use]
    pub const fn codex_executable(&self) -> &ValidatedCodexExecutable {
        &self.codex_executable
    }

    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.cwd
    }

    #[must_use]
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    #[must_use]
    pub const fn catalogue(&self) -> &std::sync::Arc<ModelCatalogue> {
        &self.catalogue
    }

    /// Creates an explicitly untrusted fixture configuration for integration tests.
    ///
    /// This constructor is absent from production builds.
    ///
    /// # Errors
    ///
    /// Returns an error when the listener, bearer, path, or working directory is invalid.
    #[cfg(feature = "test-support")]
    pub fn test_fixture(
        listen: SocketAddr,
        ready_file: Option<PathBuf>,
        codex_bin: &Path,
        cwd: PathBuf,
        bearer: String,
    ) -> Result<Self, BridgeError> {
        Self::test_fixture_with_catalogue(
            listen,
            ready_file,
            codex_bin,
            cwd,
            bearer,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/model-routes.json"),
        )
    }

    /// Creates an explicitly untrusted fixture configuration with an explicit catalogue.
    ///
    /// This constructor is absent from production builds.
    ///
    /// # Errors
    ///
    /// Returns an error when the listener, bearer, executable, working directory, or catalogue is invalid.
    #[cfg(feature = "test-support")]
    pub fn test_fixture_with_catalogue(
        listen: SocketAddr,
        ready_file: Option<PathBuf>,
        codex_bin: &Path,
        cwd: PathBuf,
        bearer: String,
        catalogue_path: &Path,
    ) -> Result<Self, BridgeError> {
        let codex_executable = executable::validate_test_fixture(codex_bin)?;
        Self {
            listen,
            ready_file,
            codex_executable,
            cwd,
            bearer,
            catalogue: model_catalogue::load(catalogue_path)?,
        }
        .validate()
    }
}

#[derive(Clone, Debug)]
pub struct PreflightConfig {
    codex_executable: ValidatedCodexExecutable,
    catalogue: std::sync::Arc<ModelCatalogue>,
}

impl PreflightConfig {
    /// Loads the preflight configuration from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error unless the configured or home-relative Codex path is absolute.
    pub fn from_env() -> Result<Self, BridgeError> {
        Ok(Self {
            codex_executable: required_codex_binary()?,
            catalogue: model_catalogue_from_env()?,
        })
    }

    #[must_use]
    pub const fn catalogue(&self) -> &std::sync::Arc<ModelCatalogue> {
        &self.catalogue
    }

    #[must_use]
    pub const fn codex_executable(&self) -> &ValidatedCodexExecutable {
        &self.codex_executable
    }

    /// Creates a preflight configuration using the pinned executable environment and checked-in catalogue.
    ///
    /// # Errors
    ///
    /// Returns an error when the executable or checked-in catalogue is invalid.
    #[cfg(feature = "test-support")]
    pub fn test_fixture_from_env() -> Result<Self, BridgeError> {
        Ok(Self {
            codex_executable: required_codex_binary()?,
            catalogue: model_catalogue::load(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/model-routes.json"),
            )?,
        })
    }

    /// Creates a preflight configuration from explicit test fixtures.
    ///
    /// # Errors
    ///
    /// Returns an error when the executable or catalogue fixture is invalid.
    #[cfg(feature = "test-support")]
    pub fn test_fixture(codex_bin: &Path, catalogue_path: &Path) -> Result<Self, BridgeError> {
        Ok(Self {
            codex_executable: executable::validate_test_fixture(codex_bin)?,
            catalogue: model_catalogue::load(catalogue_path)?,
        })
    }
}

/// Loads the launch-scoped model catalogue selected by the environment.
///
/// # Errors
///
/// Returns an error when the configured path is not absolute or the catalogue is invalid.
pub fn model_catalogue_from_env() -> Result<std::sync::Arc<ModelCatalogue>, BridgeError> {
    let path = model_catalogue_path(
        env::var_os("MODEL_ROCKET_CONFIG"),
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
    )?;
    model_catalogue::load(&path)
}

fn model_catalogue_path(
    configured: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, BridgeError> {
    let path = if let Some(configured) = configured {
        PathBuf::from(configured)
    } else if let Some(xdg_config_home) = xdg_config_home {
        PathBuf::from(xdg_config_home).join("model-rocket/model-routes.json")
    } else {
        PathBuf::from(home.ok_or_else(|| {
            BridgeError::configuration(
                "HOME is required when MODEL_ROCKET_CONFIG and XDG_CONFIG_HOME are not set",
            )
        })?)
        .join(".config/model-rocket/model-routes.json")
    };
    if !path.is_absolute() {
        return Err(BridgeError::configuration(
            "MODEL_ROCKET_CONFIG must be an absolute path",
        ));
    }
    Ok(path)
}

fn required_codex_binary() -> Result<ValidatedCodexExecutable, BridgeError> {
    let path = codex_binary_path(env::var_os("MODEL_ROCKET_CODEX_BIN"), env::var_os("HOME"))?;
    executable::validate_native(&path)
}

fn codex_binary_path(
    configured: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, BridgeError> {
    let path = if let Some(configured) = configured {
        PathBuf::from(configured)
    } else {
        PathBuf::from(home.ok_or_else(|| {
            BridgeError::configuration("HOME is required when MODEL_ROCKET_CODEX_BIN is not set")
        })?)
        .join(".local/bin/codex-native")
    };
    if !path.is_absolute() {
        return Err(BridgeError::configuration(
            "MODEL_ROCKET_CODEX_BIN must be an absolute path",
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{codex_binary_path, model_catalogue_path};
    use crate::contracts::codex::executable;

    #[test]
    fn documented_preflight_uses_stable_absolute_codex_default()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            codex_binary_path(None, Some(OsString::from("/Users/tester")))?,
            PathBuf::from("/Users/tester/.local/bin/codex-native")
        );
        Ok(())
    }

    #[test]
    fn configured_codex_path_must_remain_absolute() {
        assert!(codex_binary_path(Some(OsString::from("codex")), None).is_err());
    }

    #[test]
    fn configured_model_catalogue_path_must_remain_absolute() {
        assert!(
            model_catalogue_path(Some(OsString::from("model-routes.json")), None, None).is_err()
        );
    }

    #[test]
    fn npm_script_wrapper_is_not_a_native_codex_executable()
    -> Result<(), Box<dyn std::error::Error>> {
        let wrapper =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_codex.py");
        let error = executable::validate_native(&wrapper)
            .err()
            .ok_or_else(|| std::io::Error::other("script fixture must be rejected"))?;
        assert!(error.to_string().contains("not a script or wrapper"));
        Ok(())
    }
}
