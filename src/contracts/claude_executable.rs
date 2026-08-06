use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{domain::BridgeError, product::CLAUDE_NATIVE_SHA256};

/// Validates the exact official Claude Code native executable pinned by this release.
///
/// # Errors
///
/// Returns a configuration error for an unresolved path, a non-native executable, or a digest
/// that differs from the target-specific pinned artifact.
pub fn validate(path: &Path) -> Result<PathBuf, BridgeError> {
    if !path.is_absolute() {
        return Err(BridgeError::configuration(
            "Claude Code executable must use an absolute path",
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot resolve Claude Code executable {}: {error}",
            path.display()
        ))
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect Claude Code executable {}: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BridgeError::configuration(format!(
            "Claude Code executable must be an executable file: {}",
            canonical.display()
        )));
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot read Claude Code executable {}: {error}",
            canonical.display()
        ))
    })?;
    let native_magic_matches = if cfg!(target_os = "macos") {
        bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
    } else {
        bytes.starts_with(b"\x7fELF")
    };
    if !native_magic_matches {
        return Err(BridgeError::configuration(format!(
            "Claude Code executable must be the native release, not a script or wrapper: {}",
            canonical.display()
        )));
    }
    let actual_digest = format!("{:x}", Sha256::digest(&bytes));
    if actual_digest != CLAUDE_NATIVE_SHA256 {
        return Err(BridgeError::configuration(format!(
            "Claude Code executable digest does not match the pinned release at {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}
