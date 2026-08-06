use std::sync::Arc;

use super::{ClaudeToolName, JsonObject, ToolUseId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDescription(Arc<str>);

impl ToolDescription {
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    name: ClaudeToolName,
    description: ToolDescription,
    input_schema: JsonObject,
}

impl ToolDefinition {
    #[must_use]
    pub fn new(
        name: ClaudeToolName,
        description: ToolDescription,
        input_schema: JsonObject,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
        }
    }

    #[must_use]
    pub fn name(&self) -> &ClaudeToolName {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &ToolDescription {
        &self.description
    }

    #[must_use]
    pub fn input_schema(&self) -> &JsonObject {
        &self.input_schema
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolSet(Arc<[ToolDefinition]>);

impl ToolSet {
    #[must_use]
    pub fn new(tools: impl Into<Arc<[ToolDefinition]>>) -> Self {
        Self(tools.into())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[ToolDefinition] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    id: ToolUseId,
    tool: ClaudeToolName,
    arguments: JsonObject,
}

impl ToolCall {
    #[must_use]
    pub fn new(id: ToolUseId, tool: ClaudeToolName, arguments: JsonObject) -> Self {
        Self {
            id,
            tool,
            arguments,
        }
    }

    #[must_use]
    pub fn id(&self) -> &ToolUseId {
        &self.id
    }

    #[must_use]
    pub fn tool(&self) -> &ClaudeToolName {
        &self.tool
    }

    #[must_use]
    pub fn arguments(&self) -> &JsonObject {
        &self.arguments
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResultText(Arc<str>);

impl ToolResultText {
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolResultDisposition {
    Success,
    Failure,
}

#[cfg(test)]
mod tests {
    use super::{JsonObject, ToolDefinition, ToolDescription, ToolSet};
    use crate::domain::ClaudeToolName;

    #[test]
    fn tool_set_owns_definitions() -> Result<(), Box<dyn std::error::Error>> {
        let definition = ToolDefinition::new(
            ClaudeToolName::from("Read"),
            ToolDescription::new("Read a file"),
            JsonObject::from_validated("{\"type\":\"object\"}"),
        );
        let tools = ToolSet::new(vec![definition]);
        let tool = tools.as_slice().first().ok_or("tool was not retained")?;
        assert_eq!(tool.name().as_str(), "Read");
        Ok(())
    }
}
