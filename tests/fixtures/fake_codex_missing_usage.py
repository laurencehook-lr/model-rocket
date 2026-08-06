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
        send({"id": request_id, "result": {"userAgent": "fake-missing-usage"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_missing"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_missing"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "no usage", "itemId": "item_missing_usage", "threadId": "thread_missing", "turnId": "turn_missing"}})
        send({"method": "turn/completed", "params": {"threadId": "thread_missing", "turn": {"id": "turn_missing", "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
