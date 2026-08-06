#!/usr/bin/env python3
import json
import os
import sys
import tempfile


if sys.argv[1:] == ["--version"]:
    print("codex-cli 0.146.0")
    sys.exit(0)


catalogue_path = None
for argument in sys.argv[1:]:
    if argument.startswith("model_catalog_json="):
        catalogue_path = json.loads(argument.split("=", 1)[1])
        break
if catalogue_path is None:
    raise SystemExit("missing model_catalog_json override")
with open(catalogue_path, encoding="utf-8") as catalogue_file:
    configured_catalogue = json.load(catalogue_file)
configured_models = {
    model["slug"]
    for model in configured_catalogue["models"]
}
expected_models = {"gpt-a", "gpt-b"}
if configured_models != expected_models:
    raise SystemExit(
        "unexpected restricted model catalogue: "
        + ",".join(sorted(configured_models))
    )


state_path = os.path.join(
    tempfile.gettempdir(),
    "model-rocket-two-model-recovery-" + str(os.getppid()) + ".state",
)
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


thread_starts = 0
active_model = None
for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params", {})
    if method == "initialize":
        send({"id": request_id, "result": {"userAgent": "fake-two-model-recovery"}})
    elif method == "initialized":
        continue
    elif method == "account/read":
        send({
            "id": request_id,
            "result": {
                "account": {"type": "chatgpt"},
                "requiresOpenaiAuth": True,
            },
        })
    elif method == "model/list":
        send({
            "id": request_id,
            "result": {
                "data": [
                    {"id": model, "model": model}
                    for model in sorted(configured_models)
                ],
                "nextCursor": None,
            },
        })
    elif method == "thread/start":
        thread_starts += 1
        if launch_number == 1 and thread_starts == 2:
            sys.exit(70)
        active_model = params.get("model")
        if active_model not in configured_models:
            raise SystemExit("thread/start used an unconfigured model")
        send({
            "id": request_id,
            "result": {"thread": {"id": "thread_" + str(launch_number)}},
        })
    elif method == "turn/start":
        turn_model = params.get("model")
        if turn_model != active_model:
            raise SystemExit("turn/start model does not match thread/start")
        thread_id = params.get("threadId")
        turn_id = "turn_" + str(launch_number)
        send({
            "id": request_id,
            "result": {
                "turn": {
                    "id": turn_id,
                    "status": "inProgress",
                    "items": [],
                    "error": None,
                },
            },
        })
        send({
            "method": "item/agentMessage/delta",
            "params": {
                "delta": active_model + " via launch " + str(launch_number),
                "itemId": "item_" + str(launch_number),
                "threadId": thread_id,
                "turnId": turn_id,
            },
        })
        usage = {
            "inputTokens": 12,
            "cachedInputTokens": 0,
            "outputTokens": 5,
            "reasoningOutputTokens": 0,
            "totalTokens": 17,
        }
        send({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": thread_id,
                "turnId": turn_id,
                "tokenUsage": {
                    "last": usage,
                    "total": usage,
                    "modelContextWindow": 1000000,
                },
            },
        })
        send({
            "method": "turn/completed",
            "params": {
                "threadId": thread_id,
                "turn": {
                    "id": turn_id,
                    "status": "completed",
                    "items": [],
                },
            },
        })
    else:
        send({
            "id": request_id,
            "error": {"code": -32601, "message": "unknown method"},
        })
