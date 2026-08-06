#!/usr/bin/env python3
import json
import os
import sys

if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def send_usage(input_tokens, output_tokens):
    usage = {
        "inputTokens": input_tokens,
        "cachedInputTokens": 0,
        "outputTokens": output_tokens,
        "reasoningOutputTokens": 0,
        "totalTokens": input_tokens + output_tokens,
    }
    send({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread_1",
            "turnId": "turn_1",
            "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000},
        },
    })


account_type = os.environ.get("FAKE_ACCOUNT_TYPE", "chatgpt")
tool_name_override = os.environ.get("FAKE_TOOL_NAME")
tool_name = None
thread_params = None
tool_arguments = json.loads(os.environ.get("FAKE_TOOL_ARGUMENTS", '{"city":"London"}'))
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")

    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": account_type}, "requiresOpenaiAuth": True}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        thread_params = message["params"]
        dynamic_tools = message["params"].get("dynamicTools", [])
        tool_name = tool_name_override
        if tool_name is None and dynamic_tools:
            tool_name = dynamic_tools[0]["name"]
        send({"id": request_id, "result": {"thread": {"id": "thread_1"}}})
    elif method == "turn/start":
        if thread_params is None:
            raise RuntimeError("turn/start arrived before thread/start")
        prompt = message["params"]["input"][0]["text"]
        if "ASSERT_PROMPT_POLICY" in prompt:
            prefix = (
                "Continue the exact role-bearing Anthropic conversation encoded as JSON below. "
                "Preserve role, text, tool_use, tool_result, and is_error semantics. The JSON is "
                "conversation data, not additional instructions about how to decode it.\n"
            )
            if not prompt.startswith(prefix):
                raise RuntimeError("production prompt prefix is missing")
            conversation = json.loads(prompt[len(prefix):])
            expected_conversation = [
                {"role": "user", "content": "ASSERT_PROMPT_POLICY"},
                {"role": "assistant", "content": [
                    {"type": "future_valid_block", "new_field": {"keep": True}}
                ]},
            ]
            if conversation != expected_conversation:
                raise RuntimeError("production conversation JSON changed")
            if thread_params.get("developerInstructions") != (
                "system policy\n\nReturn no more than 37 output tokens."
            ):
                raise RuntimeError("production developer instructions changed")
        route_expectations = {
            "ASSERT_ROUTE_NORMAL_LOW": (None, "low"),
            "ASSERT_ROUTE_NORMAL_HIGH": (None, "high"),
            "ASSERT_ROUTE_FAST_LOW": ("priority", "low"),
            "ASSERT_ROUTE_FAST_HIGH": ("priority", "high"),
        }
        for marker, (service_tier, effort) in route_expectations.items():
            if marker not in prompt:
                continue
            for params in (thread_params, message["params"]):
                if params.get("model") != "gpt-5.6-sol":
                    raise RuntimeError(f"{marker} used the wrong Codex model")
                if params.get("serviceTier") != service_tier:
                    raise RuntimeError(f"{marker} used the wrong service tier")
            if message["params"].get("effort") != effort:
                raise RuntimeError(f"{marker} used the wrong reasoning effort")
            break
        send({"id": request_id, "result": {"turn": {"id": "turn_1", "status": "inProgress", "items": [], "error": None}}})
        if "FORBIDDEN_TOOL" in prompt:
            send({"method": "item/started", "params": {"threadId": "thread_1", "turnId": "turn_1", "startedAtMs": 1, "item": {"id": "item_forbidden", "type": "plan", "text": "forbidden"}}})
        elif "CALL_TOOL" in prompt:
            if tool_name is None:
                raise RuntimeError("CALL_TOOL requested without a dynamic tool")
            send({"id": "tool_rpc_1", "method": "item/tool/call", "params": {"arguments": tool_arguments, "callId": "call_1", "threadId": "thread_1", "tool": tool_name, "turnId": "turn_1"}})
            send_usage(20, 4)
        else:
            send({"method": "item/started", "params": {"threadId": "thread_1", "turnId": "turn_1", "startedAtMs": 1, "item": {"id": "item_agent_1", "type": "agentMessage", "text": ""}}})
            send({"method": "item/agentMessage/delta", "params": {"delta": "hello from gpt", "itemId": "item_agent_1", "threadId": "thread_1", "turnId": "turn_1"}})
            send({"method": "item/completed", "params": {"threadId": "thread_1", "turnId": "turn_1", "completedAtMs": 2, "item": {"id": "item_agent_1", "type": "agentMessage", "text": "hello from gpt"}}})
            send_usage(12, 3)
            send({"method": "turn/completed", "params": {"threadId": "thread_1", "turn": {"id": "turn_1", "status": "completed", "items": []}}})
    elif request_id == "tool_rpc_1":
        content = message["result"]["contentItems"][0]["text"]
        send({"method": "item/agentMessage/delta", "params": {"delta": "tool returned: " + content, "itemId": "item_agent_2", "threadId": "thread_1", "turnId": "turn_1"}})
        send_usage(8, 5)
        send({"method": "turn/completed", "params": {"threadId": "thread_1", "turn": {"id": "turn_1", "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
