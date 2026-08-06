use axum::{
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Uri, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::value::RawValue;

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
const KEEP_ALIVE_HEADER: &str = "keep-alive";
const PROXY_CONNECTION_HEADER: &str = "proxy-connection";
const TE_HEADER: &str = "te";
const TRAILER_HEADER: &str = "trailer";
const UPGRADE_HEADER: &str = "upgrade";
const BEARER_PREFIX: &str = "Bearer ";

pub struct ReqwestRequestParts {
    pub method: Method,
    pub url: reqwest::Url,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Deserialize)]
struct ResponseMode<'a> {
    #[serde(default, borrow)]
    stream: Option<&'a RawValue>,
}

/// Reports whether an Anthropic messages request asks for an SSE response.
///
/// # Errors
///
/// Returns an error when the already-validated request body cannot be decoded.
pub fn streams_response(request: &AnthropicRequest) -> Result<bool, BridgeError> {
    let AnthropicRequestBody::Json(document) = request.body() else {
        return Ok(false);
    };
    let mode: ResponseMode<'_> = serde_json::from_str(document.as_str()).map_err(|error| {
        BridgeError::invalid_request(format!("cannot decode Anthropic response mode: {error}"))
    })?;
    Ok(mode.stream.is_some_and(|stream| stream.get() == "true"))
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
    let (method, mut url) = match request.operation() {
        AnthropicOperation::Messages => (Method::POST, upstream_url(base_url, "v1/messages")?),
        AnthropicOperation::Models => (Method::GET, upstream_url(base_url, "v1/models")?),
        AnthropicOperation::Model(model) => {
            let mut url = upstream_url(base_url, "v1/models/")?;
            url.path_segments_mut()
                .map_err(|()| {
                    BridgeError::anthropic_unavailable(
                        "Anthropic upstream URL cannot contain path segments",
                    )
                })?
                .pop_if_empty()
                .push(model.as_str());
            (Method::GET, url)
        }
    };
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
    let headers = sanitized_upstream_request_headers(&headers)?;
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

