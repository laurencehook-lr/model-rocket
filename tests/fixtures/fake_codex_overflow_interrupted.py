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
        send({"id": request_id, "result": {"userAgent": "fake-overflow-interrupted"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_overflow"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_overflow"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "éstreamed beyond limit", "itemId": "item_overflow", "threadId": "thread_overflow", "turnId": "turn_overflow"}})
    elif method == "turn/interrupt":
        params = message.get("params", {})
        if params.get("threadId") != "thread_overflow":
            raise RuntimeError("turn/interrupt used the wrong thread ID")
        if params.get("turnId") != "turn_overflow":
            raise RuntimeError("turn/interrupt used the wrong turn ID")
        send({"id": request_id, "result": {}})
        usage = {"inputTokens": 12, "cachedInputTokens": 0, "outputTokens": 2, "reasoningOutputTokens": 0, "totalTokens": 14}
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_overflow", "turnId": "turn_overflow", "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000}}})
        send({"method": "turn/completed", "params": {"threadId": "thread_overflow", "turn": {"id": "turn_overflow", "status": "interrupted", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
