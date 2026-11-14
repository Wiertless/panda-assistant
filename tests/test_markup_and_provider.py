import unittest

from panda_assistant.markup import to_pango


class MarkupTest(unittest.TestCase):
    def test_escapes_first(self):
        self.assertEqual(to_pango("<b>x</b> & y"), "&lt;b&gt;x&lt;/b&gt; &amp; y")

    def test_basic_markdown(self):
        self.assertEqual(to_pango("**bold** and *it* and `code`"), "<b>bold</b> and <i>it</i> and <tt>code</tt>")
        self.assertEqual(to_pango("# Title\n- one\n- two"), "<b>Title</b>\n• one\n• two")

    def test_code_block_is_not_formatted(self):
        self.assertEqual(to_pango("```sh\necho **hi** <x>\n```"), "<tt>echo **hi** &lt;x&gt;</tt>")


try:
    from panda_assistant import search_provider as sp
except (ImportError, ValueError):  # no GObject introspection here
    sp = None


@unittest.skipIf(sp is None, "needs PyGObject")
class SearchProviderTest(unittest.TestCase):
    def test_results(self):
        self.assertEqual(sp.results_for(["hi"]), [])
        self.assertEqual(sp.results_for(["what", "time?"]), ["what time?"])

    def test_meta(self):
        meta = sp.meta_for("hello there")
        self.assertEqual(meta["name"].unpack(), "Ask: hello there")
        self.assertEqual(meta["id"].unpack(), "hello there")


if __name__ == "__main__":
    unittest.main()


@unittest.skipIf(sp is None, "needs PyGObject")
class SearchResultsTest(unittest.TestCase):
    def setUp(self):
        import os
        import tempfile
        from unittest import mock
        self.tmp = tempfile.TemporaryDirectory()
        patcher = mock.patch.dict(os.environ, {"XDG_DATA_HOME": self.tmp.name})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.tmp.cleanup)

    def test_skills_and_chats_follow_the_ask_result(self):
        from panda_assistant import store
        store.save_skill("Morning Routine", "1. dark mode", "Start the day")
        store.save_chat("c1", [{"role": "user", "content": "Plan my morning run"}], "m")
        store.save_chat("c2", [{"role": "user", "content": "Something else"}], "m")
        results = sp.results_for(["morning"])
        self.assertEqual(results, ["morning", sp.SKILL + "morning-routine", sp.CHAT + "c1"])
        self.assertEqual(sp.meta_for(results[1])["name"].unpack(), "Run skill: Morning Routine")
        self.assertEqual(sp.meta_for(results[2])["name"].unpack(), "Plan my morning run")
        self.assertEqual(sp.results_for(["/morning", "routine"])[1], sp.SKILL + "morning-routine")

    def test_activation_routes(self):
        from unittest import mock
        with mock.patch.object(sp, "launch_app") as launch:
            sp.activate(sp.SKILL + "morning")
            sp.activate(sp.CHAT + "c1")
            sp.activate("what time")
        self.assertEqual([c.args for c in launch.call_args_list],
                         [("ask", "/morning"), ("open-chat", "c1"), ("ask", "what time")])
