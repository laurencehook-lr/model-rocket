use std::sync::Arc;

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
    routing::{get, post},
};
use futures_util::{future, stream};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{
    contracts::anthropic::{
        HEALTH_ROUTE, MESSAGES_ROUTE, MODEL_ROUTE, MODELS_ROUTE, MessageAccumulator, ModelDispatch,
        ResponseSequence, channel_closed, decode_message, decode_model, decode_models,
        error_response, json_response, presented_credential, response_head_missing,
        response_with_body, routed_response_mismatch,
    },
    domain::BridgeError,
    domain::{
        AuthorizedRequest, ModelCatalogue, ModelRequest, ModelResponseChunk, ModelResponseEnd,
        ModelResponseHead,
    },
    policies::http_limits::{MAX_REQUEST_BYTES, MODEL_DELTA_CHANNEL_CAPACITY},
    ports::{ModelResponseSink, ModelRouter, PortFuture},
};

#[derive(Clone)]
struct HttpState {
    model_router: Arc<dyn ModelRouter>,
    catalogue: Arc<ModelCatalogue>,
}

pub fn router(model_router: Arc<dyn ModelRouter>, catalogue: Arc<ModelCatalogue>) -> Router {
    Router::new()
        .route(HEALTH_ROUTE, get(health))
        .route(MESSAGES_ROUTE, post(messages))
        .route(MODELS_ROUTE, get(models))
        .route(MODEL_ROUTE, get(model))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(Arc::new(HttpState {
            model_router,
            catalogue,
        }))
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn messages(
    State(state): State<Arc<HttpState>>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    messages_inner(&state, token, request)
        .await
        .unwrap_or_else(|error| error_response(&error))
}

async fn messages_inner(
    state: &HttpState,
    token: String,
    request: Request,
) -> Result<Response, BridgeError> {
    let credential = presented_credential(token);
    let authorization = state.model_router.authorize(&credential)?;
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| BridgeError::PayloadTooLarge)?;
    dispatch(
        Arc::clone(&state.model_router),
        authorization,
        decode_message(&body, &parts.headers, &parts.uri, &state.catalogue)?,
    )
    .await
}

async fn models(
    State(state): State<Arc<HttpState>>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    models_inner(&state, token, request)
        .await
        .unwrap_or_else(|error| error_response(&error))
}

async fn models_inner(
    state: &HttpState,
    token: String,
    request: Request,
) -> Result<Response, BridgeError> {
    let credential = presented_credential(token);
    dispatch(
        Arc::clone(&state.model_router),
        state.model_router.authorize(&credential)?,
        ModelRequest::Anthropic(decode_models(request.headers(), request.uri())?),
    )
    .await
}

async fn model(
    State(state): State<Arc<HttpState>>,
    Path((token, model)): Path<(String, String)>,
    request: Request,
) -> Response {
    model_inner(&state, token, model, request)
        .await
        .unwrap_or_else(|error| error_response(&error))
}

async fn model_inner(
    state: &HttpState,
    token: String,
    model: String,
    request: Request,
) -> Result<Response, BridgeError> {
    let credential = presented_credential(token);
    let authorization = state.model_router.authorize(&credential)?;
    match decode_model(&model, request.headers(), request.uri(), &state.catalogue)? {
        ModelDispatch::Local(body) => Ok(json_response(StatusCode::OK, body)),
        ModelDispatch::Anthropic(request) => {
            dispatch(
                Arc::clone(&state.model_router),
                authorization,
                ModelRequest::Anthropic(request),
            )
            .await
        }
    }
}

