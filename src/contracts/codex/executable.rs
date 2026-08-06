use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    domain::executable::ExecutableProvenance,
    domain::{BridgeError, ValidatedCodexExecutable},
    product::CODEX_NATIVE_SHA256,
};

pub(crate) fn validate_native(path: &Path) -> Result<ValidatedCodexExecutable, BridgeError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot resolve native Codex binary {}: {error}",
            path.display()
        ))
    })?;
    validate_native_path(&canonical)?;
    validated_capability(&canonical)
}

pub(crate) fn revalidate(executable: &ValidatedCodexExecutable) -> Result<PathBuf, BridgeError> {
    let path = PathBuf::from(executable.as_str());
    match executable.provenance() {
        ExecutableProvenance::PinnedNative => validate_native_path(&path)?,
        #[cfg(feature = "test-support")]
        ExecutableProvenance::TestFixture => validate_test_fixture_path(&path)?,
    }
    Ok(path)
}

fn validate_native_path(path: &Path) -> Result<(), BridgeError> {
    let metadata = executable_metadata(path, "native Codex binary")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BridgeError::configuration(format!(
            "Codex binary must be a native executable file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot read native Codex binary {}: {error}",
            path.display()
        ))
    })?;
    let native_magic_matches = if cfg!(target_os = "macos") {
        bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
    } else {
        bytes.starts_with(b"\x7fELF")
    };
    if !native_magic_matches {
        return Err(BridgeError::configuration(format!(
            "Codex binary must be the native executable, not a script or wrapper: {}",
            path.display()
        )));
    }
    let actual_digest = format!("{:x}", Sha256::digest(&bytes));
    if actual_digest != CODEX_NATIVE_SHA256 {
        return Err(BridgeError::configuration(format!(
            "Codex native executable digest does not match the pinned release at {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(feature = "test-support")]
pub(crate) fn validate_test_fixture(path: &Path) -> Result<ValidatedCodexExecutable, BridgeError> {
    if !path.is_absolute() {
        return Err(BridgeError::configuration(
            "test Codex fixture must use an absolute path",
        ));
    }
    let metadata = executable_metadata(path, "test Codex fixture")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BridgeError::configuration(format!(
            "test Codex fixture must be executable: {}",
            path.display()
        )));
    }
    validated_fixture_capability(path)
}

fn executable_metadata(path: &Path, description: &str) -> Result<fs::Metadata, BridgeError> {
    fs::metadata(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        ))
    })
}

fn validated_capability(path: &Path) -> Result<ValidatedCodexExecutable, BridgeError> {
    let value = path.to_str().ok_or_else(|| {
        BridgeError::configuration("Codex executable path must contain valid UTF-8")
    })?;
    Ok(ValidatedCodexExecutable::from_validated_native(value))
}

#[cfg(feature = "test-support")]
fn validate_test_fixture_path(path: &Path) -> Result<(), BridgeError> {
    if !path.is_absolute() {
        return Err(BridgeError::configuration(
            "test Codex fixture must use an absolute path",
        ));
    }
    let metadata = executable_metadata(path, "test Codex fixture")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BridgeError::configuration(format!(
            "test Codex fixture must be executable: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(feature = "test-support")]
fn validated_fixture_capability(path: &Path) -> Result<ValidatedCodexExecutable, BridgeError> {
    let value = path.to_str().ok_or_else(|| {
        BridgeError::configuration("Codex executable path must contain valid UTF-8")
    })?;
    Ok(ValidatedCodexExecutable::from_validated_test_fixture(value))
}
