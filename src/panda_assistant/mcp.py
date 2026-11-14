"""A small Model Context Protocol (MCP) client.

Other apps and services expose tools over MCP; Panda can use them like its
own. Servers are listed in ~/.config/panda-assistant/mcp.json in the common
format (the same one Claude Desktop and Zed use):

    {"mcpServers": {
        "notes": {"command": "notes-mcp", "args": ["--stdio"], "env": {}},
        "home":  {"url": "http://homeassistant.local:8123/mcp_server/sse",
                  "headers": {"Authorization": "Bearer ..."}}
    }}

Extra keys Panda understands per server: "disabled": true, and
"trust": true to run that server's tools without asking first.

Only the parts Panda needs are implemented: initialize, tools/list and
tools/call, over stdio or Streamable HTTP. No third-party packages.
"""

from __future__ import annotations

import json
import os
import queue
import re
import shlex
import subprocess
import threading
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

from . import VERSION

PROTOCOL_VERSION = "2025-06-18"
START_TIMEOUT = 30
CALL_TIMEOUT = 120
MAX_TOOLS = 16          # across all servers: each schema costs context tokens
MAX_DESCRIPTION = 240
MAX_RESULT = 4000
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class McpError(Exception):
    pass


def config_path() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / "panda-assistant" / "mcp.json"


def _log_dir() -> Path:
    base = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(base) / "panda-assistant"


@dataclass
class ServerConfig:
    name: str
    command: str = ""
    args: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)
    url: str = ""
    headers: dict[str, str] = field(default_factory=dict)
    disabled: bool = False
    trust: bool = False

    def to_json(self) -> dict:
        out: dict = {}
        if self.url:
            out["url"] = self.url
            if self.headers:
                out["headers"] = self.headers
        else:
            out["command"] = self.command
            if self.args:
                out["args"] = self.args
            if self.env:
                out["env"] = self.env
        if self.disabled:
            out["disabled"] = True
        if self.trust:
            out["trust"] = True
        return out


def _read_raw() -> dict:
    try:
        data = json.loads(config_path().read_text())
    except FileNotFoundError:
        return {}
    except (OSError, ValueError) as e:
        raise McpError(f"{config_path()} isn't valid JSON: {e}") from None
    return data if isinstance(data, dict) else {}


def load_config() -> list[ServerConfig]:
    servers = _read_raw().get("mcpServers") or {}
    out = []
    for name, spec in servers.items():
        if not isinstance(spec, dict):
            continue
        args = spec.get("args") or []
        out.append(ServerConfig(
            name=str(name), command=str(spec.get("command") or ""),
            args=[str(a) for a in args] if isinstance(args, list) else [],
            env={str(k): str(v) for k, v in (spec.get("env") or {}).items()},
            url=str(spec.get("url") or spec.get("serverUrl") or ""),
            headers={str(k): str(v) for k, v in (spec.get("headers") or {}).items()},
            disabled=bool(spec.get("disabled")), trust=bool(spec.get("trust"))))
    return out


def save_config(servers: list[ServerConfig]) -> None:
    raw = _read_raw()
    raw["mcpServers"] = {s.name: s.to_json() for s in servers}
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(".mcp.json.tmp")
    tmp.write_text(json.dumps(raw, indent=2) + "\n")
    os.chmod(tmp, 0o600)  # headers may hold tokens
    os.replace(tmp, path)


def parse_server(name: str, command_or_url: str) -> ServerConfig:
    """Build a config from what the user typed: a command line or a URL."""
    name = name.strip()
    text = command_or_url.strip()
    if not name or not text:
        raise McpError("give the server a name and a command or URL")
    if re.match(r"^https?://", text):
        return ServerConfig(name=name, url=text)
    try:
        parts = shlex.split(text)
    except ValueError as e:
        raise McpError(f"couldn't read the command: {e}") from None
    return ServerConfig(name=name, command=parts[0], args=parts[1:])


# -- transports -----------------------------------------------------------------