async fn dispatch(
    model_router: Arc<dyn ModelRouter>,
    authorization: AuthorizedRequest,
    request: ModelRequest,
) -> Result<Response, BridgeError> {
    let (head_tx, head_rx) = oneshot::channel();
    let (chunk_tx, chunk_rx) = mpsc::channel(MODEL_DELTA_CHANNEL_CAPACITY);
    let (outcome_tx, outcome_rx) = oneshot::channel();
    let sink = Arc::new(ChannelModelResponseSink {
        head: Mutex::new(Some(head_tx)),
        chunks: chunk_tx,
    });
    let task_sink = Arc::clone(&sink);
    tokio::spawn(async move {
        let outcome = model_router
            .dispatch(authorization, request, task_sink.as_ref())
            .await;
        std::mem::drop(task_sink);
        let _result = outcome_tx.send(outcome);
    });
    std::mem::drop(sink);
    match future::select(head_rx, outcome_rx).await {
        future::Either::Left((Ok(ModelResponseHead::Assistant { model, streaming }), outcome)) => {
            local_response(model.as_str(), streaming, chunk_rx, outcome).await
        }
        future::Either::Left((Ok(ModelResponseHead::Anthropic(head)), outcome)) => {
            anthropic_response(&head, chunk_rx, outcome)
        }
        future::Either::Left((Err(_), _)) | future::Either::Right((Ok(Ok(_)), _)) => {
            Err(response_head_missing())
        }
        future::Either::Right((Ok(Err(error)), _)) => Err(error),
        future::Either::Right((Err(_), _)) => Err(channel_closed()),
    }
}

async fn local_response(
    model: &str,
    streaming: bool,
    chunks: mpsc::Receiver<ModelResponseChunk>,
    outcome: oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>,
) -> Result<Response, BridgeError> {
    if streaming {
        let state = ModelStreamState::new(model, chunks, outcome)?;
        return Ok(Sse::new(stream::unfold(state, model_stream_step)).into_response());
    }
    non_stream_message(model.to_owned(), chunks, outcome).await
}

struct ModelStreamState {
    chunks: mpsc::Receiver<ModelResponseChunk>,
    outcome: Option<oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>>,
    sequence: ResponseSequence,
    phase: ModelStreamPhase,
}

impl ModelStreamState {
    fn new(
        model: &str,
        chunks: mpsc::Receiver<ModelResponseChunk>,
        outcome: oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>,
    ) -> Result<Self, BridgeError> {
        Ok(Self {
            chunks,
            outcome: Some(outcome),
            sequence: ResponseSequence::new(model)?,
            phase: ModelStreamPhase::Deltas,
        })
    }
}

enum ModelStreamPhase {
    Deltas,
    Outcome,
    Finished,
}

async fn model_stream_step(
    mut state: ModelStreamState,
) -> Option<(
    Result<axum::response::sse::Event, BridgeError>,
    ModelStreamState,
)> {
    loop {
        if let Some(frame) = state.sequence.pop() {
            return Some((Ok(frame.into_event()), state));
        }
        match state.phase {
            ModelStreamPhase::Deltas => match state.chunks.recv().await {
                Some(ModelResponseChunk::Assistant(delta)) => {
                    if let Err(error) = state.sequence.accept_delta(delta.as_str()) {
                        return Some((Err(error), state));
                    }
                }
                Some(ModelResponseChunk::Anthropic(_)) => {
                    return Some((Err(routed_response_mismatch()), state));
                }
                None => state.phase = ModelStreamPhase::Outcome,
            },
            ModelStreamPhase::Outcome => {
                let outcome = state.outcome.take()?;
                match outcome.await {
                    Ok(Ok(ModelResponseEnd::Assistant(outcome))) => {
                        if let Err(error) = state.sequence.finish(outcome) {
                            return Some((Err(error), state));
                        }
                    }
                    Ok(Ok(ModelResponseEnd::Anthropic)) => {
                        return Some((Err(routed_response_mismatch()), state));
                    }
                    Ok(Err(error)) => {
                        if let Err(encoding_error) = state.sequence.fail(&error) {
                            return Some((Err(encoding_error), state));
                        }
                    }
                    Err(_) => {
                        let error = channel_closed();
                        if let Err(encoding_error) = state.sequence.fail(&error) {
                            return Some((Err(encoding_error), state));
                        }
                    }
                }
                state.phase = ModelStreamPhase::Finished;
            }
            ModelStreamPhase::Finished => return None,
        }
    }
}

