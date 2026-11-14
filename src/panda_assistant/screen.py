"""Read what's in a window through the accessibility tree (AT-SPI).

This gives the assistant the text of the page, document or message in the
focused window, and anything selected, which suits small models far better
than a screenshot. It runs as its own short-lived process
(`python3 -m panda_assistant.screen`) because AT-SPI calls are blocking
D-Bus round trips into other apps: a slow app can't hang the assistant, and
a time limit always holds.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time

MAX_NODES = 3000
MAX_CHARS = 4000
TIME_BUDGET = 4.0
SKIP_APPS = {"gnome-shell", "mutter-x11-frames", "xdg-desktop-portal-gnome", "ibus-extension-gtk3",
             "ibus-x11", "evolution-alarm-notify", "gsd-xsettings"}
OBJECT_CHAR = "￼"  # stands for an embedded child in AT-SPI text


def collect_text(root, child_count, child_at, text_of, name_of, role_of, showing, selection_of,
                 max_nodes=MAX_NODES, max_chars=MAX_CHARS, budget=TIME_BUDGET):
    """Walk an accessibility tree depth-first, gathering visible text and selections.

    The accessors are passed in so the walk can be tested without a desktop.
    """
    deadline = time.monotonic() + budget
    parts, selected, seen, total = [], [], 0, 0
    stack = [root]
    while stack and seen < max_nodes and total < max_chars and time.monotonic() < deadline:
        node = stack.pop()
        seen += 1
        try:
            if not showing(node):
                continue
            text = text_of(node)
            if text is None and role_of(node) in ("label", "heading", "list item", "table cell", "link"):
                text = name_of(node)
            if text:
                text = " ".join(text.replace(OBJECT_CHAR, " ").split())
                if text and (not parts or parts[-1] != text):
                    parts.append(text)
                    total += len(text)
            sel = selection_of(node)
            if sel:
                selected.append(sel)
            n = child_count(node)
        except Exception:  # noqa: BLE001 - apps vanish and objects go stale mid-walk
            continue
        for i in reversed(range(min(n, 500))):
            try:
                stack.append(child_at(node, i))
            except Exception:  # noqa: BLE001
                continue
    body = "\n".join(parts)
    return body[:max_chars], "\n".join(selected)[:max_chars]


def _atspi():
    import gi
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi
    Atspi.init()
    return Atspi


def _windows(Atspi, skip_pid: int):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(i)
        if app is None:
            continue
        try:
            if app.get_process_id() in (skip_pid, os.getpid()) or (app.get_name() or "") in SKIP_APPS:
                continue
            for j in range(app.get_child_count()):
                win = app.get_child_at_index(j)
                if win is not None:
                    yield app, win
        except Exception:  # noqa: BLE001
            continue


def read(app_name: str = "", skip_pid: int = 0) -> dict:
    Atspi = _atspi()
    S = Atspi.StateType
    wins = []
    for app, win in _windows(Atspi, skip_pid):
        try:
            states = win.get_state_set()
            wins.append({"app": app.get_name() or "", "title": win.get_name() or "", "obj": win,
                         "active": states.contains(S.ACTIVE), "showing": states.contains(S.SHOWING)})
        except Exception:  # noqa: BLE001
            continue
    if app_name:
        want = app_name.casefold()
        chosen = [w for w in wins if want in w["app"].casefold() or want in w["title"].casefold()]
        chosen.sort(key=lambda w: (not w["active"], not w["showing"]))
    else:
        chosen = [w for w in wins if w["active"]]
    if not chosen:
        return {"windows": [f"{w['app']}: {w['title']}" for w in wins if w["showing"] and w["title"]][:15]}
    win = chosen[0]

    def text_of(node):
        iface = node.get_text_iface()
        if iface is None:
            return None
        n = Atspi.Text.get_character_count(node)
        return Atspi.Text.get_text(node, 0, min(n, 20000)) if n else ""

    def selection_of(node):
        if node.get_text_iface() is None or Atspi.Text.get_n_selections(node) < 1:
            return ""
        r = Atspi.Text.get_selection(node, 0)
        return Atspi.Text.get_text(node, r.start_offset, r.end_offset) if r.end_offset > r.start_offset else ""

    text, selected = collect_text(
        win["obj"], lambda n: n.get_child_count(), lambda n, i: n.get_child_at_index(i), text_of,
        lambda n: n.get_name(), lambda n: n.get_role_name(),
        lambda n: n.get_state_set().contains(S.SHOWING), selection_of)
    return {"app": win["app"], "title": win["title"], "text": text, "selected": selected}


def format_result(result: dict) -> str:
    if "windows" in result:
        names = "\n".join(result["windows"]) or "none"
        return ("No window is focused. Ask which one, or call again with app set to one of these:\n" + names)
    head = f"Window: {result['title']} ({result['app']})"
    if result.get("selected", "").strip():
        head += f"\nSelected text:\n{result['selected'].strip()}"
    body = result.get("text", "").strip()
    if not body:
        return head + "\n(This window shares no text. Try look_at_screen instead.)"
    return head + "\nWindow text:\n" + body


def read_window(ctx, args: dict) -> str:
    """Tool: text of the focused window (or the one named), via a helper process."""
    from .tools import ToolError
    cmd = [sys.executable, "-m", "panda_assistant.screen", "--skip-pid", str(os.getpid())]
    if str(args.get("app", "")).strip():
        cmd += ["--app", str(args["app"]).strip()]
    env = dict(os.environ, PYTHONPATH=os.pathsep.join(p for p in sys.path if p))
    # If the assistant's own window has focus, step aside so the window the
    # user means (the one behind) becomes the focused one.
    if ctx.hide_window:
        ctx.hide_window()
        time.sleep(0.4)
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=TIME_BUDGET + 6, env=env,
                              stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        raise ToolError("the window took too long to answer") from None
    finally:
        if ctx.show_window:
            ctx.show_window()
    try:
        result = json.loads(proc.stdout)
    except ValueError:
        raise ToolError("couldn't read the window (accessibility isn't available)") from None
    if "error" in result:
        raise ToolError(result["error"])
    return format_result(result)


def main(argv: list[str]) -> int:
    app, skip = "", 0
    it = iter(argv)
    for arg in it:
        if arg == "--app":
            app = next(it, "")
        elif arg == "--skip-pid":
            skip = int(next(it, "0") or 0)
    try:
        result = read(app, skip)
    except Exception as e:  # noqa: BLE001 - reported to the model as a tool error
        result = {"error": f"couldn't read the window: {e}"}
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
