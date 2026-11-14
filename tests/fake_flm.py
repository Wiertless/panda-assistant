"""A tiny stand-in for FastFlowLM's OpenAI endpoint, scripted per test."""

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def sse(chunks):
    out = []
    for c in chunks:
        out.append("data: " + json.dumps(c) + "\n\n")
    out.append("data: [DONE]\n\n")
    return "".join(out).encode()


def text_reply(*parts, reasoning=None):
    chunks = []
    if reasoning:
        chunks.append({"choices": [{"delta": {"reasoning_content": reasoning}}]})
    chunks += [{"choices": [{"delta": {"content": p}}]} for p in parts]
    chunks.append({"choices": [{"delta": {"content": None}, "finish_reason": "stop"}]})
    return chunks


def tool_reply(name, args_json, call_id="call_1"):
    half = len(args_json) // 2
    return [
        {"choices": [{"delta": {"tool_calls": [{"index": 0, "id": call_id, "type": "function",
                                                "function": {"name": name, "arguments": ""}}]}}]},
        {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": args_json[:half]}}]}}]},
        {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": args_json[half:]}}]}}]},
        {"choices": [{"delta": {"content": None}, "finish_reason": "tool_calls"}]},
    ]


class FakeFlm:
    """Serves queued replies in order; records every request body."""

    def __init__(self):
        self.replies = []
        self.requests = []
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                if self.path == "/api/version":
                    self._send(200, b'{"version":"test"}', "application/json")
                else:
                    self._send(404, b"{}", "application/json")

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fake.requests.append(body)
                status, payload = fake.replies.pop(0)
                if status != 200:
                    self._send(status, json.dumps({"error": payload}).encode(), "application/json")
                else:
                    self._send(200, sse(payload), "text/event-stream")

            def _send(self, status, data, ctype):
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
