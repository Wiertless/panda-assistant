import os
import unittest

os.environ["GSETTINGS_BACKEND"] = "memory"  # never touch the real desktop settings

from panda_assistant import tools as T  # noqa: E402

try:
    from gi.repository import Gio
    HAVE = Gio.SettingsSchemaSource.get_default().lookup("org.gnome.desktop.interface", True) is not None
except (ImportError, ValueError):
    HAVE = False


def run(name, **args):
    return T.run_tool(T.Context(), name, args)


@unittest.skipUnless(HAVE, "needs PyGObject and the GNOME desktop schemas")
class SettingsTest(unittest.TestCase):
    def test_find(self):
        out = run("find_settings", query="text scaling")
        self.assertIn("org.gnome.desktop.interface text-scaling-factor", out)

    def test_change_and_read(self):
        self.assertIn("1.25", run("change_setting", schema="org.gnome.desktop.interface",
                                  key="text-scaling-factor", value="1.25"))
        self.assertIn("= 1.25", run("get_setting", schema="org.gnome.desktop.interface", key="text-scaling-factor"))

    def test_string_without_quotes(self):
        self.assertIn("'prefer-dark'", run("change_setting", schema="org.gnome.desktop.interface",
                                           key="color-scheme", value="prefer-dark"))

    def test_out_of_range(self):
        with self.assertRaises(T.ToolError):
            run("change_setting", schema="org.gnome.desktop.interface", key="text-scaling-factor", value="10")

    def test_wrong_type(self):
        with self.assertRaises(T.ToolError):
            run("change_setting", schema="org.gnome.desktop.interface", key="text-scaling-factor", value="huge")

    def test_blocked(self):
        for schema, key in (("org.gnome.desktop.lockdown", "disable-lock-screen"),
                            ("org.gnome.shell", "enabled-extensions"),
                            ("org.gnome.system.proxy", "mode"),
                            ("org.freedesktop.Tracker3.Miner.Files", "index-recursive-directories")):
            with self.assertRaises(T.ToolError, msg=f"{schema} {key}"):
                run("change_setting", schema=schema, key=key, value="true")


class PanelTest(unittest.TestCase):
    def test_unknown_panel(self):
        with self.assertRaises(T.ToolError):
            run("open_settings", panel="../../bin/sh")

    def test_bad_power_mode(self):
        with self.assertRaises(T.ToolError):
            run("set_power_mode", mode="turbo")


if __name__ == "__main__":
    unittest.main()
