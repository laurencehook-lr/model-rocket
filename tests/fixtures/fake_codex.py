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
        dynamic_tools = message["params"].get("dynamicTools", [])
        tool_name = tool_name_override
        if tool_name is None and dynamic_tools:
            tool_name = dynamic_tools[0]["name"]
        send({"id": request_id, "result": {"thread": {"id": "thread_1"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_1", "status": "inProgress", "items": [], "error": None}}})
        prompt = message["params"]["input"][0]["text"]
        if "FORBIDDEN_TOOL" in prompt:
            send({"method": "item/started", "params": {"item": {"type": "commandExecution"}}})
        elif "CALL_TOOL" in prompt:
            if tool_name is None:
                raise RuntimeError("CALL_TOOL requested without a dynamic tool")
            send({"id": "tool_rpc_1", "method": "item/tool/call", "params": {"arguments": tool_arguments, "callId": "call_1", "threadId": "thread_1", "tool": tool_name, "turnId": "turn_1"}})
            send_usage(20, 4)
        else:
            send({"method": "item/agentMessage/delta", "params": {"delta": "hello from gpt"}})
            send_usage(12, 3)
            send({"method": "turn/completed", "params": {"turn": {"status": "completed"}}})
    elif request_id == "tool_rpc_1":
        content = message["result"]["contentItems"][0]["text"]
        send({"method": "item/agentMessage/delta", "params": {"delta": "tool returned: " + content}})
        send_usage(8, 5)
        send({"method": "turn/completed", "params": {"turn": {"status": "completed"}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
