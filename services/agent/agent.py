"""Convert a single model proposal to a typed request for the trusted host."""
import json
import sys

MAX_BYTES = 8192


def plan(message):
    if not isinstance(message, dict) or message.get("version") != 1:
        raise ValueError("invalid protocol")
    calls = message.get("calls")
    if not isinstance(calls, list) or len(calls) != 1:
        raise ValueError("expected one tool call")
    call = calls[0]
    function = call.get("function") if isinstance(call, dict) else None
    if not isinstance(function, dict):
        raise ValueError("invalid function")
    if function.get("name") != "launch_notepad" or function.get("arguments") != {}:
        raise ValueError("invalid tool or arguments")
    return {"version": 1, "operation": "launch_notepad"}


def main():
    try:
        data = sys.stdin.buffer.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES:
            return 2
        print(json.dumps(plan(json.loads(data)), separators=(",", ":")))
        return 0
    except (ValueError, RecursionError):
        return 2


if __name__ == "__main__":
    sys.exit(main())
