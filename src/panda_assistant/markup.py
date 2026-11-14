"""A small Markdown subset to Pango markup, for model answers.

Handles code blocks, inline code, bold, italics, headings and bullet lists.
Everything is escaped first, so model output can never inject markup.
"""

from __future__ import annotations

import re
from html import escape

_CODE_BLOCK = re.compile(r"```[^\n]*\n(.*?)(```|$)", re.S)
_INLINE_CODE = re.compile(r"`([^`\n]+)`")
_BOLD = re.compile(r"\*\*(?=\S)(.+?)(?<=\S)\*\*")
_ITALIC = re.compile(r"(?<![\w*])\*(?=\S)([^*\n]+?)(?<=\S)\*(?![\w*])")
_HEADING = re.compile(r"^#{1,6}\s+(.*)$", re.M)
_BULLET = re.compile(r"^(\s*)[-*]\s+", re.M)


def to_pango(text: str) -> str:
    blocks: list[str] = []

    def stash(m: re.Match) -> str:
        blocks.append(f"<tt>{escape(m.group(1).rstrip(), quote=False)}</tt>")
        return f"\x00{len(blocks) - 1}\x00"

    text = _CODE_BLOCK.sub(stash, text)
    text = escape(text, quote=False)
    text = _INLINE_CODE.sub(r"<tt>\1</tt>", text)
    text = _BOLD.sub(r"<b>\1</b>", text)
    text = _ITALIC.sub(r"<i>\1</i>", text)
    text = _HEADING.sub(r"<b>\1</b>", text)
    text = _BULLET.sub(lambda m: m.group(1) + "• ", text)
    return re.sub(r"\x00(\d+)\x00", lambda m: blocks[int(m.group(1))], text)
