use std::sync::Arc;

/// The real model identifier accepted by a model provider.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CodexModelId(Arc<str>);

impl CodexModelId {
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The route identifier visible to Claude Code.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ClaudeModelId(Arc<str>);

impl ClaudeModelId {
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ReasoningEffort {
    Low,
    High,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ServiceTier {
    Standard,
    Fast,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRoute {
    pub claude_model: ClaudeModelId,
    pub display_name: Arc<str>,
    pub description: Arc<str>,
    pub codex_model: CodexModelId,
    pub service_tier: ServiceTier,
    pub reasoning_effort: ReasoningEffort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountMode {
    ManagedChatGpt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreflightReport {
    account: AccountMode,
    model: CodexModelId,
}

impl PreflightReport {
    #[must_use]
    pub fn new(account: AccountMode, model: CodexModelId) -> Self {
        Self { account, model }
    }

    #[must_use]
    pub const fn account(&self) -> AccountMode {
        self.account
    }

    #[must_use]
    pub const fn model(&self) -> &CodexModelId {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountMode, CodexModelId, PreflightReport};

    #[test]
    fn preflight_report_uses_typed_values() {
        let report = PreflightReport::new(AccountMode::ManagedChatGpt, CodexModelId::new("model"));
        assert_eq!(report.account(), AccountMode::ManagedChatGpt);
        assert_eq!(report.model().as_str(), "model");
    }
}
