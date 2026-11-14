"""Web search and page reading for the model, stdlib only.

Search uses DuckDuckGo's lightweight HTML page (no API key). If that
returns nothing (layout change, bot check), it falls back to Wikipedia's
search API. Results are kept short: local models run with small contexts.
"""

from __future__ import annotations

import json
import re
import urllib.error
import urllib.parse
import urllib.request
from html import unescape
from html.parser import HTMLParser

USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) Panda-Assistant"
TIMEOUT = 12
MAX_PAGE_BYTES = 2_000_000
MAX_RESULTS = 5
SNIPPET_CHARS = 220
PAGE_CHARS = 3000


class WebError(Exception):
    pass


def _get(url: str, data: bytes | None = None) -> tuple[bytes, str]:
    req = urllib.request.Request(url, data=data, headers={
        "User-Agent": USER_AGENT, "Accept-Language": "en-US,en;q=0.8"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            return resp.read(MAX_PAGE_BYTES), resp.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        raise WebError(f"{url} returned HTTP {e.code}") from None
    except (OSError, urllib.error.URLError) as e:
        raise WebError(f"couldn't reach {urllib.parse.urlparse(url).hostname}: {e}") from None


def _clean(text: str) -> str:
    return re.sub(r"\s+", " ", unescape(text)).strip()


def _unwrap(href: str) -> str:
    """DuckDuckGo wraps result links as //duckduckgo.com/l/?uddg=<url>."""
    if href.startswith("//"):
        href = "https:" + href
    parts = urllib.parse.urlparse(href)
    if parts.hostname and parts.hostname.endswith("duckduckgo.com") and parts.path.startswith("/l/"):
        target = urllib.parse.parse_qs(parts.query).get("uddg", [""])[0]
        if target:
            return target
    return href


class _DdgParser(HTMLParser):
    """Results from lite.duckduckgo.com (result-link / result-snippet) and
    html.duckduckgo.com (result__a / result__snippet)."""

    def __init__(self):
        super().__init__()
        self.results: list[dict] = []
        self._field = None
        self._buf: list[str] = []

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        cls = a.get("class") or ""
        if tag == "a" and ("result-link" in cls or "result__a" in cls):
            self.results.append({"title": "", "url": _unwrap(a.get("href", "")), "snippet": ""})
            self._field, self._buf = "title", []
        elif ("result-snippet" in cls or "result__snippet" in cls) and self.results:
            self._field, self._buf = "snippet", []

    def handle_endtag(self, tag):
        if self._field and tag in ("a", "td", "div"):
            self.results[-1][self._field] = _clean("".join(self._buf))
            self._field = None

    def handle_data(self, data):
        if self._field:
            self._buf.append(data)


def parse_duckduckgo(html: str) -> list[dict]:
    parser = _DdgParser()
    parser.feed(html)
    out = []
    for r in parser.results:
        if r["title"] and r["url"].startswith(("http://", "https://")) and "duckduckgo.com/y.js" not in r["url"]:
            out.append(r)
    return out


def search_duckduckgo(query: str) -> list[dict]:
    body, _ = _get("https://lite.duckduckgo.com/lite/", urllib.parse.urlencode({"q": query}).encode())
    return parse_duckduckgo(body.decode("utf-8", errors="replace"))


def search_wikipedia(query: str, lang: str = "en") -> list[dict]:
    url = (f"https://{lang}.wikipedia.org/w/api.php?"
           + urllib.parse.urlencode({"action": "query", "list": "search", "srsearch": query,
                                     "srlimit": MAX_RESULTS, "format": "json", "utf8": 1}))
    body, _ = _get(url)
    hits = json.loads(body).get("query", {}).get("search", [])
    return [{"title": h["title"],
             "url": f"https://{lang}.wikipedia.org/wiki/" + urllib.parse.quote(h["title"].replace(" ", "_")),
             "snippet": _clean(re.sub(r"<[^>]+>", "", h.get("snippet", "")))} for h in hits]


def web_search(query: str) -> str:
    query = query.strip()
    if not query:
        raise WebError("empty search")
    results, source, errors = [], "DuckDuckGo", []
    try:
        results = search_duckduckgo(query)
    except WebError as e:
        errors.append(str(e))
    if not results:
        source = "Wikipedia"
        try:
            results = search_wikipedia(query)
        except WebError as e:
            errors.append(str(e))
    if not results:
        raise WebError("no results" + (f" ({'; '.join(errors)})" if errors else ""))
    lines = [f"Results from {source} for '{query}':"]
    for i, r in enumerate(results[:MAX_RESULTS], 1):
        snippet = r["snippet"][:SNIPPET_CHARS]
        lines.append(f"{i}. {r['title']} - {r['url']}" + (f"\n   {snippet}" if snippet else ""))
    lines.append("Use read_web_page on a result for details.")
    return "\n".join(lines)


class _TextParser(HTMLParser):
    SKIP = {"script", "style", "noscript", "nav", "header", "footer", "aside", "form", "svg", "template"}
    BLOCK = {"p", "div", "li", "br", "h1", "h2", "h3", "h4", "tr", "section", "article", "pre"}

    def __init__(self):
        super().__init__()
        self.parts: list[str] = []
        self.title = ""
        self._skip = 0
        self._in_title = False

    def handle_starttag(self, tag, attrs):
        if tag in self.SKIP:
            self._skip += 1
        elif tag == "title":
            self._in_title = True
        elif tag in self.BLOCK:
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in self.SKIP and self._skip:
            self._skip -= 1
        elif tag == "title":
            self._in_title = False

    def handle_data(self, data):
        if self._in_title:
            self.title += data
        elif not self._skip:
            self.parts.append(data)


def page_text(html: str) -> tuple[str, str]:
    p = _TextParser()
    p.feed(html)
    lines = [_clean(line) for line in "".join(p.parts).split("\n")]
    return _clean(p.title), "\n".join(line for line in lines if len(line) > 1)


def read_web_page(url: str) -> str:
    body, ctype = _get(url)
    text = body.decode("utf-8", errors="replace")
    if "html" in ctype or text.lstrip()[:15].lower().startswith(("<!doctype", "<html")):
        title, text = page_text(text)
    elif ctype.startswith(("text/", "application/json")):
        title = ""
    else:
        raise WebError(f"can't read {ctype or 'this kind of file'}")
    if len(text) > PAGE_CHARS:
        text = text[:PAGE_CHARS].rsplit(" ", 1)[0] + " […]"
    return (f"{title}\n{url}\n\n" if title else f"{url}\n\n") + (text or "(no readable text)")
