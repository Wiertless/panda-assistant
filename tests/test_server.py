import os
import socket
import stat
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

from panda_assistant.flm import FlmClient, serve_command

FAKE_FLM = textwrap.dedent("""\
    #!{python}
    import os, sys
    from http.server import BaseHTTPRequestHandler, HTTPServer
    with open(os.environ["FAKE_FLM_LOG"], "a") as f:
        f.write(" ".join(sys.argv[1:]) + "\\n")
    if sys.argv[1:2] != ["serve"]:
        sys.exit(0)
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a): pass
        def do_GET(self):
            self.send_response(200); self.end_headers(); self.wfile.write(b"{{}}")
    HTTPServer(("127.0.0.1", int(os.environ["FAKE_FLM_PORT"])), H).serve_forever()
""")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class ServeCommandTest(unittest.TestCase):
    def test_context_length(self):
        self.assertEqual(serve_command("flm", "gemma4-it:e4b", 8192),
                         ["flm", "serve", "gemma4-it:e4b", "--ctx-len", "8192"])
        self.assertEqual(serve_command("flm", "gemma4-it:e4b", None), ["flm", "serve", "gemma4-it:e4b"])

    def test_prefix_cache_capped_for_qwen35(self):
        self.assertIn("--prefix-cache-mb", serve_command("flm", "qwen3.5:4b", 8192))
        self.assertIn("--prefix-cache-mb", serve_command("flm", "qwen3.6-moe:35b-a3b", 8192))
        self.assertNotIn("--prefix-cache-mb", serve_command("flm", "qwen3:4b", 8192))


class ServerLifecycleTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        bin_dir = Path(self.tmp.name)
        flm = bin_dir / "flm"
        flm.write_text(FAKE_FLM.format(python=sys.executable))
        flm.chmod(flm.stat().st_mode | stat.S_IEXEC)
        self.log = bin_dir / "calls.log"
        self.port = free_port()
        self.env = {"PATH": f"{bin_dir}:{os.environ['PATH']}", "FAKE_FLM_LOG": str(self.log),
                    "FAKE_FLM_PORT": str(self.port), "XDG_STATE_HOME": str(bin_dir / "state")}
        self.old = {k: os.environ.get(k) for k in self.env}
        os.environ.update(self.env)
        self.client = FlmClient(f"http://127.0.0.1:{self.port}")

    def tearDown(self):
        self.client.stop_server()
        for k, v in self.old.items():
            os.environ.pop(k, None) if v is None else os.environ.__setitem__(k, v)
        self.tmp.cleanup()

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_start_reuse_restart_and_unload(self):
        self.assertTrue(self.client.ensure_server("gemma4-it:e4b", 8192, wait=20))
        self.assertTrue(self.client.owns_server)
        self.assertEqual(self.calls(), ["serve gemma4-it:e4b --ctx-len 8192"])

        # Already running with the same settings: nothing is started.
        self.assertFalse(self.client.ensure_server("gemma4-it:e4b", 8192, wait=20))
        self.assertEqual(len(self.calls()), 1)

        # A new context length means a restart.
        self.assertTrue(self.client.ensure_server("gemma4-it:e4b", 4096, wait=20))
        self.assertEqual(self.calls()[-1], "serve gemma4-it:e4b --ctx-len 4096")

        self.assertTrue(self.client.stop_server())
        self.assertFalse(self.client.owns_server)
        deadline = time.monotonic() + 5
        while self.client.is_up() and time.monotonic() < deadline:
            time.sleep(0.1)
        self.assertFalse(self.client.is_up())
        self.assertFalse(self.client.stop_server())

    def test_server_started_elsewhere_is_left_alone(self):
        import subprocess
        other = subprocess.Popen(["flm", "serve", "x"])
        try:
            deadline = time.monotonic() + 10
            while not self.client.is_up() and time.monotonic() < deadline:
                time.sleep(0.1)
            self.assertFalse(self.client.ensure_server("gemma4-it:e4b", 8192, wait=5))
            self.assertFalse(self.client.owns_server)
            self.assertFalse(self.client.stop_server())
            self.assertIsNone(other.poll())
        finally:
            other.terminate()
            other.wait()


if __name__ == "__main__":
    unittest.main()
