"""Turn one model proposal into one typed agent decision for the trusted host.

The agent never executes anything. It validates the model output against the
tool registry below and returns either a typed tool request or a plain reply.
The host validates the decision again and submits tool requests to the broker.
"""
import json
import sys

VERSION = 2
MAX_INPUT_BYTES = 32 * 1024
MAX_REPLY_CHARS = 4000

MAX_FILE_NAME = 128
_NAME_CHARS = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -_")
_RESERVED = {"CON", "PRN", "AUX", "NUL"}


def is_text_file_name(value):
    """Flat ASCII `.txt` name, as accepted by the host's local file store."""
    if not isinstance(value, str) or len(value) > MAX_FILE_NAME or "." not in value:
        return False
    stem, extension = value.rsplit(".", 1)
    upper = stem.upper()
    reserved = upper in _RESERVED or (
        len(upper) == 4 and upper[:3] in ("COM", "LPT") and upper[3].isdigit()
    )
    return (
        extension.lower() == "txt"
        and stem != ""
        and stem == stem.strip()
        and not reserved
        and all(char in _NAME_CHARS for char in stem)
    )


# Tool name -> argument name -> allowed values (tuple) or validator (callable).
# Every declared argument is required and no other argument is accepted.
TOOLS = {
    "launch_application": {"application": ("notepad",)},
    "list_text_files": {},
    "read_text_file": {"name": is_text_file_name},
    "copy_text_file": {"source": is_text_file_name, "destination": is_text_file_name},
    "inspect_window": {},
}


def _tool_request(calls):
    if not isinstance(calls, list) or len(calls) != 1:
        raise ValueError("expected one tool call")
    call = calls[0]
    function = call.get("function") if isinstance(call, dict) else None
    if not isinstance(function, dict):
        raise ValueError("invalid function")
    name = function.get("name")
    schema = TOOLS.get(name) if isinstance(name, str) else None
    if schema is None:
        raise ValueError("unknown tool")
    arguments = function.get("arguments")
    if not isinstance(arguments, dict) or set(arguments) != set(schema):
        raise ValueError("invalid arguments")
    for key, rule in schema.items():
        value = arguments[key]
        valid = rule(value) if callable(rule) else isinstance(value, str) and value in rule
        if not valid:
            raise ValueError("argument outside schema")
    if name == "copy_text_file" and arguments["source"].lower() == arguments["destination"].lower():
        raise ValueError("copy onto itself")
    return {"version": VERSION, "action": "tool", "tool": name, "arguments": dict(arguments)}


def _reply(text):
    if not isinstance(text, str):
        raise ValueError("invalid reply")
    text = text.strip()
    if not text or len(text) > MAX_REPLY_CHARS or "\x00" in text:
        raise ValueError("invalid reply")
    return {"version": VERSION, "action": "reply", "text": text}


def decide(message):
    if not isinstance(message, dict) or message.get("version") != VERSION:
        raise ValueError("invalid protocol")
    proposal = message.get("proposal")
    if not isinstance(proposal, dict) or len(proposal) != 1:
        raise ValueError("invalid proposal")
    if "calls" in proposal:
        return _tool_request(proposal["calls"])
    if "text" in proposal:
        return _reply(proposal["text"])
    raise ValueError("invalid proposal")


def main():
    try:
        data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
        if len(data) > MAX_INPUT_BYTES:
            return 2
        print(json.dumps(decide(json.loads(data)), separators=(",", ":")))
        return 0
    except (ValueError, RecursionError):
        return 2


if __name__ == "__main__":
    sys.exit(main())
