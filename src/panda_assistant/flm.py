"""FastFlowLM (FLM) client: server lifecycle, model list, streaming chat.

FLM serves an OpenAI-compatible API on 127.0.0.1:52625. Everything here is
plain stdlib so it can be tested without a GUI or an NPU.
"""

from __future__ import annotations

import glob
import json
import os
import re
import shutil
import subprocess
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterator

DEFAULT_URL = os.environ.get("PANDA_FLM_URL", "http://127.0.0.1:52625")


class FlmError(RuntimeError):
    """The FLM server refused a request or could not be reached."""

    def __init__(self, message: str, status: int | None = None):
        super().__init__(message)
        self.status = status


@dataclass
class Model:
    name: str
    installed: bool = False
    labels: list[str] = field(default_factory=list)

    @property
    def can_use_tools(self) -> bool:
        return "tool-calling" in self.labels

    @property
    def single_turn(self) -> bool:
        return "single-turn" in self.labels


@dataclass
class ToolCall:
    id: str
    name: str
    arguments: str = ""

    def parsed_arguments(self) -> dict:
        if not self.arguments.strip():
            return {}
        value = json.loads(self.arguments)
        if not isinstance(value, dict):
            raise ValueError("tool arguments are not a JSON object")
        return value


@dataclass
class StreamEvent:
    kind: str  # "text" | "reasoning" | "done"
    text: str = ""
    tool_calls: list[ToolCall] = field(default_factory=list)
    finish_reason: str | None = None


def parse_sse_lines(lines: Iterator[str]) -> Iterator[StreamEvent]:
    """Turn OpenAI chat-completion SSE lines into events.

    Tool call deltas are merged by index (only the first delta of a call has
    its id and name) and reported once, in the final "done" event.
    """
    calls: dict[int, ToolCall] = {}
    finish_reason = None
    for raw in lines:
        line = raw.strip()
        if not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if data == "[DONE]":
            break
        try:
            chunk = json.loads(data)
        except json.JSONDecodeError:
            continue
        if "error" in chunk and not chunk.get("choices"):
            err = chunk["error"]
            raise FlmError(err.get("message", str(err)) if isinstance(err, dict) else str(err))
        for choice in chunk.get("choices") or []:
            if choice.get("finish_reason"):
                finish_reason = choice["finish_reason"]
            delta = choice.get("delta") or {}
            if delta.get("reasoning_content"):
                yield StreamEvent("reasoning", delta["reasoning_content"])
            if delta.get("content"):
                yield StreamEvent("text", delta["content"])
            for tc in delta.get("tool_calls") or []:
                idx = tc.get("index", 0)
                fn = tc.get("function") or {}
                call = calls.get(idx)
                if call is None:
                    call = calls[idx] = ToolCall(id=tc.get("id") or f"call_{idx}", name=fn.get("name") or "")
                elif fn.get("name"):
                    call.name = fn["name"]
                call.arguments += fn.get("arguments") or ""
    yield StreamEvent("done", tool_calls=[calls[i] for i in sorted(calls)], finish_reason=finish_reason)


_THINK_RE = re.compile(r"<think>.*?(</think>|$)", re.S)


def strip_think(text: str) -> str:
    """Drop inline <think> blocks some models leave in the answer text."""
    return _THINK_RE.sub("", text).lstrip()


def _find_model_list() -> Path | None:
    candidates = []
    flm = shutil.which("flm")
    if flm:
        candidates.append(Path(os.path.realpath(flm)).parent / "model_list.json")
    for pattern in ("/usr/share/flm/model_list.json", "/usr/share/fastflowlm/model_list.json",
                    "/usr/lib*/flm/model_list.json", "/usr/lib*/fastflowlm/model_list.json",
                    "/opt/*/model_list.json", "/usr/share/*/model_list.json"):
        candidates.extend(Path(p) for p in glob.glob(pattern))
    for path in candidates:
        if path.is_file():
            return path
    return None


