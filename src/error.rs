use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("authentication failed")]
    Authentication,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unsupported request: {0}")]
    Unsupported(String),
    #[error("request body exceeds 8 MiB")]
    PayloadTooLarge,
    #[error("App Server unavailable: {0}")]
    AppServerUnavailable(String),
    #[error("App Server protocol error: {0}")]
    AppServerProtocol(String),
    #[error("Anthropic upstream unavailable: {0}")]
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

    fn status(&self) -> StatusCode {
        match self {
            Self::Authentication => StatusCode::UNAUTHORIZED,
            Self::InvalidRequest(_) | Self::Unsupported(_) | Self::Configuration(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::AppServerUnavailable(_)
            | Self::AppServerProtocol(_)
            | Self::AnthropicUnavailable(_) => StatusCode::BAD_GATEWAY,
        }
    }

    fn error_type(&self) -> &'static str {
        match self {
            Self::Authentication => "authentication_error",
            Self::InvalidRequest(_) | Self::Configuration(_) | Self::PayloadTooLarge => {
                "invalid_request_error"
            }
            Self::Unsupported(_) => "unsupported_capability_error",
            Self::AppServerUnavailable(_) | Self::AnthropicUnavailable(_) => "api_error",
            Self::AppServerProtocol(_) => "protocol_error",
        }
    }
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    r#type: &'static str,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    r#type: &'static str,
    message: &'a str,
}

impl IntoResponse for BridgeError {
    fn into_response(self) -> Response {
        let status = self.status();
        let error_type = self.error_type();
        let message = self.to_string();
        (
            status,
            Json(ErrorEnvelope {
                r#type: "error",
                error: ErrorBody {
                    r#type: error_type,
                    message: &message,
                },
            }),
        )
            .into_response()
    }
}
