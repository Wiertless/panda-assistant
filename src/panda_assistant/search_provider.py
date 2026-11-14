"""GNOME Shell search provider: shows "Ask: <query>" and opens the app.

It never talks to the model itself. GNOME queries providers on every
keystroke, so this only echoes the text back; the NPU runs when the user
picks the result. The process is D-Bus activated and exits when idle.
"""

from __future__ import annotations

import os
import subprocess
import sys

from gi.repository import Gio, GLib

from . import APP_ID

BUS_NAME = APP_ID + ".SearchProvider"
OBJECT_PATH = "/" + APP_ID.replace(".", "/") + "/SearchProvider"
IDLE_EXIT_SECONDS = 60
MIN_CHARS = 3

INTERFACE_XML = """
<node>
  <interface name="org.gnome.Shell.SearchProvider2">
    <method name="GetInitialResultSet">
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetSubsearchResultSet">
      <arg type="as" name="previous_results" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="as" name="results" direction="out"/>
    </method>
    <method name="GetResultMetas">
      <arg type="as" name="identifiers" direction="in"/>
      <arg type="aa{sv}" name="metas" direction="out"/>
    </method>
    <method name="ActivateResult">
      <arg type="s" name="identifier" direction="in"/>
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="LaunchSearch">
      <arg type="as" name="terms" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
  </interface>
</node>
"""


def results_for(terms: list[str]) -> list[str]:
    query = " ".join(terms).strip()
    return [query] if len(query) >= MIN_CHARS else []


def meta_for(query: str) -> dict:
    return {
        "id": GLib.Variant("s", query),
        "name": GLib.Variant("s", f"Ask: {query}"),
        "description": GLib.Variant("s", "Ask Panda, running on your NPU"),
        "gicon": GLib.Variant("s", APP_ID),
    }


def launch_app(query: str) -> None:
    """Ask the (D-Bus activatable) app; start it the classic way if that fails."""
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        bus.call_sync(APP_ID, "/" + APP_ID.replace(".", "/"), "org.freedesktop.Application", "ActivateAction",
                      GLib.Variant("(sava{sv})", ("ask", [GLib.Variant("s", query)], {})),
                      None, Gio.DBusCallFlags.NONE, 10000, None)
        return
    except GLib.Error:
        pass
    exe = os.environ.get("PANDA_ASSISTANT_BIN", "/usr/bin/panda-assistant")
    subprocess.Popen([exe, "--ask", query], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True)


class SearchProvider:
    def __init__(self):
        self.loop = GLib.MainLoop()
        self.idle_id = 0
        self.node = Gio.DBusNodeInfo.new_for_xml(INTERFACE_XML)

    def _touch(self) -> None:
        if self.idle_id:
            GLib.source_remove(self.idle_id)
        self.idle_id = GLib.timeout_add_seconds(IDLE_EXIT_SECONDS, self._quit)

    def _quit(self) -> bool:
        self.loop.quit()
        return GLib.SOURCE_REMOVE

    def _on_call(self, conn, sender, path, iface, method, params, invocation):
        self._touch()
        args = params.unpack()
        if method == "GetInitialResultSet":
            invocation.return_value(GLib.Variant("(as)", (results_for(args[0]),)))
        elif method == "GetSubsearchResultSet":
            invocation.return_value(GLib.Variant("(as)", (results_for(args[1]),)))
        elif method == "GetResultMetas":
            invocation.return_value(GLib.Variant("(aa{sv})", ([meta_for(i) for i in args[0]],)))
        elif method == "ActivateResult":
            launch_app(args[0])
            invocation.return_value(None)
        elif method == "LaunchSearch":
            launch_app(" ".join(args[0]))
            invocation.return_value(None)
        else:
            invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)

    def _on_bus(self, conn, name):
        conn.register_object(OBJECT_PATH, self.node.interfaces[0], self._on_call, None, None)

    def run(self) -> int:
        Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.NONE,
                         self._on_bus, None, lambda *a: self.loop.quit())
        self._touch()
        self.loop.run()
        return 0


def main() -> int:
    return SearchProvider().run()


if __name__ == "__main__":
    sys.exit(main())
