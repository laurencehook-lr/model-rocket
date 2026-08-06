use std::{
    fs::{self, File},
    io::{BufReader, Read},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    domain::executable::ExecutableProvenance,
    domain::{BridgeError, ValidatedCodexExecutable},
    product::CODEX_NATIVE_SHA256,
};

const MAX_CODEX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const EXECUTABLE_HASH_BUFFER_BYTES: usize = 64 * 1024;
const NATIVE_HEADER_BYTES: usize = 4;
const MACH_O_64_HEADER: [u8; NATIVE_HEADER_BYTES] = [0xcf, 0xfa, 0xed, 0xfe];
const ELF_HEADER: [u8; NATIVE_HEADER_BYTES] = *b"\x7fELF";

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
    let file = executable_file(
        path,
        "native Codex binary",
        "Codex binary must be a native executable file",
    )?;
    let actual_digest = native_digest(file, path)?;
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
    validate_test_fixture_path(path)?;
    validated_fixture_capability(path)
}

fn executable_file(
    path: &Path,
    description: &str,
    invalid_file_message: &str,
) -> Result<File, BridgeError> {
    let file = File::open(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BridgeError::configuration(format!(
            "{invalid_file_message}: {}",
            path.display()
        )));
    }
    Ok(file)
}

fn native_digest(file: File, path: &Path) -> Result<String, BridgeError> {
    let metadata = file.metadata().map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect native Codex binary {}: {error}",
            path.display()
        ))
    })?;
    if metadata.len() > MAX_CODEX_EXECUTABLE_BYTES {
        return Err(BridgeError::configuration(format!(
            "Codex native executable exceeds {MAX_CODEX_EXECUTABLE_BYTES} bytes at {}",
            path.display()
        )));
    }
    let mut reader = BufReader::with_capacity(EXECUTABLE_HASH_BUFFER_BYTES, file);
    let mut header = [0_u8; NATIVE_HEADER_BYTES];
    reader.read_exact(&mut header).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot read native Codex binary {}: {error}",
            path.display()
        ))
    })?;
    let native_magic_matches = if cfg!(target_os = "macos") {
        header == MACH_O_64_HEADER
    } else {
        header == ELF_HEADER
    };
    if !native_magic_matches {
        return Err(BridgeError::configuration(format!(
            "Codex binary must be the native executable, not a script or wrapper: {}",
            path.display()
        )));
    }
    let mut hasher = Sha256::new();
    hasher.update(header);
    let mut total_bytes = u64::try_from(header.len()).map_err(|error| {
        BridgeError::configuration(format!("native Codex header length is invalid: {error}"))
    })?;
    let mut buffer = vec![0_u8; EXECUTABLE_HASH_BUFFER_BYTES].into_boxed_slice();
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            BridgeError::configuration(format!(
                "cannot read native Codex binary {}: {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes
            .checked_add(u64::try_from(read).map_err(|error| {
                BridgeError::configuration(format!("native Codex read length is invalid: {error}"))
            })?)
            .ok_or_else(|| BridgeError::configuration("native Codex byte count overflowed"))?;
        if total_bytes > MAX_CODEX_EXECUTABLE_BYTES {
            return Err(BridgeError::configuration(format!(
                "Codex native executable exceeds {MAX_CODEX_EXECUTABLE_BYTES} bytes at {}",
                path.display()
            )));
        }
        let chunk = buffer.get(..read).ok_or_else(|| {
            BridgeError::configuration("native Codex read exceeded the hashing buffer")
        })?;
        hasher.update(chunk);
    }
    Ok(format!("{:x}", hasher.finalize()))
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
    executable_file(
        path,
        "test Codex fixture",
        "test Codex fixture must be executable",
    )?;
    Ok(())
}

#[cfg(feature = "test-support")]
fn validated_fixture_capability(path: &Path) -> Result<ValidatedCodexExecutable, BridgeError> {
    let value = path.to_str().ok_or_else(|| {
        BridgeError::configuration("Codex executable path must contain valid UTF-8")
    })?;
    Ok(ValidatedCodexExecutable::from_validated_test_fixture(value))
}

#[cfg(test)]
mod tests {
    use std::{fs::File, path::PathBuf};

    use super::{MAX_CODEX_EXECUTABLE_BYTES, native_digest};

    struct TemporaryFile(PathBuf);

    impl TemporaryFile {
        fn sparse(size: u64) -> Result<Self, Box<dyn std::error::Error>> {
            let path = std::env::temp_dir().join(format!(
                "model-rocket-executable-bound-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            File::create(&path)?.set_len(size)?;
            Ok(Self(path))
        }
    }

    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            let _removed = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn native_digest_rejects_files_over_the_streaming_bound_before_reading()
    -> Result<(), Box<dyn std::error::Error>> {
        let oversized = TemporaryFile::sparse(MAX_CODEX_EXECUTABLE_BYTES + 1)?;
        let file = File::open(&oversized.0)?;
        let error = native_digest(file, &oversized.0)
            .err()
            .ok_or("oversized native executable was accepted")?;
        assert!(error.to_string().contains("exceeds"));
        Ok(())
    }
}
