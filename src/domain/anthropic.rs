use std::{fmt, sync::Arc};

use super::{JsonDocument, RequestedModelId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnthropicOperation {
    Messages,
    Models,
    Model(RequestedModelId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AnthropicQuery {
    #[default]
    Absent,
    Present(Arc<str>),
}

impl AnthropicQuery {
    #[must_use]
    pub fn from_optional(value: Option<&str>) -> Self {
        value.map_or(Self::Absent, |query| Self::Present(Arc::from(query)))
    }

    #[must_use]
    pub fn as_deref(&self) -> Option<&str> {
        match self {
            Self::Absent => None,
            Self::Present(value) => Some(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnthropicHeader {
    name: Arc<str>,
    value: Arc<[u8]>,
}

impl AnthropicHeader {
    /// Creates an HTTP header with validated name and value bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the name or value contains forbidden bytes.
    pub fn new(
        name: impl Into<Arc<str>>,
        value: impl Into<Arc<[u8]>>,
    ) -> Result<Self, HeaderError> {
        let name = name.into();
        let value = value.into();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(HeaderError::InvalidName);
        }
        if value
            .iter()
            .any(|byte| (*byte < b' ' && *byte != b'\t') || *byte == b'\x7f')
        {
            return Err(HeaderError::InvalidValue);
        }
        Ok(Self { name, value })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnthropicHeaders(Arc<[AnthropicHeader]>);

impl AnthropicHeaders {
    #[must_use]
    pub fn new(values: impl Into<Arc<[AnthropicHeader]>>) -> Self {
        Self(values.into())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[AnthropicHeader] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnthropicRequestBody {
    Empty,
    Json(JsonDocument),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnthropicRequest {
    operation: AnthropicOperation,
    query: AnthropicQuery,
    headers: AnthropicHeaders,
    body: AnthropicRequestBody,
}

impl AnthropicRequest {
    /// Creates an operation whose body shape matches the requested Anthropic endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the operation and body are inconsistent.
    pub fn new(
        operation: AnthropicOperation,
        query: AnthropicQuery,
        headers: AnthropicHeaders,
        body: AnthropicRequestBody,
    ) -> Result<Self, AnthropicRequestError> {
        let valid_body = matches!(
            (&operation, &body),
            (AnthropicOperation::Messages, AnthropicRequestBody::Json(_))
                | (
                    AnthropicOperation::Models | AnthropicOperation::Model(_),
                    AnthropicRequestBody::Empty
                )
        );
        if !valid_body {
            return Err(AnthropicRequestError);
        }
        Ok(Self {
            operation,
            query,
            headers,
            body,
        })
    }

    #[must_use]
    pub fn operation(&self) -> &AnthropicOperation {
        &self.operation
    }

    #[must_use]
    pub fn query(&self) -> &AnthropicQuery {
        &self.query
    }

    #[must_use]
    pub fn headers(&self) -> &AnthropicHeaders {
        &self.headers
    }

    #[must_use]
    pub fn body(&self) -> &AnthropicRequestBody {
        &self.body
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnthropicStatus(u16);

impl AnthropicStatus {
    /// Creates a valid HTTP response status.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is outside the HTTP status range.
    pub fn new(value: u16) -> Result<Self, AnthropicStatusError> {
        if (100..=599).contains(&value) {
            Ok(Self(value))
        } else {
            Err(AnthropicStatusError)
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn is_redirect(self) -> bool {
        self.0 >= 300 && self.0 < 400
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnthropicResponseHead {
    status: AnthropicStatus,
    headers: AnthropicHeaders,
}

impl AnthropicResponseHead {
    #[must_use]
    pub const fn new(status: AnthropicStatus, headers: AnthropicHeaders) -> Self {
        Self { status, headers }
    }

    #[must_use]
    pub const fn status(&self) -> AnthropicStatus {
        self.status
    }

    #[must_use]
    pub const fn headers(&self) -> &AnthropicHeaders {
        &self.headers
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnthropicResponseChunk(Arc<[u8]>);

impl AnthropicResponseChunk {
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderError {
    InvalidName,
    InvalidValue,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => formatter.write_str("header name is invalid"),
            Self::InvalidValue => formatter.write_str("header value is invalid"),
        }
    }
}

impl std::error::Error for HeaderError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnthropicRequestError;

impl fmt::Display for AnthropicRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Anthropic operation and request body are inconsistent")
    }
}

impl std::error::Error for AnthropicRequestError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnthropicStatusError;

impl fmt::Display for AnthropicStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HTTP status is outside the valid range")
    }
}

impl std::error::Error for AnthropicStatusError {}

#[cfg(test)]
mod tests {
    use super::AnthropicHeader;

    #[test]
    fn header_value_allows_horizontal_tab_but_rejects_c0_and_del() {
        assert!(AnthropicHeader::new("x-test", b"value\tvalue".as_slice()).is_ok());
        for byte in 0_u8..=0x1f {
            if byte != b'\t' {
                assert!(AnthropicHeader::new("x-test", vec![byte]).is_err());
            }
        }
        assert!(AnthropicHeader::new("x-test", vec![0x7f]).is_err());
    }
}
