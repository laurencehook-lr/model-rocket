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
        send({"id": request_id, "result": {"userAgent": "fake-completion-forbidden"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_forbidden_completion"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_forbidden_completion"}}})
        send({"method": "turn/completed", "params": {"threadId": "thread_forbidden_completion", "turn": {"id": "turn_forbidden_completion", "status": "completed", "items": [{"id": "item_allowed", "type": "agentMessage", "text": ""}, {"id": "item_forbidden", "type": "commandExecution", "command": "pwd"}]}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
