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


if __name__ == "__main__":
    unittest.main()