fn upstream_url(base_url: &reqwest::Url, path: &str) -> Result<reqwest::Url, BridgeError> {
    base_url.join(path).map_err(|error| {
        BridgeError::anthropic_unavailable(format!("cannot construct upstream URL: {error}"))
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
    let connection_tokens = response_connection_tokens(headers)?;
    let fields = headers
        .iter()
        .filter(|(name, _)| forward_response_header(name, &connection_tokens))
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
    let mut headers = HeaderMap::new();
    for field in head.headers().as_slice() {
        let name = HeaderName::from_bytes(field.name().as_bytes()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid Anthropic header name: {error}"))
        })?;
        let value = HeaderValue::from_bytes(field.value()).map_err(|error| {
            BridgeError::anthropic_unavailable(format!("invalid Anthropic header value: {error}"))
        })?;
        headers.append(name, value);
    }
    let headers = sanitized_downstream_response_headers(&headers)?;
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

pub(super) fn request_headers(headers: &HeaderMap) -> Result<AnthropicHeaders, BridgeError> {
    require_anthropic_oauth(headers)?;
    let connection_tokens = request_connection_tokens(headers)?;
    let fields = headers
        .iter()
        .filter(|(name, _)| forward_request_header(name, &connection_tokens))
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

fn sanitized_upstream_request_headers(headers: &HeaderMap) -> Result<HeaderMap, BridgeError> {
    let connection_tokens = response_connection_tokens(headers)?;
    let mut sanitized = HeaderMap::new();
    for (name, value) in headers {
        if forward_request_header(name, &connection_tokens) {
            sanitized.append(name.clone(), value.clone());
        }
    }
    Ok(sanitized)
}

fn sanitized_downstream_response_headers(headers: &HeaderMap) -> Result<HeaderMap, BridgeError> {
    let connection_tokens = response_connection_tokens(headers)?;
    let mut sanitized = HeaderMap::new();
    for (name, value) in headers {
        if forward_response_header(name, &connection_tokens) {
            sanitized.append(name.clone(), value.clone());
        }
    }
    Ok(sanitized)
}

fn request_connection_tokens(headers: &HeaderMap) -> Result<Vec<HeaderName>, BridgeError> {
    parse_connection_tokens(headers).map_err(|message| {
        BridgeError::invalid_request(format!("invalid Connection header: {message}"))
    })
}

fn response_connection_tokens(headers: &HeaderMap) -> Result<Vec<HeaderName>, BridgeError> {
    parse_connection_tokens(headers).map_err(|message| {
        BridgeError::anthropic_unavailable(format!("invalid upstream Connection header: {message}"))
    })
}

fn parse_connection_tokens(headers: &HeaderMap) -> Result<Vec<HeaderName>, String> {
    let mut tokens = Vec::new();
    for value in headers.get_all(header::CONNECTION) {
        let value = value
            .to_str()
            .map_err(|error| format!("value is not visible ASCII: {error}"))?;
        for token in value
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            tokens.push(
                HeaderName::from_bytes(token.as_bytes())
                    .map_err(|error| format!("invalid nominated header {token}: {error}"))?,
            );
        }
    }
    Ok(tokens)
}

fn forward_request_header(name: &HeaderName, connection_tokens: &[HeaderName]) -> bool {
    name != header::HOST
        && name != header::CONTENT_LENGTH
        && !is_hop_by_hop(name)
        && !connection_tokens.contains(name)
        && name.as_str() != LEGACY_LOCAL_AUTH_HEADER
        && name.as_str() != API_KEY_HEADER
}

fn forward_response_header(name: &HeaderName, connection_tokens: &[HeaderName]) -> bool {
    name != header::CONTENT_LENGTH
        && name != header::LOCATION
        && !is_hop_by_hop(name)
        && !connection_tokens.contains(name)
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        KEEP_ALIVE_HEADER
            | PROXY_AUTHENTICATE_HEADER
            | PROXY_AUTHORIZATION_HEADER
            | PROXY_CONNECTION_HEADER
            | TE_HEADER
            | TRAILER_HEADER
            | UPGRADE_HEADER
    ) || name == header::CONNECTION
        || name == header::TRANSFER_ENCODING
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, header},
    };

    use crate::{
        contracts::json as json_contract,
        domain::{
            AnthropicHeader, AnthropicHeaders, AnthropicOperation, AnthropicQuery,
            AnthropicRequest, AnthropicRequestBody, AnthropicStatus, RequestedModelId,
        },
    };

    use super::{
        build_reqwest_request, request_headers, response_head, response_with_body, streams_response,
    };

    #[test]
    fn response_mode_distinguishes_streaming_from_bounded_responses()
    -> Result<(), Box<dyn std::error::Error>> {
        let streaming = AnthropicRequest::new(
            AnthropicOperation::Messages,
            AnthropicQuery::Absent,
            AnthropicHeaders::default(),
            AnthropicRequestBody::Json(json_contract::document(
                r#"{"stream":true,"large_number":1e400}"#,
            )?),
        )?;
        let bounded = AnthropicRequest::new(
            AnthropicOperation::Messages,
            AnthropicQuery::Absent,
            AnthropicHeaders::default(),
            AnthropicRequestBody::Json(json_contract::document(r#"{"stream":false}"#)?),
        )?;
        let models = AnthropicRequest::new(
            AnthropicOperation::Models,
            AnthropicQuery::Absent,
            AnthropicHeaders::default(),
            AnthropicRequestBody::Empty,
        )?;

        assert!(streams_response(&streaming)?);
        assert!(!streams_response(&bounded)?);
        assert!(!streams_response(&models)?);
        Ok(())
    }

    #[test]
    fn request_filter_removes_standard_and_connection_nominated_hop_headers()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer subscription"),
        );
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("x-remove, keep-alive"),
        );
        headers.insert("x-remove", HeaderValue::from_static("secret"));
        headers.insert("x-keep", HeaderValue::from_static("visible"));
        for name in [
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
        ] {
            headers.insert(name, HeaderValue::from_static("hop"));
        }

        let filtered = request_headers(&headers)?;
        let names = filtered
            .as_slice()
            .iter()
            .map(AnthropicHeader::name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["authorization", "x-keep"]);
        Ok(())
    }

    #[test]
    fn outbound_builder_defensively_filters_semantic_hop_headers()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = AnthropicRequest::new(
            AnthropicOperation::Models,
            AnthropicQuery::Absent,
            AnthropicHeaders::new(vec![
                AnthropicHeader::new("connection", b"x-remove".as_slice())?,
                AnthropicHeader::new("x-remove", b"secret".as_slice())?,
                AnthropicHeader::new("x-keep", b"visible".as_slice())?,
            ]),
            AnthropicRequestBody::Empty,
        )?;
        let base = reqwest::Url::parse("https://api.anthropic.com/")?;

        let parts = build_reqwest_request(&base, &request)?;

        assert!(!parts.headers.contains_key(header::CONNECTION));
        assert!(!parts.headers.contains_key("x-remove"));
        assert_eq!(
            parts.headers.get("x-keep"),
            Some(&HeaderValue::from_static("visible"))
        );
        Ok(())
    }

    #[test]
    fn model_detail_url_encodes_the_model_as_one_path_segment()
    -> Result<(), Box<dyn std::error::Error>> {
        for (model, expected_path) in [
            (
                "claude-x/../../messages",
                "/v1/models/claude-x%2F..%2F..%2Fmessages",
            ),
            ("claude-x/../models", "/v1/models/claude-x%2F..%2Fmodels"),
            (
                "claude-x?redirect=messages",
                "/v1/models/claude-x%3Fredirect=messages",
            ),
            ("claude-x#fragment", "/v1/models/claude-x%23fragment"),
            (
                "claude-x%2F..%2Fmessages",
                "/v1/models/claude-x%252F..%252Fmessages",
            ),
        ] {
            let request = AnthropicRequest::new(
                AnthropicOperation::Model(RequestedModelId::from(model)),
                AnthropicQuery::Absent,
                AnthropicHeaders::default(),
                AnthropicRequestBody::Empty,
            )?;
            let base = reqwest::Url::parse("https://api.anthropic.com/")?;

            let parts = build_reqwest_request(&base, &request)?;

            assert_eq!(parts.url.host_str(), Some("api.anthropic.com"));
            assert_eq!(parts.url.path(), expected_path);
            assert_eq!(parts.url.path_segments().map(Iterator::count), Some(3));
            assert_eq!(parts.url.query(), None);
            assert_eq!(parts.url.fragment(), None);
        }
        Ok(())
    }

    #[test]
    fn response_filter_removes_standard_and_connection_nominated_hop_headers()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("x-remove, upgrade"),
        );
        headers.insert("x-remove", HeaderValue::from_static("secret"));
        headers.insert("x-keep", HeaderValue::from_static("visible"));
        for name in [
            "keep-alive",
            "proxy-authenticate",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
        ] {
            headers.insert(name, HeaderValue::from_static("hop"));
        }
        headers.insert(
            header::LOCATION,
            HeaderValue::from_static("https://attacker.invalid"),
        );

        let head = response_head(reqwest::StatusCode::OK, &headers)?;

        assert_eq!(head.status(), AnthropicStatus::new(200)?);
        let fields = head.headers().as_slice();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields.first().map(AnthropicHeader::name), Some("x-keep"));
        Ok(())
    }

    #[test]
    fn downstream_builder_defensively_filters_semantic_hop_headers()
    -> Result<(), Box<dyn std::error::Error>> {
        let head = crate::domain::AnthropicResponseHead::new(
            AnthropicStatus::new(200)?,
            AnthropicHeaders::new(vec![
                AnthropicHeader::new("connection", b"x-remove".as_slice())?,
                AnthropicHeader::new("x-remove", b"secret".as_slice())?,
                AnthropicHeader::new("upgrade", b"websocket".as_slice())?,
                AnthropicHeader::new("x-keep", b"visible".as_slice())?,
            ]),
        );

        let response = response_with_body(&head, Body::empty())?;

        assert!(!response.headers().contains_key(header::CONNECTION));
        assert!(!response.headers().contains_key("x-remove"));
        assert!(!response.headers().contains_key(header::UPGRADE));
        assert_eq!(
            response.headers().get("x-keep"),
            Some(&HeaderValue::from_static("visible"))
        );
        Ok(())
    }
}
