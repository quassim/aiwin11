import json
from pathlib import Path
import subprocess
import sys
import unittest


SCRIPT = Path(__file__).with_name("agent.py")
NOTEPAD = {"function": {"name": "launch_notepad", "arguments": {}}}


class AgentProtocolTest(unittest.TestCase):
    def invoke(self, message):
        return subprocess.run(
            [sys.executable, "-I", "-S", "-B", str(SCRIPT)],
            input=json.dumps(message),
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )

    def test_one_parameterless_notepad_call_returns_typed_operation(self):
        result = self.invoke({"version": 1, "calls": [NOTEPAD]})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            json.loads(result.stdout), {"version": 1, "operation": "launch_notepad"}
        )

    def test_rejected_proposals_never_return_an_operation(self):
        proposals = [
            {"version": 2, "calls": [NOTEPAD]},
            {"version": 1, "calls": [NOTEPAD, NOTEPAD]},
            {"version": 1, "calls": [{"function": {"name": "other", "arguments": {}}}]},
            {
                "version": 1,
                "calls": [
                    {"function": {"name": "launch_notepad", "arguments": {"path": "other.exe"}}}
                ],
            },
        ]
        for proposal in proposals:
            with self.subTest(proposal=proposal):
                result = self.invoke(proposal)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
