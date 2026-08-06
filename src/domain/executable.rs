use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutableProvenance {
    PinnedNative,
    #[cfg(feature = "test-support")]
    TestFixture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCodexExecutable {
    path: Arc<str>,
    provenance: ExecutableProvenance,
}

impl ValidatedCodexExecutable {
    #[must_use]
    pub(crate) fn from_validated_native(path: impl Into<Arc<str>>) -> Self {
        Self {
            path: path.into(),
            provenance: ExecutableProvenance::PinnedNative,
        }
    }

    #[cfg(feature = "test-support")]
    #[must_use]
    pub(crate) fn from_validated_test_fixture(path: impl Into<Arc<str>>) -> Self {
        Self {
            path: path.into(),
            provenance: ExecutableProvenance::TestFixture,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub(crate) const fn provenance(&self) -> ExecutableProvenance {
        self.provenance
    }
}
