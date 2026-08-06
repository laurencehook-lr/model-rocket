#!/usr/bin/env python3
import json
import os
import sys
import tempfile


if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


state_path = os.path.join(tempfile.gettempdir(), "model-rocket-recovery-" + str(os.getppid()) + ".state")
try:
    with open(state_path, encoding="utf-8") as state_file:
        launch_number = int(state_file.read()) + 1
except FileNotFoundError:
    launch_number = 1
with open(state_path, "w", encoding="utf-8") as state_file:
    state_file.write(str(launch_number))


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-recovery"}})
    elif method == "initialized":
        continue
    elif launch_number == 1 and method == "thread/start":
        sys.exit(70)
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}, "requiresOpenaiAuth": True}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_recovered"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_recovered", "status": "inProgress", "items": [], "error": None}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": "recovered process succeeded", "itemId": "item_recovered", "threadId": "thread_recovered", "turnId": "turn_recovered"}})
        usage = {"inputTokens": 12, "cachedInputTokens": 0, "outputTokens": 3, "reasoningOutputTokens": 0, "totalTokens": 15}
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread_recovered", "turnId": "turn_recovered", "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000}}})
        send({"method": "turn/completed", "params": {"threadId": "thread_recovered", "turn": {"id": "turn_recovered", "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
