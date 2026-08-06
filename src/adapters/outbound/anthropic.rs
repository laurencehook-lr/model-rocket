use futures_util::StreamExt;

use crate::{
    contracts::anthropic::{
        ANTHROPIC_BASE_URL, body_stream_ended, build_reqwest_request, client_build_failed,
        invalid_base_url, redirect_rejected, request_failed, response_head,
    },
    domain::BridgeError,
    domain::{AnthropicRequest, AnthropicResponseChunk},
    ports::{AnthropicGateway, AnthropicResponseSink, PortFuture},
};

#[derive(Clone)]
pub struct ReqwestAnthropicGateway {
    client: reqwest::Client,
    base_url: reqwest::Url,
}

impl ReqwestAnthropicGateway {
    /// Creates the production Anthropic gateway.
    ///
    /// # Errors
    ///
    /// Returns an error when the pinned upstream endpoint or client cannot be constructed.
    pub fn new() -> Result<Self, BridgeError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(client_build_failed)?;
        let base_url = reqwest::Url::parse(ANTHROPIC_BASE_URL).map_err(invalid_base_url)?;
        Ok(Self { client, base_url })
    }
}

impl AnthropicGateway for ReqwestAnthropicGateway {
    fn exchange<'a>(
        &'a self,
        request: AnthropicRequest,
        output: &'a dyn AnthropicResponseSink,
    ) -> PortFuture<'a, ()> {
        Box::pin(async move {
            let parts = build_reqwest_request(&self.base_url, &request)?;
            let response = self
                .client
                .request(parts.method, parts.url)
                .headers(parts.headers)
                .body(parts.body)
                .send()
                .await
                .map_err(request_failed)?;
            if response.status().is_redirection() {
                return Err(redirect_rejected());
            }
            output
                .start(response_head(response.status(), response.headers())?)
                .await?;
            let mut body = response.bytes_stream();
            while let Some(chunk) = body.next().await {
                let chunk = chunk.map_err(body_stream_ended)?;
                output
                    .emit(AnthropicResponseChunk::new(chunk.to_vec()))
                    .await?;
            }
            Ok(())
        })
    }
}
