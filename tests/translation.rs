use model_rocket::anthropic::MessagesRequest;

#[test]
fn transcript_preserves_text_and_tool_order() -> Result<(), Box<dyn std::error::Error>> {
    let messages = r#"[{"role":"user","content":"question"}, {"role":"assistant","content":[{"type":"future_valid_block","new_field":{"keep":true}}, {"type":"tool_use","id":"call_1","name":"weather","input":{"city":"London"},"future_tool_field":7}]}, {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"sunny","future_result_field":"keep"}]}]"#;
    let request_json = format!(
        r#"{{"model":"gpt-5.6-sol","max_tokens":100,"stream":true,"system":"system text","messages":{messages}}}"#
    );
    let request = serde_json::from_str::<MessagesRequest>(&request_json)?;

    let transcript = request.transcript()?;
    let (_, encoded) = transcript
        .split_once('\n')
        .ok_or_else(|| std::io::Error::other("encoded transcript missing"))?;
    assert_eq!(encoded, messages);
    let developer_instructions = request.developer_instructions();
    assert!(developer_instructions.contains("system text"));
    assert!(developer_instructions.contains("100 output tokens"));
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
