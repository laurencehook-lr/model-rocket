#!/usr/bin/env python3
import json
import sys

if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


tool_name = None
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-limit-tool"}})
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
        send({"id": request_id, "result": {"thread": {"id": "thread_limit"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_limit"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "éstreamed beyond limit", "itemId": "item_limit", "threadId": "thread_limit", "turnId": "turn_limit"}})
    elif method == "turn/interrupt":
        send({"id": request_id, "result": {}})
        send({"id": "tool_after_limit", "method": "item/tool/call", "params": {"arguments": {"city": "London"}, "callId": "call_after_limit", "threadId": "thread_limit", "tool": tool_name, "turnId": "turn_limit"}})
        send({"method": "turn/completed", "params": {"threadId": "thread_limit", "turn": {"id": "turn_limit", "status": "interrupted", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
