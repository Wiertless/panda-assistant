"""The small language the training conversations are written in (data.py,
generated.py): T is one tool call and its result, ex a turn, chat a turn
answered through the reply tool."""

POLISH = set("ąćęłńóśźżĄĆĘŁŃÓŚŹŻ")
PL_WORDS = {"ile", "jak", "czy", "co", "gdzie", "kiedy", "jest", "nie", "mi", "mnie", "sie", "prosze",
            "dzieki", "zrob", "pokaz", "otworz", "wlacz", "wylacz", "ustaw", "jutro", "dzis", "teraz",
            "troche", "ciszej", "glosniej", "napisz", "przypomnij", "znajdz", "wyslij", "puść", "pusc"}


def lang_of(text):
    if any(c in POLISH for c in text):
        return "Polish"
    words = {w.strip(".,?!").lower() for w in text.split()}
    return "Polish" if words & PL_WORDS else "English"


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
    EXAMPLES.append(dict(user=user, steps=steps, answer=answer, before=before, lang=lang, src=SOURCE))


def chat(user, answer, before=(), lang=None):
    """Small talk or general knowledge: the answer goes through the reply tool."""
    EXAMPLES.append(dict(user=user, steps=(), answer=answer, before=before, lang=lang, reply=True, src=SOURCE))


