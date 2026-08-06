mod diagnostics;
mod messages;
mod request;
mod response;
mod transport;

pub use diagnostics::{
    body_stream_ended, channel_closed, client_build_failed, header_conversion_failed,
    invalid_base_url, invalid_status, redirect_rejected, request_failed, response_head_missing,
    routed_response_mismatch,
};
pub use messages::MessagesRequest;
pub use request::{
    ModelDispatch, decode_message, decode_model, decode_models, presented_credential,
};
pub use response::{MessageAccumulator, ResponseSequence, SseFrame, error_response, json_response};
pub use transport::{
    ANTHROPIC_BASE_URL, CLAUDE_SESSION_HEADER, HEALTH_ROUTE, MESSAGES_ROUTE, MODEL_ROUTE,
    MODELS_ROUTE, ReqwestRequestParts, build_reqwest_request, response_head, response_with_body,
};
