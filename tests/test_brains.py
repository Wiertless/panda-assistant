"""Memory, skills, chat history, pictures, networks, Bluetooth and commands."""
import os
import stat
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

from fake_flm import FakeFlm, text_reply, tool_reply
from panda_assistant import store
from panda_assistant import tools as T
from panda_assistant.agent import IMAGE_MARKER, Agent, Callbacks
from panda_assistant.flm import FlmClient, Model, pick_vision_model

IMG = "data:image/jpeg;base64,AAAA"


class DataDir(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        patcher = mock.patch.dict(os.environ, {"XDG_DATA_HOME": self.tmp.name})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.addCleanup(self.tmp.cleanup)


class MemoryTest(DataDir):
    def test_remember_and_forget(self):
        self.assertEqual(store.facts(), [])
        self.assertIn("Remembered", store.remember("Prefers   metric units"))
        self.assertEqual(store.remember("prefers metric units."), "Already remembered.")
        store.remember("Uses fish shell")
        self.assertEqual(store.facts(), ["Prefers metric units", "Uses fish shell"])
        self.assertTrue(store.memory_file().read_text().startswith("# Things the assistant remembers"))
        self.assertEqual(store.forget("fish"), ["Uses fish shell"])
        self.assertEqual(store.forget("nothing like this"), [])
        with self.assertRaises(ValueError):
            store.remember("x" * 500)

    def test_memory_tools(self):
        self.assertIn("Remembered", T.run_tool(T.Context(), "remember", {"fact": "Has a Framework 13"}))
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "forget", {"fact": "zzz"})
        self.assertIn("Forgot", T.run_tool(T.Context(), "forget", {"fact": "framework"}))

    def test_oldest_facts_drop_off(self):
        for i in range(store.MAX_FACTS + 3):
            store.remember(f"fact {i}")
        facts = store.facts()
        self.assertEqual(len(facts), store.MAX_FACTS)
        self.assertEqual(facts[0], "fact 3")


class SkillTest(DataDir):
    def test_save_find_run(self):
        skill = store.save_skill("Morning Routine", "1. Dark mode on\n2. Volume 30%", "Start the day")
        self.assertEqual(skill.slug, "morning-routine")
        found = store.find_skill("/morning")
        self.assertEqual(found.title, "Morning Routine")
        self.assertEqual(found.description, "Start the day")
        self.assertIn("2. Volume 30%", found.steps)
        self.assertIn("Details: quietly", store.skill_prompt(found, "quietly"))
        self.assertIn("Do these steps now", T.run_tool(T.Context(), "use_skill", {"name": "morning routine"}))
        self.assertTrue(store.delete_skill("morning-routine"))
        self.assertIsNone(store.find_skill("morning"))

    def test_hand_written_skill_files(self):
        store.skills_dir().mkdir(parents=True)
        (store.skills_dir() / "backup.md").write_text("- copy Documents to the USB drive\n- eject it\n")
        (store.skills_dir() / "empty.md").write_text("# Empty\n")
        skills = store.skills()
        self.assertEqual([s.title for s in skills], ["Backup"])
        self.assertEqual(skills[0].description, "")

    def test_save_skill_tool_validates(self):
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "save_skill", {"name": "!!!", "steps": "x"})
        self.assertIn("/tidy-up", T.run_tool(T.Context(), "save_skill", {"name": "Tidy up", "steps": "1. a"}))

    def test_ambiguous_skill_isnt_guessed(self):
        store.save_skill("Backup photos", "1. a")
        store.save_skill("Backup music", "1. b")
        self.assertIsNone(store.find_skill("backup"))
        self.assertEqual(store.find_skill("backup music").slug, "backup-music")


class HistoryTest(DataDir):
    def test_save_list_search_delete(self):
        msgs = [{"role": "user", "content": [{"type": "text", "text": "What is this?"},
                                             {"type": "image_url", "image_url": {"url": IMG}}]},
                {"role": "assistant", "content": "A red panda."}]
        store.save_chat("a-1", msgs, "m")
        store.save_chat("b-2", [{"role": "user", "content": "Weather in Kraków"}], "m")
        store.save_chat("c-3", [{"role": "system", "content": "no user turn"}], "m")
        chats = store.list_chats()
        self.assertEqual([c.id for c in chats], ["b-2", "a-1"])
        self.assertEqual([c.id for c in store.list_chats("panda")], ["a-1"])
        saved = store.load_chat("a-1")
        self.assertEqual(saved["messages"][0]["content"], "What is this?\n" + IMAGE_MARKER)
        self.assertNotIn("base64", (store.chats_dir() / "a-1.json").read_text())
        self.assertIsNone(store.load_chat("../../etc/passwd"))
        store.delete_chat("a-1")
        self.assertEqual([c.id for c in store.list_chats()], ["b-2"])

    def test_titles(self):
        self.assertEqual(store.chat_title([{"role": "user", "content": "x" * 100}]), "x" * 57 + "…")
        prompt = store.skill_prompt(store.Skill("m", "Morning", "", "1. a", Path("/")))
        self.assertEqual(store.chat_title([{"role": "user", "content": prompt}]), "Skill: Morning")


