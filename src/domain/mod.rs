//! Provider-neutral values used by Model Rocket's application boundary.

mod anthropic;
mod catalogue;
mod error;
pub(crate) mod executable;
mod execution;
mod identifiers;
mod json;
mod model;
mod tokens;
mod tool;

pub use catalogue::{ContextTokens, ModelCatalogue, ModelDefinition};
pub use execution::{
    AssistantOutcome, AssistantTextDelta, AuthorizedRequest, ContinueModelTurn, ConversationJson,
    DeveloperInstructions, ExecuteMessage, ExpectedCredential, ModelPrompt, ModelRequest,
    ModelResponseChunk, ModelResponseEnd, ModelResponseHead, PendingToolResult,
    PresentedCredential, StartModelTurn, SystemInstructions, WorkingDirectory,
};
pub use identifiers::{
    ClaudeModelId, ClaudeSessionId, ClaudeToolName, CodexModelId, RequestedModelId, ToolUseId,
};
pub use json::{JsonDocument, JsonObject};
pub use model::{AccountMode, ModelRoute, PreflightReport, ReasoningEffort, ServiceTier};
pub use tokens::{
    CompletionCause, OutputTokenLimit, OutputTokenLimitError, TokenCount, TokenUsage,
};
pub use tool::{
    ToolCall, ToolDefinition, ToolDescription, ToolResultDisposition, ToolResultText, ToolSet,
};

pub use anthropic::{
    AnthropicHeader, AnthropicHeaders, AnthropicOperation, AnthropicQuery, AnthropicRequest,
    AnthropicRequestBody, AnthropicRequestError, AnthropicResponseChunk, AnthropicResponseHead,
    AnthropicStatus, AnthropicStatusError, HeaderError,
};
pub use error::BridgeError;
pub use executable::ValidatedCodexExecutable;
