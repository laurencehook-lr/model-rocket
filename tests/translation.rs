use model_rocket::{
    contracts::anthropic::MessagesRequest,
    domain::{
        ClaudeModelId, ClaudeSessionId, CodexModelId, ModelRoute, ReasoningEffort, ServiceTier,
    },
    policies::model_prompt::{developer_instructions, model_prompt},
};

#[test]
fn transcript_preserves_text_and_tool_order() -> Result<(), Box<dyn std::error::Error>> {
    let messages = r#"[{"role":"user","content":"question"}, {"role":"system","content":"provider transition"}, {"role":"system","content":[{"type":"text","text":"weighted token budget"}]}, {"role":"assistant","content":[{"type":"future_valid_block","new_field":{"keep":true}}, {"type":"tool_use","id":"call_1","name":"weather","input":{"city":"London"},"future_tool_field":7}]}, {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"sunny","future_result_field":"keep"}]}]"#;
    let request_json = format!(
        r#"{{"model":"anthropic-model-rocket-gpt-5.6-sol-normal-high","max_tokens":100,"stream":true,"system":"system text","messages":{messages}}}"#
    );
    let request = serde_json::from_str::<MessagesRequest>(&request_json)?;

    let execution = request.execute_message(
        ClaudeSessionId::from("claude-session"),
        ModelRoute {
            claude_model: ClaudeModelId::new("anthropic-model-rocket-gpt-5.6-sol-normal-high"),
            display_name: "Test route".into(),
            description: "Test route description".into(),
            codex_model: CodexModelId::new("gpt-5.6-sol"),
            service_tier: ServiceTier::Standard,
            reasoning_effort: ReasoningEffort::High,
        },
    )?;
    let prompt = model_prompt(execution.conversation());
    let (_, encoded) = prompt
        .as_str()
        .split_once('\n')
        .ok_or_else(|| std::io::Error::other("encoded transcript missing"))?;
    assert_eq!(encoded, messages);
    let instructions = developer_instructions(execution.system(), execution.output_limit());
    assert!(instructions.as_str().contains("system text"));
    assert!(instructions.as_str().contains("100 output tokens"));
    Ok(())
}

#[test]
fn malformed_or_known_unsupported_conversations_fail_explicitly()
-> Result<(), Box<dyn std::error::Error>> {
    let invalid_messages = [
        serde_json::json!([1]),
        serde_json::json!([{"role": "user"}]),
        serde_json::json!([{"content": "hello"}]),
        serde_json::json!([{"role": "system", "content": [
            {"type": "tool_use", "id": "call_1", "name": "weather", "input": {}}
        ]}]),
        serde_json::json!([{"role": "user", "content": 7}]),
        serde_json::json!([{"role": "user", "content": [1]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "data": "ignored"}}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "document", "source": {"type": "text", "data": "ignored"}}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "text"}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "tool_use", "id": "call_1", "name": "weather", "input": {}}
        ]}]),
        serde_json::json!([{"role": "assistant", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
        ]}]),
        serde_json::json!([{"role": "assistant", "content": [
            {"type": "tool_use", "id": "call_1", "name": "weather", "input": "London"}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": [
                {"type": "image", "source": {"type": "base64", "data": "ignored"}}
            ]}
        ]}]),
    ];

    for messages in invalid_messages {
        let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": messages
        }))?;
        assert!(
            request.validate("gpt-5.6-sol").is_err(),
            "malformed conversation was accepted"
        );
    }
    Ok(())
}

#[test]
fn unsupported_fields_fail_explicitly() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
        "model": "gpt-5.6-sol",
        "max_tokens": 100,
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}],
        "thinking": {"type": "enabled"}
    }))?;
    let error = request
        .validate("gpt-5.6-sol")
        .err()
        .ok_or_else(|| std::io::Error::other("thinking must not be silently discarded"))?;
    assert!(error.to_string().contains("adaptive thinking"));

    let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
        "model": "gpt-5.6-sol",
        "max_tokens": 100,
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}],
        "unknown_top_level_field": true
    }))?;
    let error = request
        .validate("gpt-5.6-sol")
        .err()
        .ok_or_else(|| std::io::Error::other("unknown field must not be silently discarded"))?;
    assert!(error.to_string().contains("unsupported request fields"));
    Ok(())
}

#[test]
fn one_final_tool_result_is_extracted() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
        "model": "gpt-5.6-sol",
        "max_tokens": 100,
        "stream": true,
        "messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny", "is_error": false}
        ]}]
    }))?;
    let result = request
        .final_tool_result()?
        .ok_or_else(|| std::io::Error::other("result missing"))?;
    assert_eq!(result.tool_use_id, "call_1");
    assert_eq!(result.text, "sunny");
    assert!(!result.is_error);
    Ok(())
}

#[test]
fn malformed_tool_continuation_envelopes_fail_explicitly() -> Result<(), Box<dyn std::error::Error>>
{
    for messages in [
        serde_json::json!([{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"},
            {"type": "text", "text": "also keep this"}
        ]}]),
        serde_json::json!([{"role": "user", "content": [
            {"type": "text", "text": "also keep this"},
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
        ]}]),
        serde_json::json!([{"role": "assistant", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "sunny"}
        ]}]),
    ] {
        let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
            "model": "gpt-5.6-sol",
            "max_tokens": 100,
            "stream": true,
            "messages": messages
        }))?;
        let error = request
            .final_tool_result()
            .err()
            .ok_or("malformed continuation was accepted")?;
        assert!(error.to_string().contains("exactly one tool_result block"));
    }
    Ok(())
}

#[test]
fn structured_output_rejects_unknown_format_type() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_value::<MessagesRequest>(serde_json::json!({
        "model": "gpt-5.6-sol",
        "max_tokens": 100,
        "stream": false,
        "messages": [{"role": "user", "content": "hello"}],
        "output_config": {
            "format": {"type": "not_json_schema", "schema": {"type": "object"}}
        }
    }))?;
    let error = request
        .validate("gpt-5.6-sol")
        .err()
        .ok_or_else(|| std::io::Error::other("unknown output format must fail"))?;
    assert!(error.to_string().contains("must be json_schema"));
    Ok(())
}