class Recorder(Callbacks):
    def __init__(self):
        super().__init__()
        self.log = []
        self.notice = lambda s: self.log.append(("notice", s))
        self.handoff = lambda m: self.log.append(("handoff", m))


class AgentNotesAndImagesTest(DataDir):
    def setUp(self):
        super().setUp()
        self.flm = FakeFlm()
        self.addCleanup(self.flm.close)
        self.agent = Agent(FlmClient(self.flm.url), "eyes", machine="About this computer: test")

    def test_memory_and_skills_reach_the_prompt(self):
        store.remember("Prefers metric units")
        store.save_skill("Morning", "1. a", "Start the day")
        self.flm.replies = [(200, text_reply("Hi"))]
        self.agent.ask("hi", Recorder())
        system = self.flm.requests[0]["messages"][0]["content"]
        self.assertIn("- Prefers metric units", system)
        self.assertIn("- Morning: Start the day", system)
        names = {t["function"]["name"] for t in self.flm.requests[0]["tools"]}
        self.assertIn("remember", names)
        self.assertNotIn("look_at_screen", names)  # "eyes" isn't known to see

    def test_memory_off(self):
        store.remember("Secret")
        self.agent.memory_enabled = False
        self.flm.replies = [(200, text_reply("Hi"))]
        self.agent.ask("hi", Recorder())
        self.assertNotIn("Secret", self.flm.requests[0]["messages"][0]["content"])
        self.assertNotIn("remember", {t["function"]["name"] for t in self.flm.requests[0]["tools"]})

    def test_images_go_to_a_seeing_model_and_are_dropped_later(self):
        self.agent.sees = lambda m: True
        self.flm.replies = [(200, text_reply("A panda.")), (200, text_reply("Yes."))]
        self.agent.ask("what is it", Recorder(), [IMG])
        content = self.flm.requests[0]["messages"][-1]["content"]
        self.assertEqual(content[1], {"type": "image_url", "image_url": {"url": IMG}})
        self.agent.ask("sure?", Recorder())
        self.assertEqual(self.flm.requests[1]["messages"][1]["content"], "what is it\n" + IMAGE_MARKER)

    def test_blind_model_hands_images_over(self):
        self.agent.vision_model = "seer"
        self.flm.replies = [(200, text_reply("A cat."))]
        rec = Recorder()
        self.agent.ask("what is it", rec, [IMG])
        self.assertIn(("handoff", "seer"), rec.log)
        self.assertEqual(self.flm.requests[0]["model"], "seer")
        self.assertEqual(self.agent.messages[-1], {"role": "assistant", "content": "A cat."})
        self.assertEqual(self.agent.messages[-2]["content"], "what is it\n" + IMAGE_MARKER)

    def test_blind_model_without_helper_says_so(self):
        self.flm.replies = [(200, text_reply("I can't see."))]
        rec = Recorder()
        self.agent.ask("what is it", rec, [IMG])
        self.assertTrue(any("can't see images" in e[1] for e in rec.log if e[0] == "notice"))
        self.assertEqual(self.flm.requests[0]["messages"][-1]["content"], "what is it")

    def test_screenshot_reaches_the_model(self):
        self.agent.sees = lambda m: True
        self.flm.replies = [(200, tool_reply("look_at_screen", "{}")), (200, text_reply("A terminal."))]
        with mock.patch.object(T, "take_screenshot", return_value=Path("/nonexistent/shot.png")), \
                mock.patch.object(T, "image_data_url", return_value=IMG):
            self.agent.ask("what's on my screen", Recorder())
        last = self.flm.requests[1]["messages"][-1]
        self.assertEqual(last["role"], "user")
        self.assertEqual(last["content"][1]["image_url"]["url"], IMG)


class PickTest(unittest.TestCase):
    def test_pick(self):
        names = ["Wiertless's AirPods Pro", "MX Master 3", "JBL Flip 5"]
        self.assertEqual(T.pick("my airpods", names, "device"), "Wiertless's AirPods Pro")
        self.assertEqual(T.pick("mx master", names, "device"), "MX Master 3")
        self.assertEqual(T.pick("flip", names, "device"), "JBL Flip 5")
        with self.assertRaises(T.ToolError):
            T.pick("headphones", names, "device")
        with self.assertRaises(T.ToolError) as e:
            T.pick("home", ["Home 5G", "Home 2.4"], "network")
        self.assertIn("more than one", str(e.exception))

    def test_vision_pick(self):
        models = [Model("gemma3:4b", True, ["vision"]), Model("qwen3vl-it:4b", True, ["vision", "tool-calling"]),
                  Model("gemma3:1b", True, []), Model("qwen3.8-mtp:27b", True, ["vision", "tool-calling"])]
        self.assertEqual(pick_vision_model(models, "gemma3:1b", ram_gib=16), "qwen3vl-it:4b")
        self.assertIsNone(pick_vision_model(models[2:3], "gemma3:1b", ram_gib=16))


