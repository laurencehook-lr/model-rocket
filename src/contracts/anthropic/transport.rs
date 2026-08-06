use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Uri, header},
    response::Response,
};

use crate::{
    domain::BridgeError,
    domain::{
        AnthropicHeader, AnthropicHeaders, AnthropicOperation, AnthropicQuery, AnthropicRequest,
        AnthropicRequestBody, AnthropicResponseHead, AnthropicStatus,
    },
};

pub const HEALTH_ROUTE: &str = "/healthz";
pub const MESSAGES_ROUTE: &str = "/{token}/v1/messages";
pub const MODELS_ROUTE: &str = "/{token}/v1/models";
pub const MODEL_ROUTE: &str = "/{token}/v1/models/{model}";
pub const CLAUDE_SESSION_HEADER: &str = "x-claude-code-session-id";
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/";

const LEGACY_LOCAL_AUTH_HEADER: &str = "x-model-rocket-token";
const API_KEY_HEADER: &str = "x-api-key";
const PROXY_AUTHORIZATION_HEADER: &str = "proxy-authorization";
const PROXY_AUTHENTICATE_HEADER: &str = "proxy-authenticate";
const BEARER_PREFIX: &str = "Bearer ";

pub struct ReqwestRequestParts {
    pub method: Method,
    pub url: reqwest::Url,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// Converts a validated Anthropic request into reqwest transport parts.
///
/// # Errors
///
/// Returns an error when the endpoint, headers, or body cannot be represented safely.
pub fn build_reqwest_request(
    base_url: &reqwest::Url,
    request: &AnthropicRequest,
) -> Result<ReqwestRequestParts, BridgeError> {
    let (method, path) = match request.operation() {
        AnthropicOperation::Messages => (Method::POST, "v1/messages".to_owned()),
        AnthropicOperation::Models => (Method::GET, "v1/models".to_owned()),
        AnthropicOperation::Model(model) => (Method::GET, format!("v1/models/{}", model.as_str())),
    };
    let mut url = base_url.join(&path).map_err(|error| {
        BridgeError::anthropic_unavailable(format!("cannot construct upstream URL: {error}"))
    })?;
    url.set_query(request.query().as_deref());
    let mut headers = HeaderMap::new();
    for field in request.headers().as_slice() {
        let name = HeaderName::from_bytes(field.name().as_bytes()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid upstream header name: {error}"))
        })?;
        let value = HeaderValue::from_bytes(field.value()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid upstream header value: {error}"))
        })?;
        headers.append(name, value);
    }
    let body = match request.body() {
        AnthropicRequestBody::Empty => Bytes::new(),
        AnthropicRequestBody::Json(document) => {
            Bytes::copy_from_slice(document.as_str().as_bytes())
        }
    };
    Ok(ReqwestRequestParts {
        method,
        url,
        headers,
        body,
    })
}

/// Converts reqwest response metadata into the transport-independent response head.
///
/// # Errors
///
/// Returns an error when the status or forwarded headers violate the domain contract.
pub fn response_head(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> Result<AnthropicResponseHead, BridgeError> {
    let status = AnthropicStatus::new(status.as_u16()).map_err(|error| {
        BridgeError::anthropic_unavailable(format!("invalid upstream status: {error}"))
    })?;
    let fields = headers
        .iter()
        .filter(|(name, _)| forward_response_header(name.as_str()))
        .map(|(name, value)| {
            AnthropicHeader::new(name.as_str(), value.as_bytes()).map_err(|error| {
                BridgeError::anthropic_unavailable(format!("invalid upstream header: {error}"))
            })
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    Ok(AnthropicResponseHead::new(
        status,
        AnthropicHeaders::new(fields),
    ))
}

/// Converts a validated response head and body stream into an Axum response.
///
/// # Errors
///
/// Returns an error when status or header values cannot be represented by Axum.
pub fn response_with_body(
    head: &AnthropicResponseHead,
    body: Body,
) -> Result<Response, BridgeError> {
    let status = axum::http::StatusCode::from_u16(head.status().get()).map_err(|error| {
        BridgeError::anthropic_unavailable(format!("invalid Anthropic status: {error}"))
    })?;
    let mut response = Response::new(body);
    *response.status_mut() = status;
    for field in head.headers().as_slice() {
        let name = HeaderName::from_bytes(field.name().as_bytes()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid Anthropic header name: {error}"))
        })?;
        let value = HeaderValue::from_bytes(field.value()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid Anthropic header value: {error}"))
        })?;
        response.headers_mut().append(name, value);
    }
    Ok(response)
}

pub(super) fn request_headers(headers: &HeaderMap) -> Result<AnthropicHeaders, BridgeError> {
    require_anthropic_oauth(headers)?;
    let fields = headers
        .iter()
        .filter(|(name, _)| forward_request_header(name))
        .map(|(name, value)| {
            AnthropicHeader::new(name.as_str(), value.as_bytes()).map_err(|error| {
                BridgeError::invalid_request(format!("invalid request header: {error}"))
            })
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    Ok(AnthropicHeaders::new(fields))
}

pub(super) fn query(uri: &Uri) -> AnthropicQuery {
    AnthropicQuery::from_optional(uri.query())
}

fn require_anthropic_oauth(headers: &HeaderMap) -> Result<(), BridgeError> {
    let oauth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(BEARER_PREFIX))
        .is_some_and(|value| !value.is_empty());
    if oauth && !headers.contains_key(API_KEY_HEADER) {
        Ok(())
    } else {
        Err(BridgeError::Authentication)
    }
}

fn forward_request_header(name: &HeaderName) -> bool {
    name != header::HOST
        && name != header::CONTENT_LENGTH
        && name != header::CONNECTION
        && name != header::TRANSFER_ENCODING
        && name.as_str() != LEGACY_LOCAL_AUTH_HEADER
        && name.as_str() != API_KEY_HEADER
        && name.as_str() != PROXY_AUTHORIZATION_HEADER
        && name.as_str() != PROXY_AUTHENTICATE_HEADER
}

fn forward_response_header(name: &str) -> bool {
    name != header::CONTENT_LENGTH.as_str()
        && name != header::CONNECTION.as_str()
        && name != header::LOCATION.as_str()
        && name != header::TRANSFER_ENCODING.as_str()
        && name != PROXY_AUTHORIZATION_HEADER
        && name != PROXY_AUTHENTICATE_HEADER
}
