import json
import unittest

from fake_flm import FakeFlm, text_reply, tool_reply
from panda_assistant import tools as T
from panda_assistant.agent import Agent, Callbacks
from panda_assistant.flm import FlmClient


class Recorder(Callbacks):
    def __init__(self, allow=False):
        super().__init__()
        self.log = []
        self.text = lambda s: self.log.append(("text", s))
        self.tool_finished = lambda n, r, ok: self.log.append(("tool", n, ok, r))
        self.notice = lambda s: self.log.append(("notice", s))
        self.confirm = lambda n, s: self.log.append(("confirm", n, s)) or allow


class AgentTest(unittest.TestCase):
    def setUp(self):
        self.flm = FakeFlm()
        self.agent = Agent(FlmClient(self.flm.url), "test-model")

    def tearDown(self):
        self.flm.close()

    def test_safe_tool_runs_and_result_goes_back(self):
        self.flm.replies = [(200, tool_reply("get_datetime", "{}")), (200, text_reply("It's late."))]
        rec = Recorder()
        self.agent.ask("what time is it", rec)
        self.assertIn(("text", "It's late."), rec.log)
        self.assertEqual([e[0] for e in rec.log if e[0] in ("confirm", "tool")], ["tool"])
        second = self.flm.requests[1]["messages"]
        self.assertEqual(second[-2]["tool_calls"][0]["function"]["name"], "get_datetime")
        self.assertEqual(second[-1]["role"], "tool")
        self.assertEqual(second[-1]["tool_call_id"], "call_1")
        self.assertIn("tools", self.flm.requests[0])

    def test_risky_tool_asks_and_declines(self):
        self.flm.replies = [(200, tool_reply("set_wifi", '{"enabled": false}')), (200, text_reply("OK."))]
        rec = Recorder(allow=False)
        self.agent.ask("turn off wifi", rec)
        self.assertIn(("confirm", "set_wifi", "Turn Wi-Fi off"), rec.log)
        tool_msg = self.flm.requests[1]["messages"][-1]
        self.assertEqual(tool_msg["content"], "The user declined this action.")

    def test_bad_arguments_are_reported_not_run(self):
        self.flm.replies = [(200, tool_reply("set_volume", '{"percent": 500}')), (200, text_reply("Sorry."))]
        rec = Recorder()
        self.agent.ask("max volume", rec)
        tool = [e for e in rec.log if e[0] == "tool"][0]
        self.assertFalse(tool[2])
        self.assertIn("0 to 100", tool[3])

    def test_unknown_tool(self):
        self.flm.replies = [(200, tool_reply("rm_rf", "{}")), (200, text_reply("Can't."))]
        rec = Recorder()
        self.agent.ask("delete stuff", rec)
        self.assertIn("no tool named rm_rf", self.flm.requests[1]["messages"][-1]["content"])

    def test_model_without_tools_falls_back_to_chat(self):
        self.flm.replies = [(400, "tools not supported"), (200, text_reply("Hi!"))]
        rec = Recorder()
        self.agent.ask("hello", rec)
        self.assertNotIn("tools", self.flm.requests[1])
        self.assertFalse(self.agent.use_tools)
        self.assertTrue(any(e[0] == "notice" for e in rec.log))

    def test_history_is_kept(self):
        self.flm.replies = [(200, text_reply("One.")), (200, text_reply("Two."))]
        self.agent.ask("first", Recorder())
        self.agent.ask("second", Recorder())
        roles = [m["role"] for m in self.flm.requests[1]["messages"]]
        self.assertEqual(roles, ["system", "user", "assistant", "user"])

    def test_cancel_stops_before_tools(self):
        self.flm.replies = [(200, tool_reply("get_battery", "{}"))]
        rec = Recorder()
        rec.text = lambda s: None
        orig = self.agent._stream

        def stream_then_cancel(*a):
            out = orig(*a)
            self.agent.cancel.set()
            return out
        self.agent._stream = stream_then_cancel
        self.agent.ask("battery", rec)
        self.assertFalse(any(e[0] == "tool" for e in rec.log))
        self.assertEqual(len(self.flm.requests), 1)


class ToolsTest(unittest.TestCase):
    def test_schemas_are_valid_json(self):
        json.dumps(T.schemas())
        self.assertEqual(len({t.name for t in T.TOOLS}), len(T.TOOLS))

    def test_url_check(self):
        for url in ("file:///etc/passwd", "javascript:alert(1)", "ssh://host", "data:text/html,x", "", "a b.com"):
            with self.assertRaises(T.ToolError, msg=url):
                T.check_url(url)
        self.assertEqual(T.check_url("example.com"), "https://example.com")
        self.assertEqual(T.check_url("localhost:8080/x"), "https://localhost:8080/x")
        self.assertEqual(T.check_url("http://example.com/a?b=1"), "http://example.com/a?b=1")

    def test_open_file_stays_in_home(self):
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "open_file", {"path": "/etc/passwd"})

    def test_only_connectivity_needs_confirmation(self):
        risky = {t.name for t in T.TOOLS if t.risk == T.CONFIRM}
        self.assertEqual(risky, {"set_wifi", "set_bluetooth", "change_setting"})

    def test_datetime_and_battery_run(self):
        self.assertTrue(T.run_tool(T.Context(), "get_datetime", {}))
        self.assertTrue(T.run_tool(T.Context(), "get_battery", {}))


if __name__ == "__main__":
    unittest.main()
