import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

try:
    import gi  # noqa: F401
    HAVE_GI = True
except ImportError:
    HAVE_GI = False

APPS = {
    "fake-mail.desktop": ("Owl Mail", "x-scheme-handler/mailto;", "Email Client", "mail;inbox;"),
    "fake-files.desktop": ("Burrow", "inode/directory;", "File Manager", "folder;"),
    "fake-web.desktop": ("Otter Browser", "x-scheme-handler/https;x-scheme-handler/http;", "Web Browser", "web;"),
    "fake-notes.desktop": ("Scribble", "text/plain;", "Text Editor", "notes;writing;"),
}

PROBE = textwrap.dedent("""
    import json
    from panda_assistant import tools as T
    out = {"defaults": T.default_apps()}
    for name in ("email", "my files", "the browser", "file manager", "writing", "Scribble"):
        try:
            out[name] = T._find_app(name).get_display_name()
        except T.ToolError as e:
            out[name] = "error: " + str(e)
    from panda_assistant.context import system_context
    out["context"] = system_context()
    print(json.dumps(out))
""")


@unittest.skipUnless(HAVE_GI, "needs PyGObject")
class DefaultAppsTest(unittest.TestCase):
    def test_roles_resolve_to_defaults(self):
        with tempfile.TemporaryDirectory() as tmp:
            data, config = Path(tmp, "data"), Path(tmp, "config")
            (data / "applications").mkdir(parents=True)
            config.mkdir()
            for fname, (name, mimes, generic, keywords) in APPS.items():
                (data / "applications" / fname).write_text(
                    f"[Desktop Entry]\nType=Application\nName={name}\nGenericName={generic}\n"
                    f"Keywords={keywords}\nExec=true %u\nMimeType={mimes}\n")
            (config / "mimeapps.list").write_text(
                "[Default Applications]\nx-scheme-handler/mailto=fake-mail.desktop\n"
                "inode/directory=fake-files.desktop\nx-scheme-handler/https=fake-web.desktop\n"
                "text/plain=fake-notes.desktop\n")
            env = dict(os.environ, XDG_DATA_HOME=str(data), XDG_DATA_DIRS=str(data), XDG_CONFIG_HOME=str(config),
                       XDG_CONFIG_DIRS=str(config), PYTHONPATH=str(Path(__file__).parents[1] / "src"))
            out = json.loads(subprocess.run([sys.executable, "-c", PROBE], env=env, capture_output=True,
                                            text=True, check=True).stdout)
        self.assertEqual(out["email"], "Owl Mail")
        self.assertEqual(out["my files"], "Burrow")
        self.assertEqual(out["the browser"], "Otter Browser")
        self.assertEqual(out["file manager"], "Burrow")
        self.assertEqual(out["writing"], "Scribble")   # by keyword
        self.assertEqual(out["Scribble"], "Scribble")  # by name
        self.assertEqual(out["defaults"]["email"], "Owl Mail")
        self.assertIn("Default apps:", out["context"])
        self.assertIn("email: Owl Mail", out["context"])


if __name__ == "__main__":
    unittest.main()