class StdioTransport:
    """JSON-RPC messages, one per line, over a child process's stdin/stdout."""

    def __init__(self, cfg: ServerConfig):
        log_dir = _log_dir()
        log_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = log_dir / f"mcp-{re.sub(r'[^\w.-]', '_', cfg.name)}.log"
        self._log = open(self.log_path, "ab")
        env = dict(os.environ, **cfg.env)
        try:
            self.proc = subprocess.Popen([cfg.command, *cfg.args], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=self._log, env=env, cwd=str(Path.home()), start_new_session=True)
        except OSError as e:
            self._log.close()
            raise McpError(f"couldn't start {cfg.command}: {e.strerror or e}") from None
        self.incoming: queue.Queue = queue.Queue()
        self._lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        try:
            for line in self.proc.stdout:
                line = line.strip()
                if not line:
                    continue
                try:
                    self.incoming.put(json.loads(line))
                except ValueError:
                    continue  # some servers print banners on stdout
        except (OSError, ValueError):
            pass  # closed under us by close()
        self.incoming.put(None)  # closed

    def send(self, msg: dict) -> None:
        with self._lock:
            try:
                self.proc.stdin.write((json.dumps(msg) + "\n").encode())
                self.proc.stdin.flush()
            except (OSError, ValueError):
                raise McpError("the server stopped") from None

    def request(self, msg: dict, timeout: float) -> dict:
        self.send(msg)
        while True:
            try:
                reply = self.incoming.get(timeout=timeout)
            except queue.Empty:
                raise McpError(f"no answer within {int(timeout)} seconds") from None
            if reply is None:
                raise McpError(f"the server stopped (see {self.log_path})")
            if reply.get("id") == msg["id"] and ("result" in reply or "error" in reply):
                return reply
            self._answer_server_request(reply)

    def _answer_server_request(self, msg: dict) -> None:
        """Servers may ask things too; Panda only answers ping."""
        if "method" in msg and "id" in msg:
            if msg["method"] == "ping":
                self.send({"jsonrpc": "2.0", "id": msg["id"], "result": {}})
            else:
                self.send({"jsonrpc": "2.0", "id": msg["id"],
                           "error": {"code": -32601, "message": "Method not found"}})

    def close(self) -> None:
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(3)
        except subprocess.TimeoutExpired:
            self.proc.terminate()
            try:
                self.proc.wait(3)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.proc.stdout.close()
        self._log.close()


class HttpTransport:
    """Streamable HTTP: POST each message; the answer is JSON or an SSE stream."""

    def __init__(self, cfg: ServerConfig):
        self.url = cfg.url
        self.headers = dict(cfg.headers)
        self.session: str | None = None

    def _post(self, msg: dict, timeout: float):
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "MCP-Protocol-Version": PROTOCOL_VERSION, **self.headers}
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        req = urllib.request.Request(self.url, data=json.dumps(msg).encode(), headers=headers, method="POST")
        try:
            return _OPENER.open(req, timeout=timeout)
        except urllib.error.HTTPError as e:
            raise McpError(f"HTTP {e.code} from {self.url}") from None
        except (urllib.error.URLError, OSError) as e:
            raise McpError(f"couldn't reach {self.url}: {getattr(e, 'reason', e)}") from None

    def send(self, msg: dict) -> None:
        self._post(msg, 15).close()

    def request(self, msg: dict, timeout: float) -> dict:
        with self._post(msg, timeout) as resp:
            self.session = resp.headers.get("Mcp-Session-Id") or self.session
            if "text/event-stream" in (resp.headers.get("Content-Type") or ""):
                data = []
                for raw in resp:
                    line = raw.decode("utf-8", "replace").rstrip("\r\n")
                    if line.startswith("data:"):
                        data.append(line[5:].lstrip())
                    elif not line and data:
                        try:
                            reply = json.loads("\n".join(data))
                        except ValueError:
                            reply = {}
                        data = []
                        if reply.get("id") == msg["id"]:
                            return reply
                raise McpError("the server closed the stream without answering")
            try:
                return json.loads(resp.read(8 * 2**20))
            except ValueError:
                raise McpError("the server's answer wasn't JSON") from None

    def close(self) -> None:
        if self.session:
            headers = {"Mcp-Session-Id": self.session, **self.headers}
            try:
                _OPENER.open(urllib.request.Request(self.url, headers=headers, method="DELETE"), timeout=5).close()
            except Exception:  # noqa: BLE001 - best effort
                pass


# -- one server -------------------------------------------------------------------

@dataclass
class McpTool:
    server: str
    name: str            # the server's own name for it
    public_name: str     # what the model sees: server__tool
    title: str
    description: str
    schema: dict
    read_only: bool


def _public_name(server: str, tool: str) -> str:
    clean = lambda s: re.sub(r"[^A-Za-z0-9_-]+", "_", s).strip("_") or "x"  # noqa: E731
    return f"{clean(server)[:20]}__{clean(tool)}"[:64]


