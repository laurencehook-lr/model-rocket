#!/usr/bin/env python3
import json
import sys


if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def send_usage():
    usage = {
        "inputTokens": 20,
        "cachedInputTokens": 0,
        "outputTokens": 4,
        "reasoningOutputTokens": 0,
        "totalTokens": 24,
    }
    send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_stale_empty", "turnId": "turn_stale_empty", "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 272000}}})


tool_name = None
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-stale-tool-usage-empty"}})
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
        send({"id": request_id, "result": {"thread": {"id": "thread_stale_empty"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_stale_empty"}}})
        send({"id": "tool_rpc_stale_empty", "method": "item/tool/call", "params": {"arguments": {"city": "London"}, "callId": "call_stale_empty", "threadId": "thread_stale_empty", "tool": tool_name, "turnId": "turn_stale_empty"}})
        send_usage()
    elif request_id == "tool_rpc_stale_empty":
        send({"method": "turn/completed", "params": {"threadId": "thread_stale_empty", "turn": {"id": "turn_stale_empty", "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
