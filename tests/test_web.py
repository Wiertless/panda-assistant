import json
import unittest
from unittest import mock

from fake_flm import FakeFlm, text_reply
from panda_assistant import tools as T
from panda_assistant import web
from panda_assistant.agent import Agent, Callbacks
from panda_assistant.flm import FlmClient

LITE = """<table>
<tr><td>1.&nbsp;</td><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ffedoraproject.org%2Fatomic-desktops%2Fsilverblue%2F&amp;rut=abc" class='result-link'>Fedora <b>Silverblue</b></a></td></tr>
<tr><td></td><td class='result-snippet'>An immutable   desktop &amp; more.</td></tr>
<tr><td>2.&nbsp;</td><td><a rel="nofollow" href="https://duckduckgo.com/y.js?ad=1" class='result-link'>Sponsored</a></td></tr>
<tr><td>3.&nbsp;</td><td><a rel="nofollow" href="https://docs.fedoraproject.org/silverblue/" class='result-link'>Silverblue docs</a></td></tr>
</table>"""

HTML_VARIANT = """<div class="result"><a class="result__a" href="https://example.org/a">Example A</a>
<a class="result__snippet" href="https://example.org/a">Snippet <b>A</b></a></div>"""


class ParseTest(unittest.TestCase):
    def test_lite_results(self):
        r = web.parse_duckduckgo(LITE)
        self.assertEqual([x["url"] for x in r], ["https://fedoraproject.org/atomic-desktops/silverblue/",
                                                 "https://docs.fedoraproject.org/silverblue/"])
        self.assertEqual(r[0]["title"], "Fedora Silverblue")
        self.assertEqual(r[0]["snippet"], "An immutable desktop & more.")

    def test_html_variant(self):
        self.assertEqual(web.parse_duckduckgo(HTML_VARIANT),
                         [{"title": "Example A", "url": "https://example.org/a", "snippet": "Snippet A"}])

    def test_page_text_skips_chrome(self):
        title, text = web.page_text("<html><head><title> T </title><style>x{}</style></head><body>"
                                    "<nav>menu</nav><h1>Head</h1><p>Body  text</p><script>bad()</script>"
                                    "<footer>foot</footer></body></html>")
        self.assertEqual(title, "T")
        self.assertEqual(text, "Head\nBody text")


class SearchTest(unittest.TestCase):
    def test_duckduckgo_first(self):
        with mock.patch.object(web, "_get", return_value=(LITE.encode(), "text/html")) as get:
            out = web.web_search("silverblue")
        self.assertIn("Results from DuckDuckGo", out)
        self.assertIn("1. Fedora Silverblue - https://fedoraproject.org/atomic-desktops/silverblue/", out)
        self.assertEqual(get.call_count, 1)

    def test_falls_back_to_wikipedia(self):
        wiki = json.dumps({"query": {"search": [{"title": "Fedora Linux",
                                                 "snippet": "<span>Fedora</span> is a distro"}]}}).encode()

        def fake_get(url, data=None):
            if "duckduckgo" in url:
                return b"<html>anomaly</html>", "text/html"
            return wiki, "application/json"

        with mock.patch.object(web, "_get", side_effect=fake_get):
            out = web.web_search("fedora")
        self.assertIn("Results from Wikipedia", out)
        self.assertIn("https://en.wikipedia.org/wiki/Fedora_Linux", out)
        self.assertIn("Fedora is a distro", out)

    def test_no_results_is_an_error(self):
        with mock.patch.object(web, "_get", side_effect=web.WebError("offline")):
            with self.assertRaises(web.WebError):
                web.web_search("x")

    def test_page_is_truncated(self):
        html = "<html><title>Long</title><p>" + "word " * 5000 + "</p></html>"
        with mock.patch.object(web, "_get", return_value=(html.encode(), "text/html")):
            out = web.read_web_page("https://example.org")
        self.assertTrue(out.startswith("Long\nhttps://example.org"))
        self.assertLess(len(out), web.PAGE_CHARS + 200)
        self.assertTrue(out.endswith("[…]"))

    def test_read_tool_rejects_non_http(self):
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "read_web_page", {"url": "file:///etc/passwd"})


class AgentWebTest(unittest.TestCase):
    def setUp(self):
        self.flm = FakeFlm()

    def tearDown(self):
        self.flm.close()

    def tool_names(self, agent):
        return [t["function"]["name"] for t in agent._tool_schemas()]

    def test_web_tools_follow_the_switch(self):
        agent = Agent(FlmClient(self.flm.url), "m")
        self.assertIn("web_search", self.tool_names(agent))
        agent.web_enabled = False
        self.assertNotIn("web_search", self.tool_names(agent))
        self.assertNotIn("read_web_page", self.tool_names(agent))

    def test_machine_context_is_in_the_prompt_and_stable(self):
        agent = Agent(FlmClient(self.flm.url), "m")
        self.flm.replies = [(200, text_reply("a")), (200, text_reply("b"))]
        agent.ask("one", Callbacks())
        agent.ask("two", Callbacks())
        first, second = (r["messages"][0]["content"] for r in self.flm.requests)
        self.assertIn("About this computer:", first)
        self.assertIn("Memory:", first)
        self.assertEqual(first, second)
        agent.reset()
        self.assertIsNone(agent.machine)


if __name__ == "__main__":
    unittest.main()
