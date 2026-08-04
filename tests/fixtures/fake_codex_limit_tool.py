#!/usr/bin/env python3
import json
import sys

if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


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
        send({"id": request_id, "result": {"thread": {"id": "thread_limit"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_limit"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "éstreamed beyond limit"}})
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_limit", "turnId": "turn_limit", "tokenUsage": {"last": {"inputTokens": 9, "outputTokens": 4}}}})
        send({"id": "tool_after_limit", "method": "item/tool/call", "params": {"arguments": {"city": "London"}, "callId": "call_after_limit", "threadId": "thread_limit", "tool": "weather", "turnId": "turn_limit"}})
        send({"method": "turn/completed", "params": {"turn": {"status": "interrupted"}}})
    elif method == "turn/interrupt":
        send({"id": request_id, "result": {}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