def model_labels() -> dict[str, list[str]]:
    """Labels ("tool-calling", "single-turn", ...) from FLM's model_list.json."""
    path = _find_model_list()
    if not path:
        return {}
    try:
        config = json.loads(path.read_text())
    except (OSError, ValueError):
        return {}
    labels = {}
    for family, sizes in (config.get("models") or {}).items():
        for size, info in (sizes or {}).items():
            labels[f"{family}:{size}"] = list(info.get("label") or [])
    return labels


_LIST_RE = re.compile(r"^\s*-\s*(\S+)\s*(.*)$")


def parse_flm_list(output: str) -> dict[str, bool]:
    """Parse `flm list` output into {model: installed}."""
    models = {}
    for line in output.splitlines():
        m = _LIST_RE.match(line)
        if m:
            models[m.group(1)] = "✅" in m.group(2)
    return models


class FlmClient:
    def __init__(self, base_url: str = DEFAULT_URL, timeout: float = 600):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self._server: subprocess.Popen | None = None

    # -- server ------------------------------------------------------------
    def is_up(self) -> bool:
        try:
            with urllib.request.urlopen(self.base_url + "/api/version", timeout=2) as r:
                return r.status == 200
        except (OSError, urllib.error.URLError):
            return False

    def ensure_server(self, model: str, wait: float = 120) -> bool:
        """Start `flm serve` if nothing is listening. True if we started it."""
        if self.is_up():
            return False
        flm = shutil.which("flm")
        if not flm:
            raise FlmError("FastFlowLM (flm) is not installed.")
        log_dir = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state")) / "panda-assistant"
        log_dir.mkdir(parents=True, exist_ok=True)
        log = open(log_dir / "flm-serve.log", "ab")
        self._server = subprocess.Popen([flm, "serve", model], stdin=subprocess.DEVNULL,
                                        stdout=log, stderr=log, start_new_session=True)
        log.close()
        deadline = time.monotonic() + wait
        while time.monotonic() < deadline:
            if self._server.poll() is not None:
                raise FlmError(f"flm serve exited ({self._server.returncode}); see {log_dir / 'flm-serve.log'}")
            if self.is_up():
                return True
            time.sleep(0.5)
        raise FlmError("FastFlowLM didn't start within two minutes.")

    def stop_server(self) -> None:
        """Stop the server only if this app started it."""
        if self._server and self._server.poll() is None:
            self._server.terminate()
            try:
                self._server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self._server.kill()
        self._server = None

    # -- models ------------------------------------------------------------
    def list_models(self) -> list[Model]:
        installed: dict[str, bool] = {}
        flm = shutil.which("flm")
        if flm:
            try:
                out = subprocess.run([flm, "list"], capture_output=True, text=True, timeout=20).stdout
                installed = parse_flm_list(out)
            except (OSError, subprocess.TimeoutExpired):
                pass
        labels = model_labels()
        names = list(installed) or sorted(labels)
        models = [Model(n, installed.get(n, False), labels.get(n, [])) for n in names]
        hidden = ("embed", "whisper", "d3")
        return [m for m in models if not m.name.startswith(hidden)]

    # -- chat --------------------------------------------------------------
    def chat_stream(self, model: str, messages: list[dict], tools: list[dict] | None = None) -> Iterator[StreamEvent]:
        body = {"model": model, "messages": messages, "stream": True}
        if tools:
            body["tools"] = tools
        req = urllib.request.Request(self.base_url + "/v1/chat/completions", data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json", "Authorization": "Bearer flm"})
        try:
            resp = urllib.request.urlopen(req, timeout=self.timeout)
        except urllib.error.HTTPError as e:
            detail = e.read().decode(errors="replace")[:500]
            raise FlmError(f"FLM returned {e.code}: {detail}", e.code) from None
        except (OSError, urllib.error.URLError) as e:
            raise FlmError(f"Can't reach FastFlowLM at {self.base_url}: {e}") from None
        with resp:
            yield from parse_sse_lines(line.decode("utf-8", errors="replace") for line in resp)
