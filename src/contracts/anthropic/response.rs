use std::collections::VecDeque;

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, header},
    response::{Response, sse::Event},
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    contracts::json as json_contract,
    domain::BridgeError,
    domain::{AnthropicResponseChunk, AssistantOutcome, CompletionCause, TokenUsage},
    policies::http_limits::MAX_NON_STREAM_RESPONSE_BYTES,
};

const FIRST_CONTENT_BLOCK: usize = 0;
const JSON_CONTENT_TYPE: &str = "application/json";

pub struct SseFrame {
    event: &'static str,
    data: String,
}

impl SseFrame {
    fn json(event: &'static str, value: &Value) -> Result<Self, BridgeError> {
        let data = serde_json::to_string(value)
            .map_err(|error| BridgeError::protocol(format!("cannot encode SSE event: {error}")))?;
        Ok(Self { event, data })
    }

    pub fn into_event(self) -> Event {
        Event::default().event(self.event).data(self.data)
    }
}

pub struct ResponseSequence {
    queue: VecDeque<SseFrame>,
    text_started: bool,
}

impl ResponseSequence {
    /// Starts an Anthropic-compatible streaming response sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when the initial event cannot be encoded.
    pub fn new(model: &str) -> Result<Self, BridgeError> {
        let initial = SseFrame::json(
            "message_start",
            &json!({
                "type": "message_start",
                "message": {
                    "id": format!("msg_{}", uuid::Uuid::now_v7().simple()),
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0}
                }
            }),
        )?;
        Ok(Self {
            queue: VecDeque::from([initial]),
            text_started: false,
        })
    }

    /// Appends one assistant text delta to the response sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when an SSE frame cannot be encoded.
    pub fn accept_delta(&mut self, text: &str) -> Result<(), BridgeError> {
        if !self.text_started {
            self.queue.push_back(text_block_start()?);
            self.text_started = true;
        }
        self.queue.push_back(SseFrame::json(
            "content_block_delta",
            &json!({
                "type": "content_block_delta",
                "index": FIRST_CONTENT_BLOCK,
                "delta": {"type": "text_delta", "text": text}
            }),
        )?);
        Ok(())
    }

    /// Completes the response sequence with a text or tool outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when a terminal SSE frame cannot be encoded.
    pub fn finish(&mut self, outcome: AssistantOutcome) -> Result<(), BridgeError> {
        let (stop_reason, usage) = match outcome {
            AssistantOutcome::Text { usage, cause } => {
                if !self.text_started {
                    self.queue.push_back(text_block_start()?);
                }
                self.queue
                    .push_back(content_block_stop(FIRST_CONTENT_BLOCK)?);
                (completion_reason(cause), usage)
            }
            AssistantOutcome::ToolCall(tool_call) => {
                let index = usize::from(self.text_started);
                if self.text_started {
                    self.queue
                        .push_back(content_block_stop(FIRST_CONTENT_BLOCK)?);
                }
                self.queue.push_back(SseFrame::json(
                    "content_block_start",
                    &json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "tool_use",
                            "id": tool_call.id().as_str(),
                            "name": tool_call.tool().as_str(),
                            "input": {}
                        }
                    }),
                )?);
                let partial_json = tool_call.arguments().as_str().to_owned();
                self.queue.push_back(SseFrame::json(
                    "content_block_delta",
                    &json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "input_json_delta", "partial_json": partial_json}
                    }),
                )?);
                self.queue.push_back(content_block_stop(index)?);
                ("tool_use", None)
            }
        };
        let mut terminal = json!({
            "type": "message_delta",
            "delta": {"stop_reason": stop_reason, "stop_sequence": null}
        });
        insert_usage(&mut terminal, usage);
        self.queue
            .push_back(SseFrame::json("message_delta", &terminal)?);
        self.queue.push_back(SseFrame::json(
            "message_stop",
            &json!({"type": "message_stop"}),
        )?);
        Ok(())
    }

    /// Appends a protocol error frame to the response sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when the error frame cannot be encoded.
    pub fn fail(&mut self, error: &BridgeError) -> Result<(), BridgeError> {
        self.queue.push_back(error_frame(error)?);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<SseFrame> {
        self.queue.pop_front()
    }
}

pub struct MessageAccumulator {
    model: String,
    text: String,
}

#[derive(Default)]
pub struct AnthropicBodyAccumulator {
    body: Vec<u8>,
}

impl AnthropicBodyAccumulator {
    /// Appends one upstream chunk to a bounded non-stream response.
    ///
    /// # Errors
    ///
    /// Returns an error before the aggregate response can exceed its byte limit.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), BridgeError> {
        let next_len = self.body.len().checked_add(chunk.len()).ok_or_else(|| {
            BridgeError::anthropic_unavailable("Anthropic non-stream response length overflowed")
        })?;
        if next_len > MAX_NON_STREAM_RESPONSE_BYTES {
            return Err(BridgeError::anthropic_unavailable(format!(
                "Anthropic non-stream response exceeds {MAX_NON_STREAM_RESPONSE_BYTES} bytes"
            )));
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }

    #[must_use]
    pub fn finish(self) -> Option<AnthropicResponseChunk> {
        if self.body.is_empty() {
            None
        } else {
            Some(AnthropicResponseChunk::new(self.body))
        }
    }
}

impl MessageAccumulator {
    #[must_use]
    pub fn new(model: String) -> Self {
        Self {
            model,
            text: String::new(),
        }
    }

