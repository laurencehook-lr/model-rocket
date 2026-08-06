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
        send({"id": request_id, "result": {"userAgent": "fake-terminal-overflow"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_terminal_overflow"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_terminal_overflow"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": " configu", "itemId": "item_terminal_overflow", "threadId": "thread_terminal_overflow", "turnId": "turn_terminal_overflow"}})
        usage = {"inputTokens": 4, "cachedInputTokens": 0, "outputTokens": 2, "reasoningOutputTokens": 0, "totalTokens": 6}
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_terminal_overflow", "turnId": "turn_terminal_overflow", "tokenUsage": {"last": usage, "total": usage}}})
        send({"method": "turn/completed", "params": {"threadId": "thread_terminal_overflow", "turn": {"id": "turn_terminal_overflow", "status": "completed", "items": []}}})
    elif method == "turn/interrupt":
        raise RuntimeError("terminal overflow was already complete and must not be interrupted")
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
