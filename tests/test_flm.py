import unittest

from fake_flm import sse, text_reply, tool_reply
from panda_assistant.flm import parse_flm_list, parse_sse_lines, strip_think


def events(chunks):
    return list(parse_sse_lines(sse(chunks).decode().splitlines()))


class SseTest(unittest.TestCase):
    def test_text_and_reasoning(self):
        ev = events(text_reply("Hel", "lo", reasoning="hmm"))
        self.assertEqual([e.kind for e in ev], ["reasoning", "text", "text", "done"])
        self.assertEqual("".join(e.text for e in ev if e.kind == "text"), "Hello")
        self.assertEqual(ev[-1].finish_reason, "stop")

    def test_tool_call_deltas_merge_by_index(self):
        done = events(tool_reply("set_volume", '{"percent": 40}'))[-1]
        self.assertEqual(len(done.tool_calls), 1)
        call = done.tool_calls[0]
        self.assertEqual((call.id, call.name), ("call_1", "set_volume"))
        self.assertEqual(call.parsed_arguments(), {"percent": 40})
        self.assertEqual(done.finish_reason, "tool_calls")

    def test_empty_arguments(self):
        call = events(tool_reply("get_battery", ""))[-1].tool_calls[0]
        self.assertEqual(call.parsed_arguments(), {})


class HelpersTest(unittest.TestCase):
    def test_strip_think(self):
        self.assertEqual(strip_think("<think>plan</think>\nAnswer"), "Answer")
        self.assertEqual(strip_think("<think>unfinished"), "")

    def test_parse_flm_list(self):
        out = "FLM v1.0.9\nModels:\n  - gemma3:1b ✅\n  - gpt-oss:20b ⏬\n"
        self.assertEqual(parse_flm_list(out), {"gemma3:1b": True, "gpt-oss:20b": False})


if __name__ == "__main__":
    unittest.main()