    /// Appends one assistant text delta to a bounded non-stream response.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured response-size limit would be exceeded.
    pub fn push(&mut self, delta: &str) -> Result<(), BridgeError> {
        let next_len = self
            .text
            .len()
            .checked_add(delta.len())
            .ok_or_else(|| BridgeError::protocol("non-stream response length overflowed"))?;
        if next_len > MAX_NON_STREAM_RESPONSE_BYTES {
            return Err(BridgeError::protocol(format!(
                "non-stream response exceeds {MAX_NON_STREAM_RESPONSE_BYTES} bytes"
            )));
        }
        self.text.push_str(delta);
        Ok(())
    }

    /// Builds the completed non-stream Anthropic-compatible response.
    ///
    /// # Errors
    ///
    /// Returns an error when tool JSON or the response body cannot be encoded within its limit.
    pub fn finish(self, outcome: AssistantOutcome) -> Result<Response, BridgeError> {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(json!({"type": "text", "text": self.text}));
        }
        let (stop_reason, usage) = match outcome {
            AssistantOutcome::Text { usage, cause } => (completion_reason(cause), usage),
            AssistantOutcome::ToolCall(tool_call) => {
                let arguments =
                    json_contract::object_value(tool_call.arguments()).map_err(|error| {
                        BridgeError::protocol(format!("cannot decode tool arguments: {error}"))
                    })?;
                content.push(json!({
                    "type": "tool_use",
                    "id": tool_call.id().as_str(),
                    "name": tool_call.tool().as_str(),
                    "input": arguments
                }));
                ("tool_use", None)
            }
        };
        let mut message = json!({
            "id": format!("msg_{}", uuid::Uuid::now_v7().simple()),
            "type": "message",
            "role": "assistant",
            "content": content,
            "model": self.model,
            "stop_reason": stop_reason,
            "stop_sequence": null
        });
        insert_usage(&mut message, usage);
        let body = serde_json::to_vec(&message).map_err(|error| {
            BridgeError::protocol(format!("cannot encode JSON response: {error}"))
        })?;
        if body.len() > MAX_NON_STREAM_RESPONSE_BYTES {
            return Err(BridgeError::protocol(format!(
                "non-stream encoded response exceeds {MAX_NON_STREAM_RESPONSE_BYTES} bytes"
            )));
        }
        Ok(json_response(StatusCode::OK, body))
    }
}

pub fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(JSON_CONTENT_TYPE),
    );
    response
}

#[must_use]
pub fn error_response(error: &BridgeError) -> Response {
    let (status, error_type) = match error {
        BridgeError::Authentication => (StatusCode::UNAUTHORIZED, "authentication_error"),
        BridgeError::InvalidRequest(_)
        | BridgeError::Configuration(_)
        | BridgeError::PayloadTooLarge => (StatusCode::BAD_REQUEST, "invalid_request_error"),
        BridgeError::Unsupported(_) => (StatusCode::BAD_REQUEST, "unsupported_capability_error"),
        BridgeError::AppServerUnavailable(_) | BridgeError::AnthropicUnavailable(_) => {
            (StatusCode::BAD_GATEWAY, "api_error")
        }
        BridgeError::AppServerProtocol(_) => (StatusCode::BAD_GATEWAY, "protocol_error"),
    };
    let status = if matches!(error, BridgeError::PayloadTooLarge) {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        status
    };
    let message = error.to_string();
    let body = serde_json::to_vec(&ErrorEnvelope {
        kind: "error",
        error: ErrorBody {
            kind: error_type,
            message: &message,
        },
    })
    .unwrap_or_else(|_| {
        br#"{"type":"error","error":{"type":"api_error","message":"cannot encode error response"}}"#
            .to_vec()
    });
    json_response(status, body)
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: &'a str,
}

fn text_block_start() -> Result<SseFrame, BridgeError> {
    SseFrame::json(
        "content_block_start",
        &json!({
            "type": "content_block_start",
            "index": FIRST_CONTENT_BLOCK,
            "content_block": {"type": "text", "text": ""}
        }),
    )
}

fn content_block_stop(index: usize) -> Result<SseFrame, BridgeError> {
    SseFrame::json(
        "content_block_stop",
        &json!({"type": "content_block_stop", "index": index}),
    )
}

fn error_frame(error: &BridgeError) -> Result<SseFrame, BridgeError> {
    SseFrame::json(
        "error",
        &json!({
            "type": "error",
            "error": {"type": "api_error", "message": error.to_string()}
        }),
    )
}

fn completion_reason(cause: CompletionCause) -> &'static str {
    match cause {
        CompletionCause::EndTurn => "end_turn",
        CompletionCause::OutputLimit => "max_tokens",
    }
}

fn insert_usage(value: &mut Value, usage: Option<TokenUsage>) {
    if let Some(usage) = usage
        && let Some(object) = value.as_object_mut()
    {
        object.insert(
            "usage".to_owned(),
            json!({
                "input_tokens": usage.input().get(),
                "output_tokens": usage.output().get()
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::policies::http_limits::MAX_NON_STREAM_RESPONSE_BYTES;

    use super::AnthropicBodyAccumulator;

    #[test]
    fn anthropic_body_accumulator_rejects_aggregate_over_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut body = AnthropicBodyAccumulator::default();
        body.push(&vec![b'x'; MAX_NON_STREAM_RESPONSE_BYTES])?;
        let error = body
            .push(b"x")
            .err()
            .ok_or_else(|| std::io::Error::other("oversized response was accepted"))?;
        assert!(
            error
                .to_string()
                .contains("Anthropic non-stream response exceeds 8388608 bytes")
        );
        Ok(())
    }

    #[test]
    fn anthropic_body_accumulator_preserves_exact_chunks() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut body = AnthropicBodyAccumulator::default();
        body.push(b"first")?;
        body.push(b"-second")?;
        let body = body
            .finish()
            .ok_or_else(|| std::io::Error::other("missing accumulated response"))?;
        assert_eq!(body.as_bytes(), b"first-second");
        Ok(())
    }
}
