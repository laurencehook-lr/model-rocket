use std::{collections::HashSet, sync::Arc};

use super::{BridgeError, ClaudeModelId, CodexModelId, ModelRoute};

const MAX_CONTEXT_TOKENS: u64 = 4_000_000;
const MAX_MODELS: usize = 64;
const MAX_ROUTES: usize = 256;
const MAX_LABEL_BYTES: usize = 512;
const ROUTE_NAMESPACE: &str = "anthropic-model-rocket-";
const CODEX_NAMESPACE: &str = "gpt-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextTokens(u64);

impl ContextTokens {
    /// Creates a validated context-window size.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or implausibly large windows.
    pub fn new(value: u64) -> Result<Self, BridgeError> {
        if value == 0 || value > MAX_CONTEXT_TOKENS {
            return Err(BridgeError::configuration(format!(
                "context_tokens must be between 1 and {MAX_CONTEXT_TOKENS}"
            )));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelDefinition {
    id: CodexModelId,
    display_name: Arc<str>,
    description: Arc<str>,
    context_tokens: ContextTokens,
}

impl ModelDefinition {
    /// Creates validated provider-model metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when identifiers or display metadata are unsafe.
    pub fn new(
        id: impl Into<Arc<str>>,
        display_name: impl Into<Arc<str>>,
        description: impl Into<Arc<str>>,
        context_tokens: ContextTokens,
    ) -> Result<Self, BridgeError> {
        let id = id.into();
        validate_identifier(&id, CODEX_NAMESPACE, "Codex model id")?;
        let display_name = display_name.into();
        validate_label(&display_name, "model display_name")?;
        let description = description.into();
        validate_label(&description, "model description")?;
        Ok(Self {
            id: CodexModelId::new(id),
            display_name,
            description,
            context_tokens,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &CodexModelId {
        &self.id
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub const fn context_tokens(&self) -> ContextTokens {
        self.context_tokens
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCatalogue {
    models: Vec<ModelDefinition>,
    routes: Vec<ModelRoute>,
    canonical_route: ModelRoute,
    minimum_context_tokens: ContextTokens,
}

impl ModelCatalogue {
    /// Creates and cross-validates an immutable model catalogue.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicates, missing references, unused models, or an invalid canonical route.
    pub fn new(
        models: Vec<ModelDefinition>,
        routes: Vec<ModelRoute>,
        canonical_route: &ClaudeModelId,
    ) -> Result<Self, BridgeError> {
        if models.is_empty() || models.len() > MAX_MODELS {
            return Err(BridgeError::configuration(format!(
                "models must contain between 1 and {MAX_MODELS} entries"
            )));
        }
        if routes.is_empty() || routes.len() > MAX_ROUTES {
            return Err(BridgeError::configuration(format!(
                "routes must contain between 1 and {MAX_ROUTES} entries"
            )));
        }
        let mut model_ids = HashSet::new();
        for model in &models {
            if !model_ids.insert(model.id().as_str()) {
                return Err(BridgeError::configuration(format!(
                    "duplicate model id {}",
                    model.id().as_str()
                )));
            }
        }
        let mut route_ids = HashSet::new();
        let mut referenced_models = HashSet::new();
        for route in &routes {
            validate_identifier(
                route.claude_model.as_str(),
                ROUTE_NAMESPACE,
                "Claude route id",
            )?;
            validate_label(&route.display_name, "route display_name")?;
            validate_label(&route.description, "route description")?;
            if !route_ids.insert(route.claude_model.as_str()) {
                return Err(BridgeError::configuration(format!(
                    "duplicate route id {}",
                    route.claude_model.as_str()
                )));
            }
            if !model_ids.contains(route.codex_model.as_str()) {
                return Err(BridgeError::configuration(format!(
                    "route {} references unknown model {}",
                    route.claude_model.as_str(),
                    route.codex_model.as_str()
                )));
            }
            referenced_models.insert(route.codex_model.as_str());
        }
        let canonical_route = routes
            .iter()
            .find(|route| &route.claude_model == canonical_route)
            .cloned()
            .ok_or_else(|| {
                BridgeError::configuration(format!(
                    "canonical_route {} is not a configured route",
                    canonical_route.as_str()
                ))
            })?;
        if let Some(unused) = models
            .iter()
            .find(|model| !referenced_models.contains(model.id().as_str()))
        {
            return Err(BridgeError::configuration(format!(
                "model {} is not referenced by any route",
                unused.id().as_str()
            )));
        }
        let minimum_context_tokens = models
            .iter()
            .map(ModelDefinition::context_tokens)
            .min_by_key(|tokens| tokens.get())
            .ok_or_else(|| BridgeError::configuration("models must not be empty"))?;
        Ok(Self {
            models,
            routes,
            canonical_route,
            minimum_context_tokens,
        })
    }

    #[must_use]
    pub fn route_for(&self, route: &str) -> Option<&ModelRoute> {
        self.routes
            .iter()
            .find(|candidate| candidate.claude_model.as_str() == route)
    }

    #[must_use]
    pub fn canonical_route(&self) -> &ModelRoute {
        &self.canonical_route
    }

    #[must_use]
    pub fn routes(&self) -> &[ModelRoute] {
        &self.routes
    }

    #[must_use]
    pub fn models(&self) -> &[ModelDefinition] {
        &self.models
    }

    #[must_use]
    pub fn minimum_context_tokens(&self) -> ContextTokens {
        self.minimum_context_tokens
    }
}

fn validate_identifier(value: &str, namespace: &str, field: &str) -> Result<(), BridgeError> {
    if !value.starts_with(namespace)
        || value.len() > MAX_LABEL_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(BridgeError::configuration(format!(
            "{field} must start with {namespace} and contain only lowercase ASCII letters, digits, hyphens, underscores, or dots"
        )));
    }
    Ok(())
}

fn validate_label(value: &str, field: &str) -> Result<(), BridgeError> {
    if value.is_empty() || value.len() > MAX_LABEL_BYTES || value.chars().any(char::is_control) {
        return Err(BridgeError::configuration(format!(
            "{field} must be non-empty, at most {MAX_LABEL_BYTES} bytes, and contain no control characters"
        )));
    }
    Ok(())
}
