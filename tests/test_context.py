"""Compaction, on-screen text, personal context and app intents."""
import datetime as dt
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from fake_flm import FakeFlm, text_reply
from panda_assistant import personal as P
from panda_assistant import screen
from panda_assistant import tools as T
from panda_assistant.agent import Agent, Callbacks, estimate_tokens, render_transcript
from panda_assistant.flm import FlmClient


try:
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio  # noqa: F401
except (ImportError, ValueError):
    gi = None


class CompactionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        patcher = mock.patch.dict(os.environ, {"XDG_DATA_HOME": self.tmp.name})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.tmp.cleanup)
        self.flm = FakeFlm()
        self.addCleanup(self.flm.close)
        self.agent = Agent(FlmClient(self.flm.url), "m", use_tools=False, machine="About this computer: x",
                           ctx_tokens=3000)

    def fill(self, turns: int, size: int = 900):
        for i in range(turns):
            self.agent.messages += [{"role": "user", "content": f"question {i} " + "x" * size},
                                    {"role": "assistant", "content": f"answer {i} " + "y" * size}]

    def test_old_turns_become_a_summary(self):
        self.fill(6)
        events = []
        self.flm.replies = [(200, text_reply("User asked six questions about x.")), (200, text_reply("Sure."))]
        self.agent.ask("next", Callbacks(compacting=lambda: events.append("compacting")))
        self.assertEqual(events, ["compacting"])
        summary_req, answer_req = self.flm.requests
        self.assertNotIn("tools", summary_req)
        self.assertIn("User: question", summary_req["messages"][1]["content"])  # newest kept if too long
        system = answer_req["messages"][0]["content"]
        self.assertIn("Earlier in this chat (summary):\nUser asked six questions about x.", system)
        self.assertNotIn("question 0", str(answer_req["messages"][1:]))
        self.assertEqual(answer_req["messages"][-1]["content"], "next")
        self.assertGreater(self.agent.compacted, 0)
        self.assertEqual(len(self.agent.messages), 14)  # nothing deleted from the saved chat
        total = sum(estimate_tokens(m) for m in answer_req["messages"])
        self.assertLess(total, 3000 - 1024)

    def test_small_chats_are_left_alone(self):
        self.fill(1, size=50)
        self.flm.replies = [(200, text_reply("Hi."))]
        self.agent.ask("hello", Callbacks())
        self.assertEqual(len(self.flm.requests), 1)
        self.assertEqual(self.agent.summary, "")

    def test_failed_summary_falls_back_to_dropping(self):
        self.fill(6)
        self.flm.replies = [(500, "boom"), (200, text_reply("Ok."))]
        self.agent.ask("next", Callbacks())
        self.assertEqual(self.agent.summary, "")
        msgs = self.flm.requests[-1]["messages"]
        self.assertEqual(msgs[-1]["content"], "next")
        self.assertLess(sum(estimate_tokens(m) for m in msgs), 3000)

    def test_transcript(self):
        text = render_transcript([{"role": "user", "content": "hi"},
                                  {"role": "assistant", "content": "", "tool_calls": [
                                      {"function": {"name": "get_battery", "arguments": "{}"}}]},
                                  {"role": "tool", "content": "81%"},
                                  {"role": "assistant", "content": "81 percent"}], limit=1000)
        self.assertEqual(text, "User: hi\n(Action get_battery {})\n(Result: 81%)\nAssistant: 81 percent")
        self.assertEqual(len(render_transcript([{"role": "user", "content": "z" * 500}], limit=100)), 100)

    def test_flm_remembers_the_last_model(self):
        client = FlmClient(self.flm.url)
        self.flm.replies = [(200, text_reply("a"))]
        list(client.chat_stream("big:9b", [{"role": "user", "content": "q"}]))
        self.assertEqual(client.last_model, "big:9b")


class ScreenWalkTest(unittest.TestCase):
    def tree(self):
        # node: (role, text or None, name, showing, selection, children)
        return ("frame", None, "Win", True, "", [
            ("label", None, "Invoice total", True, "", []),
            ("panel", None, "", False, "", [("text", "hidden tab", "", True, "", [])]),
            ("document web", "Pay by ￼ today", "", True, "Pay by", [
                ("link", "30 Nov", "", True, "", [])]),
        ])

    def walk(self, **kw):
        return screen.collect_text(self.tree(), lambda n: len(n[5]), lambda n, i: n[5][i], lambda n: n[1],
                                   lambda n: n[2], lambda n: n[0], lambda n: n[3], lambda n: n[4], **kw)

    def test_visible_text_and_selection(self):
        text, selected = self.walk()
        self.assertEqual(text, "Invoice total\nPay by today\n30 Nov")
        self.assertEqual(selected, "Pay by")

    def test_limits(self):
        text, _ = self.walk(max_chars=5)
        self.assertEqual(text, "Invoice total"[:5])

    def test_formatting(self):
        self.assertIn("app set to one of these:\nFirefox: News", screen.format_result({"windows": ["Firefox: News"]}))
        out = screen.format_result({"app": "Firefox", "title": "News", "text": "Body", "selected": "Sel"})
        self.assertEqual(out, "Window: News (Firefox)\nSelected text:\nSel\nWindow text:\nBody")
        self.assertIn("look_at_screen", screen.format_result({"app": "x", "title": "y", "text": "", "selected": ""}))


