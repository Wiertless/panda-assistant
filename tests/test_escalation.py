import unittest

from fake_flm import FakeFlm, text_reply, tool_reply
from panda_assistant.agent import ESCALATE, Agent, Callbacks
from panda_assistant.flm import FlmClient, Model, pick_smarter_model


def models(*specs):
    return [Model(name, installed, labels) for name, installed, labels in specs]


TOOLS = ["tool-calling"]


class PickTest(unittest.TestCase):
    def test_size(self):
        self.assertEqual(Model("gemma4-it:e4b").size_b, 4)
        self.assertEqual(Model("qwen3.6-moe:35b-a3b").size_b, 35)
        self.assertEqual(Model("lfm2:1.2b").size_b, 1.2)
        self.assertEqual(Model("embed-gemma:300m").size_b, 0.3)

    def test_biggest_installed_tool_model_that_fits(self):
        ms = models(("gemma4-it:e2b", True, TOOLS), ("gemma4-it:e4b", True, TOOLS),
                    ("qwen3.5:9b", True, TOOLS), ("gemma4-it:12b", False, TOOLS),
                    ("gemma3:4b", True, []), ("qwen3.8-mtp:27b", True, TOOLS),
                    ("gemma4e-flash:e4b", True, TOOLS + ["single-turn"]))
        self.assertEqual(pick_smarter_model(ms, "gemma4-it:e2b", ram_gib=16), "qwen3.5:9b")
        self.assertEqual(pick_smarter_model(ms, "gemma4-it:e2b", ram_gib=64), "qwen3.8-mtp:27b")

    def test_none_when_nothing_bigger(self):
        ms = models(("gemma4-it:e2b", True, TOOLS), ("qwen3:0.6b", True, []))
        self.assertIsNone(pick_smarter_model(ms, "gemma4-it:e2b", ram_gib=16))


class Rec(Callbacks):
    def __init__(self):
        super().__init__()
        self.log = []
        self.text = lambda s: self.log.append(("text", s))
        self.handoff = lambda m: self.log.append(("handoff", m))
        self.tool_finished = lambda n, r, ok: self.log.append(("tool", n, ok))


class EscalationTest(unittest.TestCase):
    def setUp(self):
        self.flm = FakeFlm()
        self.agent = Agent(FlmClient(self.flm.url), "small", smarter_model="big")

    def tearDown(self):
        self.flm.close()

    def test_offered_only_when_configured(self):
        names = lambda a: [t["function"]["name"] for t in a._tool_schemas()]
        self.assertIn(ESCALATE, names(self.agent))
        self.assertNotIn(ESCALATE, names(Agent(FlmClient(self.flm.url), "small")))
        self.assertNotIn(ESCALATE, names(Agent(FlmClient(self.flm.url), "big", smarter_model="big")))

    def test_hand_off_finishes_turn_on_the_big_model(self):
        self.flm.replies = [
            (200, tool_reply(ESCALATE, '{"task": "plan a trip"}')),
            (200, tool_reply("get_datetime", "{}")),
            (200, text_reply("Here's your plan.")),
        ]
        rec = Rec()
        self.agent.ask("Plan me a weekend trip", rec)
        self.assertEqual([r["model"] for r in self.flm.requests], ["small", "big", "big"])
        self.assertIn(("handoff", "big"), rec.log)
        self.assertIn(("tool", "get_datetime", True), rec.log)
        big_first = self.flm.requests[1]
        self.assertNotIn(ESCALATE, [t["function"]["name"] for t in big_first["tools"]])
        self.assertIn("plan a trip", big_first["messages"][-1]["content"])
        roles = [m["role"] for m in self.agent.messages]
        self.assertEqual(roles, ["user", "assistant", "tool", "assistant"])
        self.assertEqual(self.agent.messages[-1]["content"], "Here's your plan.")

    def test_hand_off_without_a_smarter_model_is_an_unknown_tool(self):
        agent = Agent(FlmClient(self.flm.url), "small")
        self.flm.replies = [(200, tool_reply(ESCALATE, "{}")), (200, text_reply("OK, I'll try."))]
        rec = Rec()
        agent.ask("hard one", rec)
        self.assertNotIn(("handoff", None), rec.log)
        self.assertEqual([r["model"] for r in self.flm.requests], ["small", "small"])
        self.assertIn("no tool named", self.flm.requests[1]["messages"][-1]["content"])

    def test_next_turn_history_is_valid_for_the_small_model(self):
        self.flm.replies = [(200, tool_reply(ESCALATE, "{}")), (200, text_reply("Big answer.")),
                            (200, text_reply("Small answer."))]
        self.agent.ask("hard one", Rec())
        self.agent.ask("easy one", Rec())
        last = self.flm.requests[-1]
        self.assertEqual(last["model"], "small")
        self.assertEqual([m["role"] for m in last["messages"]],
                         ["system", "user", "assistant", "tool", "assistant", "user"])


if __name__ == "__main__":
    unittest.main()
