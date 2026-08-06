#!/usr/bin/env python3
import json
import sys


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def run(*, user_agent, thread_id, turn_id, error_message, terminal_status):
    if sys.argv[1:] == ["--version"]:
        print("codex-cli 0.146.0")
        return

    for raw_line in sys.stdin:
        message = json.loads(raw_line)
        method = message.get("method")
        request_id = message.get("id")
        if method == "initialize":
            send({"id": request_id, "result": {"userAgent": user_agent}})
        elif method == "initialized":
            continue
        elif method == "account/read":
            send({"id": request_id, "result": {"account": {"type": "chatgpt"}}})
        elif method == "model/list":
            send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
        elif method == "thread/start":
            send({"id": request_id, "result": {"thread": {"id": thread_id}}})
        elif method == "turn/start":
            send({"id": request_id, "result": {"turn": {"id": turn_id}}})
            send({"method": "item/agentMessage/delta", "params": {"delta": "éstreamed beyond limit", "itemId": "item_fatal", "threadId": thread_id, "turnId": turn_id}})
        elif method == "turn/interrupt":
            params = message.get("params", {})
            if params.get("threadId") != thread_id:
                raise RuntimeError("turn/interrupt used the wrong thread ID")
            if params.get("turnId") != turn_id:
                raise RuntimeError("turn/interrupt used the wrong turn ID")
            send({"id": request_id, "result": {}})
            send({"method": "error", "params": {"error": {"message": error_message}, "threadId": thread_id, "turnId": turn_id, "willRetry": False}})
            send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": terminal_status, "items": []}}})
        else:
            send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
