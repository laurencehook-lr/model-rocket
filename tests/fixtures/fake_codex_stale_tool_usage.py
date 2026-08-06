#!/usr/bin/env python3
import json
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
    send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_stale", "turnId": "turn_stale", "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 272000}}})


tool_name = None
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-stale-tool-usage"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        dynamic_tools = message["params"].get("dynamicTools", [])
        if not dynamic_tools:
            raise RuntimeError("dynamic tool missing")
        tool_name = dynamic_tools[0]["name"]
        send({"id": request_id, "result": {"thread": {"id": "thread_stale"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_stale"}}})
        send({"id": "tool_rpc_stale", "method": "item/tool/call", "params": {"arguments": {"city": "London"}, "callId": "call_stale", "threadId": "thread_stale", "tool": tool_name, "turnId": "turn_stale"}})
        send_usage(20, 4)
    elif request_id == "tool_rpc_stale":
        send({"method": "item/agentMessage/delta", "params": {"delta": "continuation text", "itemId": "item_after_tool", "threadId": "thread_stale", "turnId": "turn_stale"}})
        send({"method": "turn/completed", "params": {"threadId": "thread_stale", "turn": {"id": "turn_stale", "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
