import json
import subprocess
import sys
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("agent.py")


def call(name, arguments):
    return {"function": {"name": name, "arguments": arguments}}


NOTEPAD = call("launch_application", {"application": "notepad"})


class AgentProtocolTest(unittest.TestCase):
    def invoke(self, message):
        return subprocess.run(
            [sys.executable, "-I", "-S", "-B", str(SCRIPT)],
            input=json.dumps(message),
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=5,
            check=False,
        )

    def assert_rejected(self, message):
        result = self.invoke(message)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")

    def test_registered_tool_call_returns_typed_request(self):
        result = self.invoke({"version": 2, "proposal": {"calls": [NOTEPAD]}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            json.loads(result.stdout),
            {
                "version": 2,
                "action": "tool",
                "tool": "launch_application",
                "arguments": {"application": "notepad"},
            },
        )

    def test_file_read_tools_are_registered_with_validated_names(self):
        for proposal, arguments in [
            (call("list_text_files", {}), {}),
            (call("read_text_file", {"name": "Notes 1.TXT"}), {"name": "Notes 1.TXT"}),
        ]:
            with self.subTest(proposal=proposal):
                result = self.invoke({"version": 2, "proposal": {"calls": [proposal]}})
                self.assertEqual(result.returncode, 0)
                self.assertEqual(json.loads(result.stdout)["arguments"], arguments)
        for name in [
            "../x.txt",
            "C:\\x.txt",
            "x.txt:stream",
            "dir/x.txt",
            "NUL.txt",
            "COM1.txt",
            "x.txt ",
            "x..txt",
            "x.exe",
            "é.txt",
            "a" * 125 + ".txt",
            42,
            None,
        ]:
            with self.subTest(name=name):
                self.assert_rejected({"version": 2, "proposal": {"calls": [call("read_text_file", {"name": name})]}})
        self.assert_rejected({"version": 2, "proposal": {"calls": [call("list_text_files", {"path": "C:\\"})]}})

    def test_move_and_trash_tools_validate_names(self):
        for proposal in [
            call("move_text_file", {"source": "a.txt", "destination": "b.txt"}),
            call("trash_text_file", {"name": "a.txt"}),
        ]:
            with self.subTest(proposal=proposal):
                self.assertEqual(self.invoke({"version": 2, "proposal": {"calls": [proposal]}}).returncode, 0)
        for proposal in [
            call("move_text_file", {"source": "a.txt", "destination": "A.txt"}),
            call("move_text_file", {"source": "a.txt", "destination": "..\\b.txt"}),
            call("trash_text_file", {"name": "*.txt"}),
            call("trash_text_file", {"name": "a.txt", "permanent": True}),
            call("delete_text_file", {"name": "a.txt"}),
        ]:
            with self.subTest(proposal=proposal):
                self.assert_rejected({"version": 2, "proposal": {"calls": [proposal]}})

    def test_typed_text_is_bounded_and_has_no_target_argument(self):
        result = self.invoke({"version": 2, "proposal": {"calls": [call("type_in_notepad", {"text": "Pain\nLait"})]}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["arguments"], {"text": "Pain\nLait"})
        for arguments in [
            {"text": ""},
            {"text": "a" * 2001},
            {"text": "a\x00b"},
            {"text": 3},
            {"text": "ok", "process_id": 1234},
            {"text": "ok", "window": 1},
        ]:
            with self.subTest(arguments=arguments):
                self.assert_rejected({"version": 2, "proposal": {"calls": [call("type_in_notepad", arguments)]}})

    def test_window_inspection_takes_no_target_from_the_model(self):
        result = self.invoke({"version": 2, "proposal": {"calls": [call("inspect_window", {})]}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["tool"], "inspect_window")
        for tool in [
            "inspect_window",
            "describe_window",
            "list_processes",
            "get_system_status",
            "read_clipboard",
            "list_services",
        ]:
            result = self.invoke({"version": 2, "proposal": {"calls": [call(tool, {})]}})
            self.assertEqual(result.returncode, 0)
            for arguments in [{"window": 123}, {"title": "Banque"}, {"process_id": 4}, {"limit": 500}]:
                with self.subTest(tool=tool, arguments=arguments):
                    self.assert_rejected({"version": 2, "proposal": {"calls": [call(tool, arguments)]}})

    def test_window_arrangement_takes_a_listed_number_and_a_known_state(self):
        result = self.invoke(
            {"version": 2, "proposal": {"calls": [call("arrange_window", {"window": 3, "arrangement": "minimize"})]}}
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["arguments"], {"window": 3, "arrangement": "minimize"})
        for arguments in [
            {"window": 0, "arrangement": "focus"},
            {"window": 21, "arrangement": "focus"},
            {"window": True, "arrangement": "focus"},
            {"window": "1", "arrangement": "focus"},
            {"window": 1, "arrangement": "close"},
            {"window": 1},
            {"window": 1, "arrangement": "focus", "process_id": 4},
        ]:
            with self.subTest(arguments=arguments):
                self.assert_rejected({"version": 2, "proposal": {"calls": [call("arrange_window", arguments)]}})

    def test_copy_tool_requires_two_distinct_valid_names(self):
        copy = call("copy_text_file", {"source": "a.txt", "destination": "b copie.txt"})
        result = self.invoke({"version": 2, "proposal": {"calls": [copy]}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["arguments"], {"source": "a.txt", "destination": "b copie.txt"})
        for arguments in [
            {"source": "a.txt"},
            {"source": "a.txt", "destination": "A.TXT"},
            {"source": "a.txt", "destination": "../b.txt"},
            {"source": "a.exe", "destination": "b.txt"},
            {"source": "a.txt", "destination": "b.txt", "overwrite": True},
        ]:
            with self.subTest(arguments=arguments):
                self.assert_rejected({"version": 2, "proposal": {"calls": [call("copy_text_file", arguments)]}})

    def test_text_proposal_returns_trimmed_reply(self):
        result = self.invoke({"version": 2, "proposal": {"text": "  Bonjour, été ! \n"}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout), {"version": 2, "action": "reply", "text": "Bonjour, été !"})

    def test_rejected_tool_proposals_never_return_an_operation(self):
        proposals = [
            {"version": 1, "proposal": {"calls": [NOTEPAD]}},
            {"version": 2, "proposal": {"calls": [NOTEPAD, NOTEPAD]}},
            {"version": 2, "proposal": {"calls": []}},
            {"version": 2, "proposal": {"calls": [call("launch_notepad", {})]}},
            {"version": 2, "proposal": {"calls": [call("shell", {"command": "dir"})]}},
            {"version": 2, "proposal": {"calls": [call("launch_application", {})]}},
            {"version": 2, "proposal": {"calls": [call("launch_application", {"application": "cmd"})]}},
            {"version": 2, "proposal": {"calls": [call("launch_application", {"application": ["notepad"]})]}},
            {
                "version": 2,
                "proposal": {"calls": [call("launch_application", {"application": "notepad", "path": "x.exe"})]},
            },
            {"version": 2, "proposal": {"calls": [NOTEPAD], "text": "ok"}},
            {"version": 2, "proposal": {}},
        ]
        for proposal in proposals:
            with self.subTest(proposal=proposal):
                self.assert_rejected(proposal)

    def test_rejected_replies(self):
        for text in ["", "   ", "a" * 4001, 42, None, "nul\x00byte"]:
            with self.subTest(text=text):
                self.assert_rejected({"version": 2, "proposal": {"text": text}})

    def test_oversized_input_is_rejected(self):
        self.assert_rejected({"version": 2, "proposal": {"text": "a" * (40 * 1024)}})


if __name__ == "__main__":
    unittest.main()
