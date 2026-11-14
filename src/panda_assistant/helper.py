"""Tools the Rust service (assistantd) borrows from the Python code for now.

They need GNOME libraries that only have GObject-introspection bindings
(AT-SPI, TinySPARQL, Evolution Data Server). Usage:

    python3 -m panda_assistant.helper <tool> < args.json

Prints {"ok": true, "result": "..."} or {"ok": false, "error": "..."}.
"""

from __future__ import annotations

import json
import sys

TOOLS = {
    "read_window": ("screen", "read_window"),
    "search_file_contents": ("personal", "search_file_contents"),
    "read_document": ("personal", "read_document"),
    "get_events": ("personal", "get_events"),
    "find_contact": ("personal", "find_contact"),
}


def main(argv: list[str]) -> int:
    import importlib
    from .tools import Context, ToolError
    if len(argv) != 1 or argv[0] not in TOOLS:
        print(json.dumps({"ok": False, "error": f"unknown helper tool; one of {', '.join(TOOLS)}"}))
        return 2
    try:
        args = json.loads(sys.stdin.read() or "{}")
        if not isinstance(args, dict):
            raise ValueError("arguments must be an object")
        module, name = TOOLS[argv[0]]
        result = getattr(importlib.import_module(f"panda_assistant.{module}"), name)(Context(), args)
        print(json.dumps({"ok": True, "result": result}, ensure_ascii=False))
    except ToolError as e:
        print(json.dumps({"ok": False, "error": str(e)}, ensure_ascii=False))
    except Exception as e:  # noqa: BLE001 - reported to the model as a tool error
        print(json.dumps({"ok": False, "error": f"{type(e).__name__}: {e}"}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