def fake_bin(folder: Path, name: str, script: str) -> None:
    path = folder / name
    path.write_text("#!/bin/sh\n" + textwrap.dedent(script))
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


class NetworkTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        bin_dir = Path(self.tmp.name)
        self.log = bin_dir / "log"
        fake_bin(bin_dir, "nmcli", f"""
            echo "$@" >> {self.log}
            case "$*" in
              *"connection show"*) printf 'Home\\\\:5G:802-11-wireless:wlp1s0\\nCafe:802-11-wireless:\\nwork-vpn:vpn:\\nlo:loopback:lo\\n' ;;
              *"wifi list"*) printf 'Cafe\\nNeighbour\\n' ;;
              *"connection up"*) echo ok ;;
            esac
        """)
        patcher = mock.patch.dict(os.environ, {"PATH": f"{bin_dir}:{os.environ['PATH']}"})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_list_and_connect(self):
        out = T.run_tool(T.Context(), "list_networks", {})
        self.assertIn("Home:5G (Wi-Fi, connected)", out)
        self.assertIn("Cafe (Wi-Fi, in range)", out)
        self.assertIn("work-vpn (VPN)", out)
        self.assertNotIn("lo", out.split())
        self.assertEqual(T.run_tool(T.Context(), "connect_network", {"name": "cafe"}), "Connected to Cafe.")
        self.assertIn("connection up id Cafe", self.log.read_text())
        self.assertIn("Already", T.run_tool(T.Context(), "connect_network", {"name": "home"}))
        self.assertIn("Connected to work-vpn", T.run_tool(T.Context(), "connect_network", {"name": "work vpn"}))


class BluetoothTest(unittest.TestCase):
    def objects(self, powered=True, connected=False):
        return {"/org/bluez/hci0": {"org.bluez.Adapter1": {"Powered": powered}},
                "/org/bluez/hci0/dev_1": {"org.bluez.Device1": {"Alias": "Wiertless's AirPods", "Paired": True,
                                                                "Connected": connected}},
                "/org/bluez/hci0/dev_2": {"org.bluez.Device1": {"Alias": "Stranger TV", "Paired": False}}}

    def test_connect_airpods(self):
        calls = []
        with mock.patch.object(T, "_bluez_objects", return_value=self.objects()), \
                mock.patch.object(T, "_bt_call", side_effect=lambda p, m: calls.append((p, m))):
            self.assertEqual(T.run_tool(T.Context(), "list_bluetooth", {}), "Wiertless's AirPods (not connected)")
            self.assertEqual(T.run_tool(T.Context(), "connect_bluetooth", {"device": "my airpods"}),
                             "Connected Wiertless's AirPods.")
        self.assertEqual(calls, [("/org/bluez/hci0/dev_1", "Connect")])

    def test_turns_bluetooth_on_first(self):
        states = [self.objects(powered=False), self.objects(powered=True)]
        with mock.patch.object(T, "_bluez_objects", side_effect=lambda: states.pop(0) if len(states) > 1
                               else states[0]), \
                mock.patch.object(T, "set_bluetooth") as power, mock.patch.object(T, "_bt_call"):
            T.run_tool(T.Context(), "connect_bluetooth", {"device": "airpods"})
        power.assert_called_once()

    def test_unpaired_devices_arent_offered(self):
        with mock.patch.object(T, "_bluez_objects", return_value=self.objects()):
            with self.assertRaises(T.ToolError):
                T.run_tool(T.Context(), "connect_bluetooth", {"device": "tv"})


class CommandTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.home = root / "home"
        (self.home / "proj").mkdir(parents=True)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        self.log = root / "log"
        fake_bin(bin_dir, "podman", "exit 0\n")
        fake_bin(bin_dir, "toolbox", f'echo "$PWD|$@" >> {self.log}\necho built; echo warn >&2; exit 3\n')
        patcher = mock.patch.dict(os.environ, {"PATH": f"{bin_dir}:{os.environ['PATH']}", "HOME": str(self.home)})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_runs_in_the_zed_toolbox(self):
        out = T.run_tool(T.Context(), "run_command", {"command": "make && ls", "folder": "proj"})
        self.assertEqual(out, "Exit code 3.\nbuilt\nwarn")
        self.assertEqual(self.log.read_text().strip(),
                         f"{self.home}/proj|run --container zed bash -lc make && ls")
        self.assertEqual(T.BY_NAME["run_command"].risk, T.CONFIRM)
        self.assertIn("make && ls", T.detail("run_command", {"command": "make && ls"}))

    def test_refuses_outside_home(self):
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "run_command", {"command": "ls", "folder": "/etc"})
        with self.assertRaises(T.ToolError):
            T.run_tool(T.Context(), "run_command", {"command": "  "})


if __name__ == "__main__":
    unittest.main()
