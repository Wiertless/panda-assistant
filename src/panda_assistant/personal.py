"""Personal context from GNOME's own stores, read on demand and never copied.

- Files by what's in them: GNOME's LocalSearch index (what Files and the
  overview search use), including text it extracted from PDFs and documents.
- Calendar events and contacts: Evolution Data Server, which holds the
  accounts from GNOME Settings → Online Accounts and GNOME Calendar/Contacts.
"""

from __future__ import annotations

import datetime as dt
import importlib
import re
from pathlib import Path
from urllib.parse import unquote, urlparse

from .tools import Context, ToolError, _hidden, _home_path, _short

MAX_HITS = 10
FTS_SKIP = {"and", "or", "not", "near", "the", "a", "an", "of", "in", "file", "files", "about"}
MAX_EVENTS = 30
MAX_TEXT = 3000

# Fedora 45 (GNOME 47+) names the indexer LocalSearch and its library
# TinySPARQL; older systems use the Tracker names.
SPARQL_NAMESPACES = ("Tsparql", "Tracker")
INDEXER_BUSES = ("org.freedesktop.LocalSearch3", "org.freedesktop.Tracker3.Miner.Files")

CONTENT_QUERY = """
SELECT DISTINCT ?url ?title fts:snippet(?ie, "", "", "…", 14) WHERE {
  ?ie fts:match ~q ;
      nie:isStoredAs ?file .
  ?file nie:url ?url .
  OPTIONAL { ?ie nie:title ?title }
} ORDER BY DESC(fts:rank(?ie)) LIMIT 40
"""

TEXT_QUERY = """
SELECT ?text WHERE {
  ?ie nie:isStoredAs ?file ;
      nie:plainTextContent ?text .
  ?file nie:url ~url .
} LIMIT 1
"""


# -- LocalSearch ------------------------------------------------------------------

def _sparql_module():
    import gi
    for ns in SPARQL_NAMESPACES:
        try:
            gi.require_version(ns, "3.0")
            return importlib.import_module(f"gi.repository.{ns}")
        except (ValueError, ImportError):
            continue
    raise ToolError("GNOME's file index (LocalSearch) isn't installed")


def _connection():
    from gi.repository import GLib
    mod = _sparql_module()
    for bus in INDEXER_BUSES:
        try:
            conn = mod.SparqlConnection.bus_new(bus, None, None)
            conn.query("ASK {}", None).next(None)  # is it really there?
            return conn
        except GLib.Error:
            continue
    raise ToolError("GNOME's file index (LocalSearch) isn't running")


def _query(sparql: str, **binds) -> list[list[str]]:
    from gi.repository import GLib
    conn = _connection()
    try:
        stmt = conn.query_statement(sparql, None)
        for key, value in binds.items():
            stmt.bind_string(key, value)
        cursor = stmt.execute(None)
        rows = []
        while cursor.next(None):
            rows.append([cursor.get_string(i)[0] or "" for i in range(cursor.get_n_columns())])
        cursor.close()
        return rows
    except GLib.Error as e:
        raise ToolError(f"the file index couldn't search: {e.message}") from None


def fts_terms(text: str) -> str:
    """User words -> an FTS5 match: every word, as a prefix; no operators."""
    words = [w for w in re.findall(r"\w+", text.casefold()) if len(w) > 1 and w not in FTS_SKIP]
    if not words:
        raise ToolError("say what to look for")
    return " ".join(f"{w}*" for w in words[:8])


def url_to_path(url: str) -> Path | None:
    parsed = urlparse(url)
    if parsed.scheme != "file":
        return None
    return Path(unquote(parsed.path))


def search_file_contents(ctx: Context, args: dict) -> str:
    query = str(args.get("query", ""))
    home = Path.home()
    lines = []
    for url, title, snippet in _query(CONTENT_QUERY, q=fts_terms(query)):
        path = url_to_path(url)
        if path is None or home not in path.parents or _hidden(path, home):
            continue  # stay inside the home folder, away from hidden files
        snippet = re.sub(r"\s+", " ", snippet).strip()
        line = _short(path) + (f" — {title}" if title and title != path.name else "")
        lines.append(line + (f"\n  “{snippet[:200]}”" if snippet else ""))
        if len(lines) >= MAX_HITS:
            break
    if not lines:
        return f"No files mention “{query}” (only indexed folders are searched)."
    return "\n".join(lines)


