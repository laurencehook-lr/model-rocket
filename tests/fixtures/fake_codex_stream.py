#!/usr/bin/env python3
import json
import sys
import time

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
        send({"id": request_id, "result": {"userAgent": "fake-stream"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_stream"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_stream"}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "éstreamed early"}})
        time.sleep(1)
        send({"method": "item/agentMessage/delta", "params": {"delta": " and completed"}})
        usage = {"inputTokens": 9, "outputTokens": 4}
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_stream", "turnId": "turn_stream", "tokenUsage": {"last": usage}}})
        send({"method": "turn/completed", "params": {"turn": {"status": "completed"}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
