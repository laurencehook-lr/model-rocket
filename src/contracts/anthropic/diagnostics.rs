use crate::domain::BridgeError;

#[must_use]
pub fn channel_closed() -> BridgeError {
    BridgeError::unavailable("generation task ended without an outcome")
}

#[must_use]
pub fn response_head_missing() -> BridgeError {
    BridgeError::anthropic_unavailable("Anthropic request ended without response headers")
}

#[must_use]
pub fn routed_response_mismatch() -> BridgeError {
    BridgeError::protocol("application response does not match its routed response head")
}

pub fn body_stream_ended(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::anthropic_unavailable(format!("Anthropic response stream failed: {error}"))
}

pub fn header_conversion_failed(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::anthropic_unavailable(format!("invalid Anthropic response header: {error}"))
}

pub fn invalid_status(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::anthropic_unavailable(format!("invalid Anthropic response status: {error}"))
}

pub fn client_build_failed(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::configuration(format!("cannot build Anthropic HTTP client: {error}"))
}

pub fn invalid_base_url(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::configuration(format!("invalid Anthropic base URL: {error}"))
}

pub fn request_failed(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::anthropic_unavailable(format!("request failed: {error}"))
}

#[must_use]
pub fn redirect_rejected() -> BridgeError {
    BridgeError::anthropic_unavailable("upstream redirect was rejected")
}