def indexed_text(path: Path) -> str:
    """Text LocalSearch extracted from a document (PDF, office files…), or ''."""
    try:
        rows = _query(TEXT_QUERY, url=path.as_uri())
    except ToolError:
        return ""
    return rows[0][0] if rows else ""


def read_document(ctx: Context, args: dict) -> str:
    path = _home_path(str(args.get("path", "")))
    if not path.is_file():
        raise ToolError(f"{_short(path)} isn't a file")
    text = indexed_text(path)
    if not text and path.suffix.lower() == ".pdf":
        import shutil
        import subprocess
        if shutil.which("pdftotext"):
            proc = subprocess.run(["pdftotext", "-l", "20", str(path), "-"], capture_output=True, text=True,
                                  timeout=20)
            text = proc.stdout
    if not text.strip():
        raise ToolError(f"no text is available for {_short(path)} (it may not be indexed yet)")
    text = re.sub(r"\n{3,}", "\n\n", text).strip()
    return f"{_short(path)}:\n" + (text[:MAX_TEXT] + "\n[…]" if len(text) > MAX_TEXT else text)


# -- calendar and contacts (Evolution Data Server) ----------------------------------

def _eds(*names: tuple[str, str]):
    import gi
    mods = []
    for ns, ver in names:
        try:
            gi.require_version(ns, ver)
            mods.append(importlib.import_module(f"gi.repository.{ns}"))
        except (ValueError, ImportError):
            raise ToolError("calendar and contacts need Evolution Data Server, which isn't installed") from None
    return mods


def _registry():
    from gi.repository import GLib
    (EDataServer,) = _eds(("EDataServer", "1.2"))
    try:
        return EDataServer, EDataServer.SourceRegistry.new_sync(None)
    except GLib.Error as e:
        raise ToolError(f"couldn't reach the calendar service: {e.message}") from None


WEEKDAYS = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"]


def time_range(when: str, today: dt.date | None = None) -> tuple[dt.datetime, dt.datetime, str]:
    """Everyday words or an ISO date -> (start, end, label), local time."""
    today = today or dt.date.today()
    w = re.sub(r"\s+", " ", (when or "today").strip().casefold())
    start, days, label = today, 1, w
    if w in ("", "today", "now"):
        label = "today"
    elif w == "tomorrow":
        start = today + dt.timedelta(days=1)
    elif w == "yesterday":
        start = today - dt.timedelta(days=1)
    elif w in ("this week", "week"):
        start, days = today, 7 - today.weekday()
    elif w == "next week":
        start, days = today + dt.timedelta(days=7 - today.weekday()), 7
    elif w in ("next 7 days", "coming week", "upcoming"):
        days = 7
    elif w.removeprefix("on ").removeprefix("next ") in WEEKDAYS:
        target = WEEKDAYS.index(w.removeprefix("on ").removeprefix("next "))
        ahead = (target - today.weekday()) % 7
        start = today + dt.timedelta(days=ahead or (7 if w.startswith("next") else 0))
    else:
        try:
            start = dt.date.fromisoformat(w)
        except ValueError:
            raise ToolError("when must be today, tomorrow, this week, next week, a weekday or a date "
                            "like 2026-11-20") from None
        label = start.strftime("%A %Y-%m-%d")
    begin = dt.datetime.combine(start, dt.time())
    return begin, begin + dt.timedelta(days=days), label


def _ical_to_local(t) -> tuple[dt.datetime, bool]:
    """An ICalGLib.Time -> (local naive datetime, all-day?)."""
    if t.is_date():
        return dt.datetime(t.get_year(), t.get_month(), t.get_day()), True
    stamp = t.as_timet_with_zone(t.get_timezone()) if t.get_timezone() else t.as_timet()
    return dt.datetime.fromtimestamp(stamp), False


