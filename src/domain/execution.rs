use std::{fmt, sync::Arc};

use super::{
    AnthropicRequest, AnthropicResponseChunk, AnthropicResponseHead, ClaudeSessionId, CodexModelId,
    CompletionCause, JsonDocument, JsonObject, ModelRoute, OutputTokenLimit, ReasoningEffort,
    RequestedModelId, ServiceTier, TokenUsage, ToolCall, ToolResultDisposition, ToolResultText,
    ToolSet, ToolUseId,
};

#[derive(Clone, Debug, PartialEq)]
pub enum ModelRequest {
    Assistant {
        model: RequestedModelId,
        streaming: bool,
        execution: ExecuteMessage,
    },
    Anthropic(AnthropicRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelResponseHead {
    Assistant {
        model: RequestedModelId,
        streaming: bool,
    },
    Anthropic(AnthropicResponseHead),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelResponseChunk {
    Assistant(AssistantTextDelta),
    Anthropic(AnthropicResponseChunk),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModelResponseEnd {
    Assistant(AssistantOutcome),
    Anthropic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingDirectory(Arc<str>);

impl WorkingDirectory {
    #[must_use]
    pub fn new(path: impl Into<Arc<str>>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! string_value {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name(Arc<str>);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<Arc<str>>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationJson(JsonDocument);

impl ConversationJson {
    #[must_use]
    pub fn new(document: JsonDocument) -> Self {
        Self(document)
    }

    #[must_use]
    pub fn document(&self) -> &JsonDocument {
        &self.0
    }
}

string_value!(SystemInstructions);
string_value!(ModelPrompt);
string_value!(DeveloperInstructions);
string_value!(AssistantTextDelta);

#[derive(Clone, PartialEq, Eq)]
pub struct PresentedCredential(Arc<str>);

impl PresentedCredential {
    #[must_use]
    pub(crate) fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExpectedCredential(Arc<str>);

impl ExpectedCredential {
    #[must_use]
    pub(crate) fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ExpectedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExpectedCredential([REDACTED])")
    }
}

impl fmt::Debug for PresentedCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PresentedCredential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizedRequest {
    _private: (),
}

impl AuthorizedRequest {
    #[must_use]
    pub(crate) const fn granted() -> Self {
        Self { _private: () }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingToolResult {
    id: ToolUseId,
    text: ToolResultText,
    disposition: ToolResultDisposition,
}

impl PendingToolResult {
    #[must_use]
    pub fn new(id: ToolUseId, text: ToolResultText, disposition: ToolResultDisposition) -> Self {
        Self {
            id,
            text,
            disposition,
        }
    }

    #[must_use]
    pub fn id(&self) -> &ToolUseId {
        &self.id
    }

    #[must_use]
    pub fn text(&self) -> &ToolResultText {
        &self.text
    }

    #[must_use]
    pub const fn disposition(&self) -> ToolResultDisposition {
        self.disposition
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecuteMessage {
    requested_model: ModelRoute,
    session: ClaudeSessionId,
    conversation: ConversationJson,
    system: SystemInstructions,
    tools: ToolSet,
    output_limit: OutputTokenLimit,
    output_schema: Option<JsonObject>,
    pending_tool_result: Option<PendingToolResult>,
}

impl ExecuteMessage {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        requested_model: ModelRoute,
        session: ClaudeSessionId,
        conversation: ConversationJson,
        system: SystemInstructions,
        tools: ToolSet,
        output_limit: OutputTokenLimit,
        output_schema: Option<JsonObject>,
        pending_tool_result: Option<PendingToolResult>,
    ) -> Self {
        Self {
            requested_model,
            session,
            conversation,
            system,
            tools,
            output_limit,
            output_schema,
            pending_tool_result,
        }
    }

    #[must_use]
    pub const fn requested_model(&self) -> &ModelRoute {
        &self.requested_model
    }
    #[must_use]
    pub fn session(&self) -> &ClaudeSessionId {
        &self.session
    }
    #[must_use]
    pub fn conversation(&self) -> &ConversationJson {
        &self.conversation
    }
    #[must_use]
    pub fn system(&self) -> &SystemInstructions {
        &self.system
    }
    #[must_use]
    pub fn tools(&self) -> &ToolSet {
        &self.tools
    }
    #[must_use]
    pub const fn output_limit(&self) -> OutputTokenLimit {
        self.output_limit
    }
    #[must_use]
    pub fn output_schema(&self) -> Option<&JsonObject> {
        self.output_schema.as_ref()
    }
    #[must_use]
    pub fn pending_tool_result(&self) -> Option<&PendingToolResult> {
        self.pending_tool_result.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartModelTurn {
    model: CodexModelId,
    working_directory: WorkingDirectory,
    tools: ToolSet,
    prompt: ModelPrompt,
    developer_instructions: DeveloperInstructions,
    output_limit: OutputTokenLimit,
    reasoning: ReasoningEffort,
    delivery: ServiceTier,
    output_schema: Option<JsonObject>,
}

impl StartModelTurn {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        model: CodexModelId,
        working_directory: WorkingDirectory,
        tools: ToolSet,
        prompt: ModelPrompt,
        developer_instructions: DeveloperInstructions,
        output_limit: OutputTokenLimit,
        reasoning: ReasoningEffort,
        delivery: ServiceTier,
        output_schema: Option<JsonObject>,
    ) -> Self {
        Self {
            model,
            working_directory,
            tools,
            prompt,
            developer_instructions,
            output_limit,
            reasoning,
            delivery,
            output_schema,
        }
    }

    #[must_use]
    pub const fn model(&self) -> &CodexModelId {
        &self.model
    }
    #[must_use]
    pub fn working_directory(&self) -> &WorkingDirectory {
        &self.working_directory
    }
    #[must_use]
    pub fn tools(&self) -> &ToolSet {
        &self.tools
    }
    #[must_use]
    pub fn prompt(&self) -> &ModelPrompt {
        &self.prompt
    }
    #[must_use]
    pub fn developer_instructions(&self) -> &DeveloperInstructions {
        &self.developer_instructions
    }
    #[must_use]
    pub const fn output_limit(&self) -> OutputTokenLimit {
        self.output_limit
    }
    #[must_use]
    pub const fn reasoning(&self) -> ReasoningEffort {
        self.reasoning
    }
    #[must_use]
    pub const fn delivery(&self) -> ServiceTier {
        self.delivery
    }
    #[must_use]
    pub fn output_schema(&self) -> Option<&JsonObject> {
        self.output_schema.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinueModelTurn {
    result: ToolResultText,
    disposition: ToolResultDisposition,
    output_limit: OutputTokenLimit,
}

impl ContinueModelTurn {
    #[must_use]
    pub fn new(
        result: ToolResultText,
        disposition: ToolResultDisposition,
        output_limit: OutputTokenLimit,
    ) -> Self {
        Self {
            result,
            disposition,
            output_limit,
        }
    }

    #[must_use]
    pub fn result(&self) -> &ToolResultText {
        &self.result
    }
    #[must_use]
    pub const fn disposition(&self) -> ToolResultDisposition {
        self.disposition
    }
    #[must_use]
    pub const fn output_limit(&self) -> OutputTokenLimit {
        self.output_limit
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssistantOutcome {
    Text {
        usage: Option<TokenUsage>,
        cause: CompletionCause,
    },
    ToolCall(ToolCall),
}

#[cfg(test)]
mod tests {
    use super::{PresentedCredential, WorkingDirectory};

    #[test]
    fn credential_debug_never_exposes_secret() {
        let credential = PresentedCredential::new("secret-token");
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("secret-token"));
        assert!(rendered.contains("REDACTED"));
    }

    #[test]
    fn working_directory_owns_its_path() {
        let source = String::from("/tmp/model-rocket");
        let directory = WorkingDirectory::new(source.as_str());
        drop(source);
        assert_eq!(directory.as_str(), "/tmp/model-rocket");
    }
}
