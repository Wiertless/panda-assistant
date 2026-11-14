import json
import os
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from fake_flm import FakeFlm, text_reply, tool_reply
from panda_assistant import mcp
from panda_assistant import tools as T
from panda_assistant.agent import Agent, Callbacks
from panda_assistant.flm import FlmClient

SERVER = str(Path(__file__).with_name("fake_mcp_server.py"))


class Env(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.old = {k: os.environ.get(k) for k in ("XDG_CONFIG_HOME", "XDG_STATE_HOME")}
        os.environ["XDG_CONFIG_HOME"] = self.tmp.name + "/cfg"
        os.environ["XDG_STATE_HOME"] = self.tmp.name + "/state"

    def tearDown(self):
        for k, v in self.old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        self.tmp.cleanup()


class ConfigTest(Env):
    def test_round_trip_and_parse(self):
        self.assertEqual(mcp.load_config(), [])
        mcp.save_config([mcp.parse_server("notes", f"{sys.executable} '{SERVER}' --x"),
                         mcp.parse_server("home", "http://localhost:1/mcp")])
        cfgs = mcp.load_config()
        self.assertEqual(cfgs[0].command, sys.executable)
        self.assertEqual(cfgs[0].args, [SERVER, "--x"])
        self.assertEqual(cfgs[1].url, "http://localhost:1/mcp")
        self.assertEqual(oct(mcp.config_path().stat().st_mode & 0o777), "0o600")
        raw = json.loads(mcp.config_path().read_text())
        self.assertIn("mcpServers", raw)

    def test_bad_input(self):
        with self.assertRaises(mcp.McpError):
            mcp.parse_server("", "x")
        mcp.config_path().parent.mkdir(parents=True)
        mcp.config_path().write_text("{nope")
        hub = mcp.Hub()
        self.assertIn("isn't valid JSON", hub.config_error)

    def test_public_names_are_valid(self):
        self.assertEqual(mcp._public_name("My Notes!", "add.note"), "My_Notes__add_note")
        self.assertLessEqual(len(mcp._public_name("s" * 80, "t" * 80)), 64)


class StdioTest(Env):
    def setUp(self):
        super().setUp()
        mcp.save_config([mcp.ServerConfig("notes", command=sys.executable, args=[SERVER])])
        self.hub = mcp.Hub()

    def tearDown(self):
        self.hub.stop_all()
        super().tearDown()

    def test_start_list_call(self):
        self.assertEqual(self.hub.start_all(), [])
        server = self.hub.servers["notes"]
        self.assertEqual(server.label, "Notes")
        self.assertEqual([t.public_name for t in server.tools], ["notes__get_note", "notes__add_note"])
        self.assertNotIn("$schema", server.tools[0].schema)
        tools = mcp.as_tools(self.hub)
        self.assertEqual([t.risk for t in tools], [T.SAFE, T.CONFIRM])
        ctx = T.Context()
        self.assertEqual(tools[0].run(ctx, {"title": "shop"}), "Note shop: buy milk")
        self.assertEqual(ctx.images, ["data:image/png;base64,aGk="])
        with self.assertRaises(T.ToolError):
            tools[0].run(ctx, {"title": "missing"})
        self.assertEqual(tools[1].summary({}), "Notes: add.note")

    def test_disabled_and_broken_servers(self):
        mcp.save_config([mcp.ServerConfig("notes", command=sys.executable, args=[SERVER], disabled=True),
                         mcp.ServerConfig("broken", command="/nonexistent/server")])
        self.hub.reload()
        problems = self.hub.start_all()
        self.assertEqual(len(problems), 1)
        self.assertIn("broken", problems[0])
        self.assertEqual(mcp.as_tools(self.hub), [])
        self.assertEqual(self.hub.start_all(), [])  # a failure isn't retried every message

    def test_trust_skips_confirmation(self):
        mcp.save_config([mcp.ServerConfig("notes", command=sys.executable, args=[SERVER], trust=True)])
        self.hub.reload()
        self.hub.start_all()
        self.assertEqual({t.risk for t in mcp.as_tools(self.hub)}, {T.SAFE})

    def test_agent_uses_mcp_tools(self):
        self.hub.start_all()
        flm = FakeFlm()
        try:
            flm.replies = [(200, tool_reply("notes__add_note", '{"text": "milk"}')), (200, text_reply("Added it."))]
            agent = Agent(FlmClient(flm.url), "m", extra_tools=lambda: mcp.as_tools(self.hub))
            asked = []
            cb = Callbacks(confirm=lambda n, s, d: asked.append((n, s, d)) or True)
            agent.ask("note milk", cb)
            names = [t["function"]["name"] for t in flm.requests[0]["tools"]]
            self.assertIn("notes__get_note", names)
            self.assertEqual(asked[0][1], "Notes: add.note")
            self.assertIn('"text": "milk"', asked[0][2])
            self.assertEqual(flm.requests[1]["messages"][-1]["content"], "Added.")
        finally:
            flm.close()


class HttpTest(Env):
    def setUp(self):
        super().setUp()
        sessions = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_DELETE(self):
                self.send_response(200)
                self.end_headers()

            def do_POST(self):
                msg = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                sessions.append(self.headers.get("Mcp-Session-Id"))
                if "id" not in msg:
                    self.send_response(202)
                    self.end_headers()
                    return
                if msg["method"] == "initialize":
                    result = {"protocolVersion": "x", "serverInfo": {"name": "web"}}
                elif msg["method"] == "tools/list":
                    result = {"tools": [{"name": "ping", "inputSchema": {"type": "object"}}]}
                else:
                    result = {"content": [{"type": "text", "text": "pong"}]}
                body = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result})
                self.send_response(200)
                self.send_header("Mcp-Session-Id", "abc")
                if msg["method"] == "tools/call":  # answer as an SSE stream
                    self.send_header("Content-Type", "text/event-stream")
                    self.end_headers()
                    self.wfile.write(f"event: message\ndata: {body}\n\n".encode())
                else:
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(body.encode())

        self.sessions = sessions
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def tearDown(self):
        self.httpd.shutdown()
        self.httpd.server_close()
        super().tearDown()

    def test_http_server(self):
        mcp.save_config([mcp.ServerConfig("web", url=f"http://127.0.0.1:{self.httpd.server_port}/mcp")])
        hub = mcp.Hub()
        self.assertEqual(hub.start_all(), [])
        tool = mcp.as_tools(hub)[0]
        self.assertEqual(tool.run(T.Context(), {}), "pong")
        self.assertEqual(self.sessions[0], None)
        self.assertEqual(self.sessions[-1], "abc")
        hub.stop_all()


if __name__ == "__main__":
    unittest.main()
