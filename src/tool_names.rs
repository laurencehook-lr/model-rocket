use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::{anthropic::Tool, error::BridgeError};

const CODEX_TOOL_ALIAS_PREFIX: &str = "model_rocket_tool_";
const CLAUDE_TOOL_DESCRIPTION_PREFIX: &str = "Claude Code tool name: ";
const MAX_CLAUDE_TOOL_NAME_BYTES: usize = 64;

#[derive(Debug)]
pub(crate) struct CodexDynamicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Default)]
pub(crate) struct DynamicToolNames {
    tools: Vec<CodexDynamicTool>,
    claude_name_by_codex_name: HashMap<String, String>,
}

impl DynamicToolNames {
    pub(crate) fn from_claude_tools(tools: &[Tool]) -> Result<Self, BridgeError> {
        let mut seen_claude_names = HashSet::with_capacity(tools.len());
        let mut codex_tools = Vec::with_capacity(tools.len());
        let mut claude_name_by_codex_name = HashMap::with_capacity(tools.len());

        for (index, tool) in tools.iter().enumerate() {
            if !valid_claude_tool_name(&tool.name) {
                return Err(BridgeError::invalid_request(format!(
                    "invalid Claude tool name {}",
                    tool.name
                )));
            }
            if !seen_claude_names.insert(tool.name.as_str()) {
                return Err(BridgeError::invalid_request(format!(
                    "duplicate Claude tool name {}",
                    tool.name
                )));
            }

            let codex_name = format!("{CODEX_TOOL_ALIAS_PREFIX}{index}");
            let description = format!(
                "{CLAUDE_TOOL_DESCRIPTION_PREFIX}{}\n\n{}",
                tool.name, tool.description
            );
            claude_name_by_codex_name.insert(codex_name.clone(), tool.name.clone());
            codex_tools.push(CodexDynamicTool {
                name: codex_name,
                description,
                input_schema: tool.input_schema.clone(),
            });
        }

        Ok(Self {
            tools: codex_tools,
            claude_name_by_codex_name,
        })
    }

    pub(crate) fn tools(&self) -> &[CodexDynamicTool] {
        &self.tools
    }

    pub(crate) fn claude_name(&self, codex_name: &str) -> Result<&str, BridgeError> {
        self.claude_name_by_codex_name
            .get(codex_name)
            .map(String::as_str)
            .ok_or_else(|| {
                BridgeError::protocol(format!("Codex requested unknown dynamic tool {codex_name}"))
            })
    }
}

fn valid_claude_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_CLAUDE_TOOL_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::DynamicToolNames;
    use crate::anthropic::Tool;

    #[test]
    fn reserved_claude_name_is_aliased_and_restored() -> Result<(), Box<dyn std::error::Error>> {
        let original = "mcp__plugin_context7_context7__query-docs";
        let names = DynamicToolNames::from_claude_tools(&[Tool {
            name: original.to_owned(),
            description: "Query documentation".to_owned(),
            input_schema: json!({"type": "object"}),
        }])?;

        let mapped = names.tools().first().ok_or("mapped tool was not created")?;
        assert_eq!(mapped.name, "model_rocket_tool_0");
        assert!(!mapped.name.starts_with("mcp__"));
        assert_eq!(names.claude_name(&mapped.name)?, original);
        assert!(mapped.description.contains(original));
        Ok(())
    }

    #[test]
    fn duplicate_claude_names_fail_explicitly() -> Result<(), Box<dyn std::error::Error>> {
        let tool = Tool {
            name: "Read".to_owned(),
            description: String::new(),
            input_schema: json!({"type": "object"}),
        };

        let Err(error) = DynamicToolNames::from_claude_tools(&[tool.clone(), tool]) else {
            return Err("duplicate tool names were accepted".into());
        };
        assert_eq!(
            error.to_string(),
            "invalid request: duplicate Claude tool name Read"
        );
        Ok(())
    }

    #[test]
    fn invalid_claude_names_fail_before_aliasing() -> Result<(), Box<dyn std::error::Error>> {
        for invalid_name in ["", "contains space", "contains.dot", &"x".repeat(65)] {
            let tool = Tool {
                name: invalid_name.to_owned(),
                description: String::new(),
                input_schema: json!({"type": "object"}),
            };
            let Err(error) = DynamicToolNames::from_claude_tools(&[tool]) else {
                return Err(
                    format!("invalid Claude tool name was accepted: {invalid_name}").into(),
                );
            };
            assert!(error.to_string().contains("invalid Claude tool name"));
        }
        Ok(())
    }

    #[test]
    fn unknown_codex_name_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let names = DynamicToolNames::default();
        let Err(error) = names.claude_name("Read") else {
            return Err("unknown Codex tool name was accepted".into());
        };
        assert_eq!(
            error.to_string(),
            "App Server protocol error: Codex requested unknown dynamic tool Read"
        );
        Ok(())
    }
}
