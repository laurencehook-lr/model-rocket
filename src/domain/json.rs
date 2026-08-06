use std::sync::Arc;

/// Exact JSON text validated by an inbound contract boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonDocument(Arc<str>);

impl JsonDocument {
    #[must_use]
    pub(crate) fn from_validated(document: impl Into<Arc<str>>) -> Self {
        Self(document.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact JSON text validated to contain an object at its root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonObject(Arc<str>);

impl JsonObject {
    #[must_use]
    pub(crate) fn from_validated(document: impl Into<Arc<str>>) -> Self {
        Self(document.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
