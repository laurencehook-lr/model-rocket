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
        send({"id": request_id, "result": {"userAgent": "fake-orphan-notification"}})
    elif method == "initialized":
        send({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": "must not escape",
                "itemId": "item_orphan",
                "threadId": "thread_orphan",
                "turnId": "turn_orphan",
            },
        })
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_active"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_active"}}})
        send({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": "normal response",
                "itemId": "item_active",
                "threadId": "thread_active",
                "turnId": "turn_active",
            },
        })
        send({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread_active",
                "turnId": "turn_active",
                "tokenUsage": {
                    "last": {
                        "inputTokens": 1,
                        "cachedInputTokens": 0,
                        "outputTokens": 1,
                        "reasoningOutputTokens": 0,
                        "totalTokens": 2,
                    },
                    "total": {
                        "inputTokens": 1,
                        "cachedInputTokens": 0,
                        "outputTokens": 1,
                        "reasoningOutputTokens": 0,
                        "totalTokens": 2,
                    },
                },
            },
        })
        send({
            "method": "turn/completed",
            "params": {
                "threadId": "thread_active",
                "turn": {
                    "id": "turn_active",
                    "items": [{
                        "id": "item_active",
                        "type": "agentMessage",
                        "text": "normal response",
                    }],
                    "status": "completed",
                },
            },
        })
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
