"""The small language the training conversations are written in (data.py,
generated.py): T is one tool call and its result, ex a turn, chat a turn
answered through the reply tool."""

import re

POLISH = set("ąćęłńóśźżĄĆĘŁŃÓŚŹŻ")
# The app's lists (core/src/agent.rs, language()): the note "(Answer in X.)" must
# come out exactly as the app writes it, or not at all when the app adds none.
PL_WORDS = ['albo', 'bardzo', 'ciszej', 'co', 'czesc', 'czy', 'daj', 'dla', 'dlaczego', 'dodaj', 'dzieki', 'dziekuje', 'dzis', 'dzisiaj', 'gdzie', 'glosniej', 'ile', 'jak', 'jaka', 'jaki', 'jakie', 'jest', 'jestem', 'jutro', 'kiedy', 'mam', 'masz', 'mi', 'mnie', 'moj', 'moja', 'moje', 'napisz', 'nastaw', 'nie', 'odpal', 'otworz', 'podlacz', 'pokaz', 'poprosze', 'prosze', 'przelacz', 'przenies', 'przez', 'przypomnij', 'pusc', 'rozlacz', 'sa', 'sie', 'tak', 'teraz', 'troche', 'uruchom', 'ustaw', 'usun', 'wlacz', 'wylacz', 'wyslij', 'zadzwon', 'zamknij', 'zapisz', 'zatrzymaj', 'zmniejsz', 'znajdz', 'zrob', 'zwieksz']
EN_WORDS = ['an', 'and', 'are', 'can', 'could', 'did', 'does', 'find', 'for', 'hello', 'hi', 'how', 'is', 'it', 'me', 'my', 'of', 'open', 'play', 'please', 'send', 'set', 'show', 'thank', 'thanks', 'that', 'the', 'this', 'turn', 'what', 'whats', 'when', 'where', 'who', 'why', 'with', 'write', 'you']
CONTEXT_END = "\n[/On screen]\n\n"


def lang_of(text):
    """The app's language(): the user's own words (after any screen context), Polish
    by its letters or more Polish than English common words, None when unclear."""
    own = text.split(CONTEXT_END, 1)[1] if text.startswith("[On screen: ") and CONTEXT_END in text else text
    if any(c in POLISH for c in own):
        return "Polish"
    words = [w.lower() for w in re.split(r"[^\w]+", own) if w]
    p, e = sum(w in PL_WORDS for w in words), sum(w in EN_WORDS for w in words)
    return "Polish" if p > e else "English" if e > p else None


class T:
    """One tool call and what it returned."""

    def __init__(self, tool, result, **args):
        self.tool, self.result, self.args = tool, result, args


EXAMPLES = []
# "hand" or "gen": a generated wording that happens to be an eval prompt is
# skipped; a hand-written one is an error.
SOURCE = "hand"


def ex(user, *steps, answer=None, before=(), lang=None):
    """A turn: the user's words, the tool steps in order, the answer after them
    (None for a one-action request the app answers with the tool's own
    confirmation), and earlier turns as (user, assistant) pairs."""
    # No tool steps: the first round must call a tool, so the answer goes through reply.
    EXAMPLES.append(dict(user=user, steps=steps, answer=answer, before=before, lang=lang, src=SOURCE,
                         reply=not steps))


def chat(user, answer, before=(), lang=None):
    """Small talk or general knowledge: the answer goes through the reply tool."""
    EXAMPLES.append(dict(user=user, steps=(), answer=answer, before=before, lang=lang, reply=True, src=SOURCE))


