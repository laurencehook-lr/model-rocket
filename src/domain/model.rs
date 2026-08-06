use std::sync::Arc;

use super::identifiers::{ClaudeModelId, CodexModelId};

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
