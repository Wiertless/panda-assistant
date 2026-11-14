"""A tiny MCP server over stdio for tests: two tools, one read-only."""
import json
import sys

TOOLS = [
    {"name": "get_note", "title": "Get Note", "description": "Read a note by title.",
     "inputSchema": {"$schema": "http://json-schema.org/draft-07/schema#", "type": "object",
                     "properties": {"title": {"type": "string"}}, "required": ["title"]},
     "annotations": {"readOnlyHint": True}},
    {"name": "add.note", "description": "Add a note.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}},
]
print("banner line that isn't JSON", flush=True)
for line in sys.stdin:
    msg = json.loads(line)
    method, mid = msg.get("method"), msg.get("id")
    if mid is None:
        continue  # notification
    if method == "initialize":
        # Ask the client something first, like real servers sometimes do.
        print(json.dumps({"jsonrpc": "2.0", "id": "s1", "method": "ping"}), flush=True)
        result = {"protocolVersion": msg["params"]["protocolVersion"], "capabilities": {"tools": {}},
                  "serverInfo": {"name": "notes", "title": "Notes", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": TOOLS}
    elif method == "tools/call":
        name, args = msg["params"]["name"], msg["params"].get("arguments", {})
        if name == "get_note" and args.get("title") == "missing":
            result = {"content": [{"type": "text", "text": "No such note"}], "isError": True}
        elif name == "get_note":
            result = {"content": [{"type": "text", "text": f"Note {args.get('title')}: buy milk"},
                                  {"type": "image", "data": "aGk=", "mimeType": "image/png"}]}
        else:
            result = {"content": [{"type": "text", "text": "Added."}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "nope"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}), flush=True)
