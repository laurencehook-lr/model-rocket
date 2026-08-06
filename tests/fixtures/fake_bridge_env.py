#!/usr/bin/env python3
import http.server
import hashlib
import json
import os
import signal
import socketserver
import sys


ambient_forbidden = (
    "OPENAI_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "MODEL_ROCKET_BRIDGE_BIN",
)
ambient_leaks = [name for name in ambient_forbidden if os.environ.get(name)]
if ambient_leaks:
    sys.stderr.write(
        "bridge command inherited forbidden environment: "
        + ",".join(ambient_leaks)
        + "\n"
    )
    sys.exit(72)


if len(sys.argv) > 1 and sys.argv[1] == "launcher-contract":
    print("default_claude_model=claude-fable-5")
    print("canonical_gpt_model=anthropic-model-rocket-gpt-5.6-sol-normal-high")
    print("required_claude_version=2.1.223 (Claude Code)")
    print("gpt_context_tokens=272000")
    print("bridge_startup_attempts=300")
    sys.exit(0)


if len(sys.argv) == 5 and sys.argv[1] == "launcher-settings":
    bridge_bin, base_url, proxy_bypass = sys.argv[2:]
    with open(os.environ["MODEL_ROCKET_CONFIG"], encoding="utf-8") as config_handle:
        catalogue = json.load(config_handle)
    canonical_id = catalogue["canonical_route"]
    canonical = next(route for route in catalogue["routes"] if route["id"] == canonical_id)
    settings_environment = {
        name: ""
        for name in (
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_CUSTOM_HEADERS",
            "ANTHROPIC_CUSTOM_MODEL_OPTION_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_DESCRIPTION",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_SUPPORTED_CAPABILITIES",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
            "CLAUDE_CODE_USE_GATEWAY",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_MANTLE",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
            "CLAUDE_CODE_OAUTH_SCOPES",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        )
    }
    contexts = {model["id"]: model["context_tokens"] for model in catalogue["models"]}
    settings_environment.update({
        "ANTHROPIC_BASE_URL": base_url,
        "ANTHROPIC_CUSTOM_MODEL_OPTION": canonical_id,
        "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME": canonical["display_name"],
        "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION": canonical["description"],
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS": str(min(contexts.values())),
        "NO_PROXY": proxy_bypass,
        "no_proxy": proxy_bypass,
    })
    json.dump({
        "apiKeyHelper": "",
        "fallbackModel": [],
        "hooks": {"ConfigChange": [{
            "matcher": "user_settings|project_settings|local_settings",
            "hooks": [{
                "type": "command",
                "command": bridge_bin + " guard-settings-change",
                "timeout": 5,
            }],
        }]},
        "availableModels": ["fable", "opus", "sonnet", "haiku"]
        + [route["id"] for route in catalogue["routes"]],
        "env": settings_environment,
    }, sys.stdout)
    print()
    sys.exit(0)


if len(sys.argv) > 1 and sys.argv[1] == "validate-settings":
    for path in sys.argv[2:]:
        try:
            with open(path, encoding="utf-8") as settings_handle:
                settings = json.load(settings_handle)
        except FileNotFoundError:
            continue
        if settings.get("modelOverrides"):
            sys.stderr.write(
                "configuration error: Claude modelOverrides is incompatible with Model Rocket: "
                + path
                + "\n"
            )
            sys.exit(1)
        if settings.get("disableAllHooks") is True:
            sys.stderr.write(
                "configuration error: Claude disableAllHooks=true is incompatible with Model Rocket: "
                + path
                + "\n"
            )
            sys.exit(1)
    sys.exit(0)


if len(sys.argv) == 3 and sys.argv[1] == "validate-claude":
    claude_path = os.path.realpath(sys.argv[2])
    with open(claude_path, "rb") as claude_handle:
        digest = hashlib.sha256(claude_handle.read()).hexdigest()
    expected = os.environ.get(
        "MODEL_ROCKET_TEST_CLAUDE_SHA256",
        "05d117bf8a34403faa57b73c6e68c11a2ccb5add172cd72cfb46a55af88d621a",
    )
    if digest != expected:
        sys.stderr.write("configuration error: Claude Code executable digest changed\n")
        sys.exit(1)
    print(claude_path)
    sys.exit(0)


if len(sys.argv) == 2 and sys.argv[1] == "preflight":
    try:
        with open(os.environ["MODEL_ROCKET_CONFIG"], encoding="utf-8") as config_handle:
            catalogue = json.load(config_handle)
        models = [model["id"] for model in catalogue["models"]]
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        sys.stderr.write("configuration error: invalid model catalogue: " + str(error) + "\n")
        sys.exit(1)
    unavailable = next((model for model in models if model == "gpt-unavailable"), None)
    if unavailable is not None:
        sys.stderr.write(
            "unavailable: model "
            + unavailable
            + " is not available to the managed ChatGPT account\n"
        )
        sys.exit(1)
    json.dump({"account_type": "chatgpt", "models": models}, sys.stdout)
    print()
    sys.exit(0)


forbidden = (
    "OPENAI_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "GITHUB_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    "CLAUDE_CODE_USE_GATEWAY",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_MANTLE",
    "LAUNCHER_PROOF_FILE",
    "MODEL_ROCKET_BRIDGE_BIN",
)
leaked = [name for name in forbidden if os.environ.get(name)]
if leaked:
    sys.stderr.write("bridge inherited forbidden environment: " + ",".join(leaked) + "\n")
    sys.exit(73)
codex_bin = os.environ.get("MODEL_ROCKET_CODEX_BIN", "")
if not os.path.isabs(codex_bin) or not codex_bin.endswith("/tests/fixtures/fake_codex.py"):
    sys.stderr.write("bridge did not receive the pinned absolute Codex fixture path\n")
    sys.exit(75)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(204)
            self.end_headers()
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, _format, *args):
        return


class Server(socketserver.TCPServer):
    allow_reuse_address = False


server = Server(("127.0.0.1", 0), Handler)
ready_file = os.environ["MODEL_ROCKET_READY_FILE"]
with open(ready_file, "w", encoding="utf-8") as ready:
    ready.write(f"127.0.0.1:{server.server_address[1]}")


def stop(_signum, _frame):
    sys.exit(0)


signal.signal(signal.SIGINT, stop)
signal.signal(signal.SIGTERM, stop)
server.serve_forever()