class PersonalTest(unittest.TestCase):
    def test_time_range(self):
        sat = dt.date(2026, 11, 14)  # a Saturday
        start, end, label = P.time_range("tomorrow", sat)
        self.assertEqual((start.date(), (end - start).days, label), (dt.date(2026, 11, 15), 1, "tomorrow"))
        start, end, _ = P.time_range("this week", sat)
        self.assertEqual((start.date(), end.date()), (sat, dt.date(2026, 11, 16)))
        start, end, _ = P.time_range("next week", sat)
        self.assertEqual((start.date(), end.date()), (dt.date(2026, 11, 16), dt.date(2026, 11, 23)))
        self.assertEqual(P.time_range("tuesday", sat)[0].date(), dt.date(2026, 11, 17))
        self.assertEqual(P.time_range("saturday", sat)[0].date(), sat)
        self.assertEqual(P.time_range("next saturday", sat)[0].date(), dt.date(2026, 11, 21))
        self.assertEqual(P.time_range("2026-12-24", sat)[2], "Thursday 2026-12-24")
        with self.assertRaises(T.ToolError):
            P.time_range("someday", sat)

    def test_format_events(self):
        d = dt.datetime(2026, 11, 15, 9, 30)
        events = [{"summary": "Dentist", "start": d, "end": d + dt.timedelta(hours=1), "all_day": False,
                   "location": "Room 2", "calendar": "Personal"},
                  {"summary": "Holiday", "start": d.replace(hour=0, minute=0), "end": d, "all_day": True,
                   "location": "", "calendar": "Work"}]
        self.assertEqual(P.format_events(events, "tomorrow"),
                         "Sun 15 Nov, all day: Holiday (Work)\nSun 15 Nov, 09:30–10:30: Dentist @ Room 2 (Personal)")
        self.assertEqual(P.format_events([], "tomorrow"), "No events tomorrow.")

    def test_fts_terms_are_plain_prefixes(self):
        self.assertEqual(P.fts_terms('the file about lease "Marek" OR* x'), "lease* marek*")
        with self.assertRaises(T.ToolError):
            P.fts_terms("?!")

    def test_search_stays_in_home_and_skips_hidden(self):
        home = Path.home()
        rows = [[(home / "Dokumenty/lease.pdf").as_uri(), "Lease", "rent  is 2000 zł"],
                [(home / ".ssh/id_rsa").as_uri(), "", "lease"],
                ["file:///etc/lease.conf", "", "lease"],
                ["https://example.com/x", "", "lease"]]
        with mock.patch.object(P, "_query", return_value=rows) as q:
            out = P.search_file_contents(T.Context(), {"query": "lease"})
        self.assertEqual(q.call_args.kwargs, {"q": "lease*"})
        self.assertEqual(out, "~/Dokumenty/lease.pdf — Lease\n  “rent is 2000 zł”")

    def test_contact_format(self):
        c = {"name": "Marek Nowak", "emails": ["m@x.pl"], "phones": [("+48 600", "mobile")], "org": "ACME"}
        self.assertEqual(P.format_contact(c), "Marek Nowak · email m@x.pl · phone +48 600 (mobile) · ACME")


