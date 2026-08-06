#!/usr/bin/env python3
import json
import sys


if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


def send(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


thread_counter = 0
thread_tools = {}
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-retained-sessions"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({"id": request_id, "result": {"account": {"type": "chatgpt"}, "requiresOpenaiAuth": True}})
    elif method == "model/list":
        send({"id": request_id, "result": {"data": [{"id": "gpt-5.6-sol", "model": "gpt-5.6-sol"}], "nextCursor": None}})
    elif method == "thread/start":
        thread_counter += 1
        thread_id = "thread_" + str(thread_counter)
        tools = message["params"].get("dynamicTools", [])
        thread_tools[thread_id] = tools[0]["name"] if tools else None
        send({"id": request_id, "result": {"thread": {"id": thread_id}}})
    elif method == "turn/start":
        thread_id = message["params"]["threadId"]
        if thread_id not in thread_tools:
            raise RuntimeError("turn/start used an unregistered thread ID")
        turn_id = "turn_" + thread_id.removeprefix("thread_")
        prompt = message["params"]["input"][0]["text"]
        send({"id": request_id, "result": {"turn": {"id": turn_id, "status": "inProgress", "items": [], "error": None}}})
        if "CALL_TOOL" in prompt:
            tool_name = thread_tools[thread_id]
            if not isinstance(tool_name, str) or not tool_name:
                raise RuntimeError("CALL_TOOL requires a registered non-empty tool name")
            send({"id": "tool_rpc_" + thread_id, "method": "item/tool/call", "params": {"arguments": {"city": "London"}, "callId": "call_" + thread_id, "threadId": thread_id, "tool": tool_name, "turnId": turn_id}})
        else:
            item_id = "item_" + thread_id
            send({"method": "item/agentMessage/delta", "params": {"delta": "fresh request succeeded", "itemId": item_id, "threadId": thread_id, "turnId": turn_id}})
            usage = {"inputTokens": 12, "cachedInputTokens": 0, "outputTokens": 3, "reasoningOutputTokens": 0, "totalTokens": 15}
            send({"method": "thread/tokenUsage/updated", "params": {"threadId": thread_id, "turnId": turn_id, "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 1000000}}})
            send({"method": "turn/completed", "params": {"threadId": thread_id, "turn": {"id": turn_id, "status": "completed", "items": []}}})
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "unknown method"}})
