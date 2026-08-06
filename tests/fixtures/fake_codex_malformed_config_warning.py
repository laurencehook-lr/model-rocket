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
        send({"id": request_id, "result": {"userAgent": "fake-malformed-warning"}})
    elif method == "initialized":
        send({"method": "configWarning", "result": None, "params": {"summary": "malformed warning"}})
    elif method == "thread/start":
        send({"id": request_id, "result": {"thread": {"id": "thread_warning"}}})
    elif method == "turn/start":
        send({"id": request_id, "result": {"turn": {"id": "turn_warning"}}})
        send({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": "must not escape",
                "itemId": "item_warning",
                "threadId": "thread_warning",
                "turnId": "turn_warning",
            },
        })
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
