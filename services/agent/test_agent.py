import json
from pathlib import Path
import subprocess
import sys
import unittest


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

    def test_text_proposal_returns_trimmed_reply(self):
        result = self.invoke({"version": 2, "proposal": {"text": "  Bonjour, été ! \n"}})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            json.loads(result.stdout), {"version": 2, "action": "reply", "text": "Bonjour, été !"}
        )

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
                "proposal": {
                    "calls": [call("launch_application", {"application": "notepad", "path": "x.exe"})]
                },
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