def format_events(events: list[dict], label: str) -> str:
    if not events:
        return f"No events {label}." if label in ("today", "tomorrow", "yesterday") else f"No events ({label})."
    lines = []
    for e in sorted(events, key=lambda e: (e["start"].date(), not e["all_day"], e["start"]))[:MAX_EVENTS]:
        day = e["start"].strftime("%a %d %b")
        when = "all day" if e["all_day"] else e["start"].strftime("%H:%M") + "–" + e["end"].strftime("%H:%M")
        extra = f" @ {e['location']}" if e.get("location") else ""
        lines.append(f"{day}, {when}: {e['summary']}{extra} ({e['calendar']})")
    return "\n".join(lines)


def _events(start: dt.datetime, end: dt.datetime) -> list[dict]:
    from gi.repository import GLib
    EDataServer, registry = _registry()
    ECal, _ical = _eds(("ECal", "2.0"), ("ICalGLib", "3.0"))
    events = []
    for source in registry.list_enabled(EDataServer.SOURCE_EXTENSION_CALENDAR):
        try:
            client = ECal.Client.connect_sync(source, ECal.ClientSourceType.EVENTS, 10, None)
        except GLib.Error:
            continue  # an account that's offline or broken shouldn't hide the rest

        def add(comp, inst_start, inst_end, *_rest, source=source):
            s, all_day = _ical_to_local(inst_start)
            e, _ = _ical_to_local(inst_end) if inst_end else (s, all_day)
            events.append({"summary": comp.get_summary() or "(no title)", "start": s, "end": e,
                           "all_day": all_day, "location": comp.get_location() or "",
                           "calendar": source.get_display_name()})
            return True

        try:
            client.generate_instances_sync(int(start.timestamp()), int(end.timestamp()), None, add)
        except (GLib.Error, TypeError):
            continue
    return events


def get_events(ctx: Context, args: dict) -> str:
    start, end, label = time_range(str(args.get("when", "today")))
    return format_events(_events(start, end), label)


def _contact_fields(contact) -> dict:
    get = contact.get_property
    out = {"name": get("full-name") or get("file-as") or "", "emails": [], "phones": []}
    for prop in ("email-1", "email-2", "email-3", "email-4"):
        if get(prop):
            out["emails"].append(get(prop))
    for prop, kind in (("mobile-phone", "mobile"), ("home-phone", "home"), ("business-phone", "work"),
                       ("primary-phone", "")):
        value = get(prop)
        if value and value not in [p for p, _ in out["phones"]]:
            out["phones"].append((value, kind))
    for prop in ("org", "birth-date"):
        value = get(prop)
        if value:
            out[prop] = value if isinstance(value, str) else str(value)
    return out


def format_contact(c: dict) -> str:
    parts = [c["name"] or "(no name)"]
    if c["emails"]:
        parts.append("email " + ", ".join(c["emails"]))
    if c["phones"]:
        parts.append("phone " + ", ".join(f"{n} ({k})" if k else n for n, k in c["phones"]))
    if c.get("org"):
        parts.append(c["org"])
    return " · ".join(parts)


def find_contact(ctx: Context, args: dict) -> str:
    from gi.repository import GLib
    name = str(args.get("name", "")).strip()
    if not name:
        raise ToolError("say whose details to find")
    EDataServer, registry = _registry()
    EBook, EBookContacts = _eds(("EBook", "1.2"), ("EBookContacts", "1.2"))
    query = EBookContacts.BookQuery.any_field_contains(name).to_string()
    found = []
    for source in registry.list_enabled(EDataServer.SOURCE_EXTENSION_ADDRESS_BOOK):
        try:
            client = EBook.BookClient.connect_sync(source, 10, None)
            ok, contacts = client.get_contacts_sync(query, None)
        except GLib.Error:
            continue
        found += [format_contact(_contact_fields(c)) for c in (contacts or [])]
    if not found:
        return f"No contact matches “{name}”."
    return "\n".join(list(dict.fromkeys(found))[:8])  # the same person can be in two books
