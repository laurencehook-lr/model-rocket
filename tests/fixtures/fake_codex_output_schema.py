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
        send({"id": request_id, "result": {"userAgent": "fake"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}, "requiresOpenaiAuth": True}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_1"}}})
    elif method == "turn/start":
        expected = {
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"],
            "additionalProperties": False,
        }
        if message["params"].get("outputSchema") != expected:
            send({"id": request_id, "error": {"code": -32602, "message": "outputSchema mismatch"}})
            continue
        send({"id": request_id, "result": {"turn": {"id": "turn_1", "status": "inProgress", "items": [], "error": None}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": '{"title":"bridge"}'}})
        usage = {
            "inputTokens": 10,
            "cachedInputTokens": 0,
            "outputTokens": 4,
            "reasoningOutputTokens": 0,
            "totalTokens": 14,
        }
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_1", "turnId": "turn_1", "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000}}})
        send({"method": "turn/completed", "params": {"turn": {"status": "completed"}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