class Server:
    def __init__(self, cfg: ServerConfig):
        self.cfg = cfg
        self.transport = None
        self.tools: list[McpTool] = []
        self.error: str | None = None
        self.label = cfg.name
        self._next_id = 0
        self._lock = threading.Lock()

    @property
    def running(self) -> bool:
        return self.transport is not None

    def _rpc(self, method: str, params: dict | None = None, timeout: float = START_TIMEOUT) -> dict:
        with self._lock:
            self._next_id += 1
            msg = {"jsonrpc": "2.0", "id": self._next_id, "method": method}
            if params is not None:
                msg["params"] = params
            reply = self.transport.request(msg, timeout)
        if "error" in reply:
            err = reply["error"] or {}
            raise McpError(str(err.get("message") or err))
        return reply.get("result") or {}

    def start(self) -> None:
        if self.running:
            return
        self.error = None
        cfg = self.cfg
        if not (cfg.url or cfg.command):
            raise McpError("no command or URL")
        self.transport = HttpTransport(cfg) if cfg.url else StdioTransport(cfg)
        try:
            info = self._rpc("initialize", {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                                            "clientInfo": {"name": "panda-assistant", "title": "Panda",
                                                           "version": VERSION}})
            self.label = (info.get("serverInfo") or {}).get("title") or cfg.name
            self.transport.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            self.tools = self._list_tools()
        except Exception:
            self.stop()
            raise

    def _list_tools(self) -> list[McpTool]:
        tools, cursor = [], None
        for _ in range(10):  # pages
            result = self._rpc("tools/list", {"cursor": cursor} if cursor else {})
            for t in result.get("tools") or []:
                if not isinstance(t, dict) or not t.get("name"):
                    continue
                schema = dict(t.get("inputSchema") or {"type": "object", "properties": {}})
                schema.pop("$schema", None)
                schema.setdefault("type", "object")
                schema.setdefault("properties", {})
                notes = t.get("annotations") or {}
                desc = " ".join(str(t.get("description") or t.get("title") or t["name"]).split())
                tools.append(McpTool(self.cfg.name, t["name"], _public_name(self.cfg.name, t["name"]),
                                     str(t.get("title") or notes.get("title") or t["name"]),
                                     desc[:MAX_DESCRIPTION], schema, bool(notes.get("readOnlyHint"))))
            cursor = result.get("nextCursor")
            if not cursor:
                break
        return tools

    def call(self, tool: str, args: dict) -> tuple[str, list[str]]:
        """Run a tool; returns its text and any images (as data: URLs)."""
        if not self.running:
            self.start()
        result = self._rpc("tools/call", {"name": tool, "arguments": args}, timeout=CALL_TIMEOUT)
        texts, images = [], []
        for part in result.get("content") or []:
            kind = part.get("type")
            if kind == "text":
                texts.append(str(part.get("text", "")))
            elif kind == "image" and part.get("data"):
                images.append(f"data:{part.get('mimeType', 'image/png')};base64,{part['data']}")
            elif kind == "resource":
                res = part.get("resource") or {}
                texts.append(str(res.get("text") or res.get("uri") or ""))
            elif kind == "resource_link":
                texts.append(f"{part.get('name', '')} {part.get('uri', '')}".strip())
        if not texts and result.get("structuredContent") is not None:
            texts.append(json.dumps(result["structuredContent"], ensure_ascii=False))
        text = "\n".join(t for t in texts if t).strip() or "Done."
        if len(text) > MAX_RESULT:
            text = text[:MAX_RESULT] + "…"
        if result.get("isError"):
            raise McpError(text)
        return text, images

    def stop(self) -> None:
        transport, self.transport = self.transport, None
        if transport:
            transport.close()


# -- all servers ------------------------------------------------------------------

class Hub:
    """The configured servers, started on first use."""

    def __init__(self):
        self.servers: dict[str, Server] = {}
        self.config_error: str | None = None
        self._lock = threading.Lock()
        self.reload()

    def reload(self) -> None:
        try:
            configs = load_config()
            self.config_error = None
        except McpError as e:
            configs, self.config_error = [], str(e)
        with self._lock:
            old = self.servers
            self.servers = {}
            for cfg in configs:
                keep = old.pop(cfg.name, None)
                if keep and keep.cfg == cfg:
                    keep.error = None  # try a failed server again
                    self.servers[cfg.name] = keep
                else:
                    if keep:
                        keep.stop()
                    self.servers[cfg.name] = Server(cfg)
            for gone in old.values():
                gone.stop()

    def enabled(self) -> list[Server]:
        return [s for s in self.servers.values() if not s.cfg.disabled]

    def start_all(self) -> list[str]:
        """Start enabled servers that aren't running, in parallel; returns problems."""
        todo = [s for s in self.enabled() if not s.running and s.error is None]

        def run(server: Server):
            try:
                server.start()
            except Exception as e:  # noqa: BLE001 - shown to the user
                server.error = str(e) or type(e).__name__

        threads = [threading.Thread(target=run, args=(s,), daemon=True) for s in todo]
        for t in threads:
            t.start()
        for t in threads:
            t.join(START_TIMEOUT + 5)
        return [f"{s.cfg.name}: {s.error}" for s in todo if s.error]

    def tools(self) -> list[McpTool]:
        out = []
        for server in self.enabled():
            if server.running:
                out.extend(server.tools)
        return out[:MAX_TOOLS]

    def stop_all(self) -> None:
        for server in list(self.servers.values()):
            server.stop()
            server.error = None


def as_tools(hub: Hub) -> list:
    """The running servers' tools as Panda tools.

    Tools that say they only read run straight away; anything else asks
    first, unless the server is marked "trust": true.
    """
    from . import tools as T
    out = []
    for t in hub.tools():
        server = hub.servers.get(t.server)
        if server is None:
            continue

        def run(ctx, args, server=server, t=t):
            try:
                text, images = server.call(t.name, args)
            except McpError as e:
                raise T.ToolError(str(e)) from None
            ctx.images.extend(images)
            return text

        out.append(T.Tool(
            t.public_name, f"[{server.label}] {t.description}", t.schema,
            T.SAFE if (t.read_only or server.cfg.trust) else T.CONFIRM, run,
            lambda a, t=t, server=server: f"{server.label}: {t.title}",
            detail=lambda a: json.dumps(a, ensure_ascii=False, indent=1)[:600] if a else ""))
    return out
