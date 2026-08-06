use std::fmt;

#[derive(Clone, Debug)]
pub enum BridgeError {
    Configuration(String),
    Authentication,
    InvalidRequest(String),
    Unsupported(String),
    PayloadTooLarge,
    AppServerUnavailable(String),
    AppServerProtocol(String),
    AnthropicUnavailable(String),
}

impl BridgeError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::AppServerUnavailable(message.into())
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::AppServerProtocol(message.into())
    }

    pub fn anthropic_unavailable(message: impl Into<String>) -> Self {
        Self::AnthropicUnavailable(message.into())
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => write!(formatter, "configuration error: {message}"),
            Self::Authentication => formatter.write_str("authentication failed"),
            Self::InvalidRequest(message) => write!(formatter, "invalid request: {message}"),
            Self::Unsupported(message) => write!(formatter, "unsupported request: {message}"),
            Self::PayloadTooLarge => {
                formatter.write_str("request body exceeds the configured limit")
            }
            Self::AppServerUnavailable(message) => {
                write!(formatter, "App Server unavailable: {message}")
            }
            Self::AppServerProtocol(message) => {
                write!(formatter, "App Server protocol error: {message}")
            }
            Self::AnthropicUnavailable(message) => {
                write!(formatter, "Anthropic upstream unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for BridgeError {}
