#!/usr/bin/env python3
import http.server
import os
import signal
import socketserver
import sys


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
    "LAUNCHER_PROOF_FILE",
    "MODEL_ROCKET_BRIDGE_BIN",
)
leaked = [name for name in forbidden if os.environ.get(name)]
if leaked:
    sys.stderr.write("bridge inherited forbidden environment: " + ",".join(leaked) + "\n")
    sys.exit(73)


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