@unittest.skipIf(gi is None, "needs PyGObject")
class IntentsTest(unittest.TestCase):
    def test_media_picks_the_playing_player(self):
        from panda_assistant import intents as I
        players = [{"bus": "org.mpris.MediaPlayer2.vlc", "name": "VLC", "status": "Paused", "title": "", "artist": ""},
                   {"bus": "org.mpris.MediaPlayer2.spotify", "name": "Spotify", "status": "Playing",
                    "title": "Song", "artist": "Band"}]
        self.assertEqual(I.choose_player(players)["name"], "Spotify")
        self.assertEqual(I.choose_player(players, "vlc")["name"], "VLC")
        with self.assertRaises(T.ToolError):
            I.choose_player([])
        calls = []
        bus = mock.Mock()
        bus.call_sync.side_effect = lambda *a: calls.append(a[:4])
        with mock.patch.object(I, "_players", return_value=players), mock.patch.object(I, "_bus", return_value=bus):
            self.assertEqual(I.media_control(T.Context(), {"action": "next"}), "Spotify: next.")
            self.assertIn("Spotify: playing, Band – Song", I.media_control(T.Context(), {"action": "status"}))
            with self.assertRaises(T.ToolError):
                I.media_control(T.Context(), {"action": "rewind"})
        self.assertEqual(calls, [("org.mpris.MediaPlayer2.spotify", "/org/mpris/MediaPlayer2",
                                  "org.mpris.MediaPlayer2.Player", "Next")])


@unittest.skipIf(gi is None, "needs PyGObject")
class AppActionsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        apps = Path(self.tmp.name) / "applications"
        apps.mkdir()
        (apps / "fox.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Foxbrowser\nExec=true\nActions=new-window;new-private-window;\n\n"
            "[Desktop Action new-window]\nName=New Window\nExec=true --new\n\n"
            "[Desktop Action new-private-window]\nName=New Private Window\nExec=true --private\n")

    def run_probe(self, code: str) -> str:
        import subprocess
        import sys
        env = dict(os.environ, XDG_DATA_DIRS=self.tmp.name, XDG_DATA_HOME=self.tmp.name + "/none",
                   PYTHONPATH=str(Path(__file__).resolve().parents[1] / "src"))
        proc = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, env=env, timeout=30)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return proc.stdout.strip()

    def test_list_and_run(self):
        out = self.run_probe(
            "from unittest import mock\n"
            "from gi.repository import Gio\n"
            "from panda_assistant import intents as I, tools as T\n"
            "print(I.list_app_actions(T.Context(), {'app': 'foxbrowser'}))\n"
            "with mock.patch.object(Gio.DesktopAppInfo, 'launch_action') as launch:\n"
            "    print(I.run_app_action(T.Context(), {'app': 'foxbrowser', 'action': 'private window'}))\n"
            "    print(launch.call_args.args[0])\n")
        self.assertEqual(out.splitlines(), ["Foxbrowser can: New Window, New Private Window",
                                            "Foxbrowser: New Private Window.", "new-private-window"])


if __name__ == "__main__":
    unittest.main()


def _sparql_ns():
    for ns in ("Tsparql", "Tracker"):
        try:
            import gi as _gi
            _gi.require_version(ns, "3.0")
            return __import__("importlib").import_module(f"gi.repository.{ns}")
        except (ImportError, ValueError, AttributeError):
            continue
    return None


SPARQL = _sparql_ns() if gi else None


@unittest.skipIf(SPARQL is None, "needs TinySPARQL")
class LocalSearchQueryTest(unittest.TestCase):
    """The real queries against an in-memory database with GNOME's (Nepomuk) schema."""

    def setUp(self):
        from gi.repository import Gio
        flags = SPARQL.SparqlConnectionFlags.FTS_ENABLE_STEMMER | SPARQL.SparqlConnectionFlags.FTS_ENABLE_UNACCENT
        self.conn = SPARQL.SparqlConnection.new(flags, None, SPARQL.sparql_get_ontology_nepomuk(), None)
        home = Path.home()
        for i, (rel, title, text) in enumerate([("Dokumenty/lease.pdf", "Lease agreement",
                                                 "Lease agreement with Marek Nowak. Rent is 2000 zloty."),
                                                (".ssh/notes.txt", "", "lease secret")]):
            url = Gio.File.new_for_path(str(home / rel)).get_uri()
            self.conn.update(f'INSERT DATA {{ <{url}> a nfo:FileDataObject ; nie:url "{url}" . '
                             f'<urn:ie:{i}> a nfo:Document ; nie:isStoredAs <{url}> ; nie:title "{title}" ; '
                             f'nie:plainTextContent "{text}" . }}', None)

    def test_search_and_text(self):
        with mock.patch.object(P, "_connection", return_value=self.conn):
            out = P.search_file_contents(T.Context(), {"query": "rent marek"})
            self.assertTrue(out.startswith("~/Dokumenty/lease.pdf — Lease agreement\n  “Lease agreement with Marek"))
            self.assertNotIn(".ssh", P.search_file_contents(T.Context(), {"query": "leas"}))
            self.assertIn("No files mention", P.search_file_contents(T.Context(), {"query": "zzz"}))
            self.assertIn("Rent is 2000", P.indexed_text(Path.home() / "Dokumenty/lease.pdf"))