async fn non_stream_message(
    model: String,
    mut chunks: mpsc::Receiver<ModelResponseChunk>,
    outcome: oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>,
) -> Result<Response, BridgeError> {
    let mut accumulator = MessageAccumulator::new(model);
    while let Some(chunk) = chunks.recv().await {
        match chunk {
            ModelResponseChunk::Assistant(delta) => accumulator.push(delta.as_str())?,
            ModelResponseChunk::Anthropic(_) => return Err(routed_response_mismatch()),
        }
    }
    let outcome = match outcome.await.map_err(|_| channel_closed())?? {
        ModelResponseEnd::Assistant(outcome) => outcome,
        ModelResponseEnd::Anthropic => return Err(routed_response_mismatch()),
    };
    accumulator.finish(outcome)
}

struct ChannelModelResponseSink {
    head: Mutex<Option<oneshot::Sender<ModelResponseHead>>>,
    chunks: mpsc::Sender<ModelResponseChunk>,
}

impl ModelResponseSink for ChannelModelResponseSink {
    fn start(&self, head: ModelResponseHead) -> PortFuture<'_, ()> {
        Box::pin(async move {
            self.head
                .lock()
                .await
                .take()
                .ok_or_else(response_head_missing)?
                .send(head)
                .map_err(|_| channel_closed())
        })
    }

    fn emit(&self, chunk: ModelResponseChunk) -> PortFuture<'_, ()> {
        Box::pin(async move { self.chunks.send(chunk).await.map_err(|_| channel_closed()) })
    }
}

fn anthropic_response(
    head: &crate::domain::AnthropicResponseHead,
    chunks: mpsc::Receiver<ModelResponseChunk>,
    outcome: oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>,
) -> Result<Response, BridgeError> {
    let state = AnthropicBodyState {
        chunks,
        outcome: Some(outcome),
        phase: AnthropicBodyPhase::Chunks,
    };
    response_with_body(
        head,
        Body::from_stream(stream::unfold(state, anthropic_body_step)),
    )
}

struct AnthropicBodyState {
    chunks: mpsc::Receiver<ModelResponseChunk>,
    outcome: Option<oneshot::Receiver<Result<ModelResponseEnd, BridgeError>>>,
    phase: AnthropicBodyPhase,
}

enum AnthropicBodyPhase {
    Chunks,
    Outcome,
    Finished,
}

async fn anthropic_body_step(
    mut state: AnthropicBodyState,
) -> Option<(Result<Bytes, BridgeError>, AnthropicBodyState)> {
    loop {
        match state.phase {
            AnthropicBodyPhase::Chunks => match state.chunks.recv().await {
                Some(ModelResponseChunk::Anthropic(chunk)) => {
                    return Some((Ok(Bytes::copy_from_slice(chunk.as_bytes())), state));
                }
                Some(ModelResponseChunk::Assistant(_)) => {
                    return Some((Err(routed_response_mismatch()), state));
                }
                None => state.phase = AnthropicBodyPhase::Outcome,
            },
            AnthropicBodyPhase::Outcome => {
                let outcome = state.outcome.take()?;
                state.phase = AnthropicBodyPhase::Finished;
                match outcome.await {
                    Ok(Ok(ModelResponseEnd::Anthropic)) => return None,
                    Ok(Ok(ModelResponseEnd::Assistant(_))) => {
                        return Some((Err(routed_response_mismatch()), state));
                    }
                    Ok(Err(error)) => return Some((Err(error), state)),
                    Err(_) => return Some((Err(channel_closed()), state)),
                }
            }
            AnthropicBodyPhase::Finished => return None,
        }
    }
}
