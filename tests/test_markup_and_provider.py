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
