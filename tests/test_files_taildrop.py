import json
import os
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

from panda_assistant import tools as T

try:
    import gi  # noqa: F401
    HAVE_GI = True
except ImportError:
    HAVE_GI = False

PROBE = textwrap.dedent("""
    import json
    from panda_assistant import tools as T
    def call(tool, **a):
        try:
            return T.run_tool(T.Context(), tool, a)
        except T.ToolError as e:
            return "error: " + str(e)
    print(json.dumps({
        "list_downloads": call("list_folder", path="Downloads"),
        "list_home": call("list_folder"),
        "find": call("find_files", name="faktura"),
        "read_rel": call("read_file", path="Dokumenty/notes.txt"),
        "read_alias": call("read_file", path="documents/notes.txt"),
        "read_tilde": call("read_file", path="~/Dokumenty/notes.txt"),
        "hidden": call("read_file", path="~/.ssh/id_ed25519"),
        "outside": call("read_file", path="/etc/hostname"),
        "escape": call("read_file", path="Dokumenty/../../../etc/hostname"),
        "binary": call("read_file", path="Pobrane/photo.jpg"),
        "find_hidden": call("find_files", name="id_ed25519"),
    }))
""")


@unittest.skipUnless(HAVE_GI, "needs PyGObject")
class FileToolsTest(unittest.TestCase):
    def test_paths_and_listing(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            (home / "Pobrane").mkdir()
            (home / "Dokumenty").mkdir()
            (home / ".ssh").mkdir()
            (home / ".config").mkdir()
            (home / ".config" / "user-dirs.dirs").write_text(
                'XDG_DOWNLOAD_DIR="$HOME/Pobrane"\nXDG_DOCUMENTS_DIR="$HOME/Dokumenty"\n')
            (home / "Pobrane" / "faktura-2026.pdf").write_bytes(b"%PDF-1.7")
            (home / "Pobrane" / "photo.jpg").write_bytes(b"\xff\xd8\x00\x00JFIF")
            (home / "Dokumenty" / "notes.txt").write_text("buy milk\n")
            (home / ".ssh" / "id_ed25519").write_text("SECRET")
            env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
                       PYTHONPATH=str(Path(__file__).parents[1] / "src"))
            out = json.loads(subprocess.run([sys.executable, "-c", PROBE], env=env, capture_output=True,
                                            text=True, check=True).stdout)
        self.assertIn("~/Pobrane", out["list_downloads"])
        self.assertIn("faktura-2026.pdf", out["list_downloads"])
        self.assertNotIn(".ssh", out["list_home"])
        self.assertIn("~/Pobrane/faktura-2026.pdf", out["find"])
        for key in ("read_rel", "read_alias", "read_tilde"):
            self.assertIn("buy milk", out[key], key)
        self.assertIn("hidden", out["hidden"])
        self.assertIn("home folder", out["outside"])
        self.assertIn("home folder", out["escape"])
        self.assertIn("isn't a text file", out["binary"])
        self.assertNotIn("SECRET", json.dumps(out))
        self.assertIn("No files", out["find_hidden"])


FAKE_TAILSCALE = textwrap.dedent("""\
    #!{python}
    import json, os, sys
    args = sys.argv[1:]
    if args[:2] == ["status", "--json"]:
        print(json.dumps({{"Peer": {{
            "a": {{"HostName": "Pixel-9", "DNSName": "pixel-9.tail1234.ts.net.", "OS": "android", "Online": True}},
            "b": {{"HostName": "nas", "DNSName": "nas.tail1234.ts.net.", "OS": "linux", "Online": False}}}}}}))
    elif args[:2] == ["file", "cp"]:
        if os.environ.get("FAKE_TS_DENY"):
            print("sending file: Access denied: file access denied", file=sys.stderr); sys.exit(1)
        open(os.environ["FAKE_TS_LOG"], "a").write(" ".join(args) + "\\n")
    else:
        sys.exit(2)
""")


class TaildropTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = Path(self.tmp.name)
        ts = d / "bin" / "tailscale"
        ts.parent.mkdir()
        ts.write_text(FAKE_TAILSCALE.format(python=sys.executable))
        ts.chmod(ts.stat().st_mode | stat.S_IEXEC)
        self.home = d / "home"
        (self.home / "Docs").mkdir(parents=True)
        (self.home / "Docs" / "trip.pdf").write_bytes(b"%PDF")
        self.log = d / "ts.log"
        self.old = dict(os.environ)
        os.environ.update(PATH=f"{ts.parent}:{os.environ['PATH']}", HOME=str(self.home), FAKE_TS_LOG=str(self.log))

    def tearDown(self):
        os.environ.clear()
        os.environ.update(self.old)
        self.tmp.cleanup()

    def test_list(self):
        out = T.run_tool(T.Context(), "list_devices", {})
        self.assertEqual(out.splitlines(), ["Pixel-9 (android, online)", "nas (linux, offline)"])

    def test_send(self):
        out = T.run_tool(T.Context(), "send_file", {"path": "Docs/trip.pdf", "device": "pixel-9"})
        self.assertEqual(out, "Sent trip.pdf to Pixel-9 with Taildrop.")
        self.assertEqual(self.log.read_text().split(), ["file", "cp", str(self.home / "Docs" / "trip.pdf"), "pixel-9:"])

    def test_send_needs_confirmation(self):
        self.assertEqual(T.BY_NAME["send_file"].risk, T.CONFIRM)

    def test_offline_unknown_and_folders(self):
        for args, msg in (({"path": "Docs/trip.pdf", "device": "nas"}, "offline"),
                          ({"path": "Docs/trip.pdf", "device": "toaster"}, "no device"),
                          ({"path": "Docs", "device": "pixel-9"}, "isn't a file")):
            with self.assertRaises(T.ToolError) as err:
                T.run_tool(T.Context(), "send_file", args)
            self.assertIn(msg, str(err.exception))
        self.assertFalse(self.log.exists())

    def test_operator_hint(self):
        os.environ["FAKE_TS_DENY"] = "1"
        with self.assertRaises(T.ToolError) as err:
            T.run_tool(T.Context(), "send_file", {"path": "Docs/trip.pdf", "device": "Pixel-9"})
        self.assertIn("tailscale set --operator", str(err.exception))


if __name__ == "__main__":
    unittest.main()
