#!/usr/bin/env python3
import json
import pathlib
import re
import sys


if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def complete_batch(batch):
    for request_id, thread_id, turn_id in batch:
        send({"id": request_id, "result": {"turn": {"id": turn_id, "status": "inProgress", "items": [], "error": None}}})
    for _, thread_id, turn_id in reversed(batch):
        item_id = "item_" + turn_id
        text = "hello from " + thread_id
        send({"method": "item/started", "params": {"threadId": thread_id, "turnId": turn_id, "startedAtMs": 1, "item": {"id": item_id, "type": "agentMessage", "text": ""}}})
        send({"method": "item/agentMessage/delta", "params": {"delta": text, "itemId": item_id, "threadId": thread_id, "turnId": turn_id}})
        send({"method": "item/completed", "params": {"threadId": thread_id, "turnId": turn_id, "completedAtMs": 2, "item": {"id": item_id, "type": "agentMessage", "text": text}}})
        usage = {"inputTokens": 12, "cachedInputTokens": 0, "outputTokens": 3, "reasoningOutputTokens": 0, "totalTokens": 15}
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": thread_id, "turnId": turn_id, "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000}}})
        send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "completed", "items": []}}})


limits_source = (
    pathlib.Path(__file__).resolve().parents[2]
    / "src"
    / "policies"
    / "router_limits.rs"
).read_text(encoding="utf-8")
limit_match = re.search(r"MAX_CONCURRENT_GPT_TURNS: usize = ([0-9]+);", limits_source)
if limit_match is None:
    raise RuntimeError("cannot read MAX_CONCURRENT_GPT_TURNS from router policy")
max_concurrent_turns = int(limit_match.group(1))


thread_counter = 0
pending_turns = []
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-multiplex"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}, "requiresOpenaiAuth": True}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        thread_counter += 1
        send({"id": request_id, "result": {"thread": {"id": "thread_" + str(thread_counter)}}})
    elif method == "turn/start":
        thread_id = message["params"]["threadId"]
        turn_id = "turn_" + thread_id.removeprefix("thread_")
        pending_turns.append((request_id, thread_id, turn_id))
        if len(pending_turns) == max_concurrent_turns:
            complete_batch(pending_turns)
            pending_turns = []
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})

if pending_turns:
    complete_batch(pending_turns)
