"""What Panda keeps between chats: memory, skills and chat history.

Everything is plain files under ~/.local/share/panda-assistant, so it can be
read, edited, backed up or deleted by hand:

    memory.md         one remembered fact per "- " line
    skills/<name>.md  saved workflows: "# Title", a description line, steps
    chats/<id>.json   saved conversations
"""

from __future__ import annotations

import json
import os
import re
import time
from dataclasses import dataclass
from pathlib import Path

MAX_FACTS = 50
MAX_FACT_LEN = 200
MAX_SKILL_LEN = 4000
MAX_CHATS = 200


def data_dir() -> Path:
    base = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
    return Path(base) / "panda-assistant"


def _write(path: Path, text: str) -> None:
    """Write atomically, so a crash never leaves half a file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.tmp")
    tmp.write_text(text)
    os.replace(tmp, path)


def _norm(text: str) -> str:
    return re.sub(r"\s+", " ", text.casefold()).strip(" .!")


# -- memory -------------------------------------------------------------------

def memory_file() -> Path:
    return data_dir() / "memory.md"


def facts() -> list[str]:
    try:
        lines = memory_file().read_text().splitlines()
    except OSError:
        return []
    return [line[2:].strip() for line in lines if line.startswith("- ") and line[2:].strip()]


def _save_facts(items: list[str]) -> None:
    _write(memory_file(), "# Things the assistant remembers\n\n" + "".join(f"- {f}\n" for f in items))


def remember(fact: str) -> str:
    fact = re.sub(r"\s+", " ", fact).strip()
    if not fact:
        raise ValueError("there's nothing to remember")
    if len(fact) > MAX_FACT_LEN:
        raise ValueError(f"keep it under {MAX_FACT_LEN} characters")
    items = facts()
    if any(_norm(f) == _norm(fact) for f in items):
        return "Already remembered."
    items.append(fact)
    _save_facts(items[-MAX_FACTS:])  # oldest go first
    return f"Remembered: {fact}"


def forget(text: str) -> list[str]:
    """Remove the facts that match text; returns what was removed."""
    key = _norm(text)
    if not key:
        return []
    items = facts()
    gone = [f for f in items if _norm(f) == key] or [f for f in items if key in _norm(f)]
    if gone:
        _save_facts([f for f in items if f not in gone])
    return gone


# -- skills -------------------------------------------------------------------

@dataclass
class Skill:
    slug: str
    title: str
    description: str
    steps: str
    path: Path


def skills_dir() -> Path:
    return data_dir() / "skills"


def slugify(name: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", name.casefold()).strip("-")
    return slug[:40]


def _parse_skill(path: Path) -> Skill | None:
    try:
        text = path.read_text()
    except OSError:
        return None
    lines = text.strip().splitlines()
    title = path.stem.replace("-", " ").capitalize()
    if lines and lines[0].startswith("# "):
        title = lines.pop(0)[2:].strip() or title
    while lines and not lines[0].strip():
        lines.pop(0)
    description = ""
    if lines and not re.match(r"^\s*(\d+[.)]|[-*])\s", lines[0]):
        description = lines.pop(0).strip()
    steps = "\n".join(lines).strip()
    if not steps and description:
        steps, description = description, ""
    return Skill(path.stem, title, description, steps, path)


def skills() -> list[Skill]:
    found = (_parse_skill(p) for p in sorted(skills_dir().glob("*.md")))
    return [s for s in found if s and s.steps]


def find_skill(name: str) -> Skill | None:
    slug = slugify(name.lstrip("/"))
    if not slug:
        return None
    all_skills = skills()
    for s in all_skills:
        if s.slug == slug or slugify(s.title) == slug:
            return s
    hits = [s for s in all_skills if slug in s.slug or slug in slugify(s.title)]
    return hits[0] if len(hits) == 1 else None


def save_skill(title: str, steps: str, description: str = "") -> Skill:
    title = re.sub(r"\s+", " ", title).strip()
    slug = slugify(title)
    if not slug:
        raise ValueError("the skill needs a name")
    steps = steps.strip()
    if not steps:
        raise ValueError("the skill needs steps")
    if len(steps) > MAX_SKILL_LEN:
        raise ValueError(f"keep the steps under {MAX_SKILL_LEN} characters")
    description = re.sub(r"\s+", " ", description).strip()
    path = skills_dir() / f"{slug}.md"
    _write(path, f"# {title}\n\n" + (f"{description}\n\n" if description else "") + steps + "\n")
    return Skill(slug, title, description, steps, path)


def delete_skill(slug: str) -> bool:
    path = skills_dir() / f"{slugify(slug)}.md"
    try:
        path.unlink()
        return True
    except OSError:
        return False


def skill_prompt(skill: Skill, details: str = "") -> str:
    """The message that runs a skill."""
    text = f"Follow my saved skill “{skill.title}”:\n{skill.steps}"
    return text + (f"\n\nDetails: {details.strip()}" if details.strip() else "")


# -- chat history ---------------------------------------------------------------

@dataclass
class ChatInfo:
    id: str
    title: str
    updated: float
    path: Path


def chats_dir() -> Path:
    return data_dir() / "chats"


def new_chat_id() -> str:
    return time.strftime("%Y%m%d-%H%M%S") + f"-{time.time_ns() % 1000000:06d}"


def plain_text(content) -> str:
    """Message content as text; images are kept as a marker only."""
    if isinstance(content, list):
        parts = []
        for part in content:
            if part.get("type") == "text":
                parts.append(part.get("text", ""))
            elif part.get("type") == "image_url":
                parts.append("[image]")
        return "\n".join(p for p in parts if p)
    return content or ""


def chat_title(messages: list[dict]) -> str:
    first = next((m for m in messages if m.get("role") == "user"), None)
    text = re.sub(r"\s+", " ", plain_text(first["content"]) if first else "").strip()
    if text.startswith("Follow my saved skill “"):
        text = text.split("”", 1)[0].replace("Follow my saved skill “", "Skill: ")
    return (text[:57] + "…") if len(text) > 60 else (text or "New Chat")


def save_chat(chat_id: str, messages: list[dict], model: str) -> None:
    if not any(m.get("role") == "user" for m in messages):
        return
    stored = []
    for m in messages:
        m = dict(m)
        if isinstance(m.get("content"), list):
            m["content"] = plain_text(m["content"])  # don't keep screenshots on disk
        stored.append(m)
    data = {"id": chat_id, "title": chat_title(messages), "model": model,
            "updated": time.time(), "messages": stored}
    _write(chats_dir() / f"{chat_id}.json", json.dumps(data, ensure_ascii=False))
    _prune()


def _prune() -> None:
    old = list_chats()[MAX_CHATS:]
    for chat in old:
        chat.path.unlink(missing_ok=True)


def load_chat(chat_id: str) -> dict | None:
    if not re.fullmatch(r"[\w-]+", chat_id):
        return None
    try:
        data = json.loads((chats_dir() / f"{chat_id}.json").read_text())
    except (OSError, ValueError):
        return None
    return data if isinstance(data.get("messages"), list) else None


def list_chats(query: str = "") -> list[ChatInfo]:
    key = _norm(query)
    out = []
    for path in chats_dir().glob("*.json"):
        try:
            data = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if key:
            text = " ".join(plain_text(m.get("content")) for m in data.get("messages", [])
                            if m.get("role") in ("user", "assistant"))
            if key not in _norm(data.get("title", "") + " " + text):
                continue
        out.append(ChatInfo(path.stem, data.get("title") or "Chat", float(data.get("updated", 0)), path))
    return sorted(out, key=lambda c: c.updated, reverse=True)


def delete_chat(chat_id: str) -> None:
    if re.fullmatch(r"[\w-]+", chat_id):
        (chats_dir() / f"{chat_id}.json").unlink(missing_ok=True)
