"""Turn one model proposal into one typed agent decision for the trusted host.

The agent never executes anything. It validates the model output against the
tool registry below and returns either a typed tool request or a plain reply.
The host validates the decision again and submits tool requests to the broker.
"""

import json
import sys
from collections.abc import Callable
from typing import Any

VERSION = 2
MAX_INPUT_BYTES = 32 * 1024
MAX_REPLY_CHARS = 4000

MAX_FILE_NAME = 128
_NAME_CHARS = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -_")
_ALIAS_CHARS = frozenset("abcdefghijklmnopqrstuvwxyz0123456789-")
_RESERVED = {"CON", "PRN", "AUX", "NUL"}


def is_text_file_name(value: object) -> bool:
    """Text file name as accepted by the host: `name.txt`, or `alias/name.txt` for an extra folder."""
    if isinstance(value, str) and value.count("/") == 1:
        alias, value = value.split("/")
        if not (0 < len(alias) <= 24 and all(char in _ALIAS_CHARS for char in alias)):
            return False
    if not isinstance(value, str) or len(value) > MAX_FILE_NAME or "." not in value:
        return False
    stem, extension = value.rsplit(".", 1)
    upper = stem.upper()
    reserved = upper in _RESERVED or (len(upper) == 4 and upper[:3] in ("COM", "LPT") and upper[3].isdigit())
    return (
        extension.lower() in ("txt", "md", "csv")
        and stem != ""
        and stem == stem.strip()
        and not reserved
        and all(char in _NAME_CHARS for char in stem)
    )


MAX_TYPED_CHARS = 2000


def is_typed_text(value: object) -> bool:
    """Text for Notepad: 1 to 2000 characters, no NUL."""
    return isinstance(value, str) and 0 < len(value) <= MAX_TYPED_CHARS and "\x00" not in value


MAX_SEARCH_CHARS = 64


def is_search_query(value: object) -> bool:
    """Search text: 2 to 64 characters once trimmed, no control character."""
    return (
        isinstance(value, str)
        and 2 <= len(value.strip()) <= MAX_SEARCH_CHARS
        and not any(ord(char) < 32 or 127 <= ord(char) < 160 for char in value)
    )


MAX_LISTED_WINDOWS = 20


def is_stoppable_process_id(value: object) -> bool:
    """A process ID above the reserved system ones (0 to 4)."""
    return is_process_id(value) and isinstance(value, int) and value > 4


def is_process_id(value: object) -> bool:
    """A real process ID: a positive 32-bit integer (booleans excluded)."""
    return isinstance(value, int) and not isinstance(value, bool) and 0 < value <= 0xFFFFFFFF


def is_window_number(value: object) -> bool:
    """Number of a window in the task's last list_windows result: 1 to 20."""
    return isinstance(value, int) and not isinstance(value, bool) and 1 <= value <= MAX_LISTED_WINDOWS


MAX_NOTE_CHARS = 1000


def is_image_name(value: object) -> bool:
    """Name of an image AiWin11 generated: image-<32 hex>.png."""
    if not isinstance(value, str) or not value.startswith("image-") or not value.endswith(".png"):
        return False
    digest = value[len("image-") : -len(".png")]
    return len(digest) == 32 and all(char in "0123456789abcdefABCDEF" for char in digest)


def is_note(value: object) -> bool:
    """Memory note: 1 to 1000 characters, not blank, no NUL."""
    return isinstance(value, str) and value.strip() != "" and len(value) <= MAX_NOTE_CHARS and "\x00" not in value


def is_control_number(value: object) -> bool:
    """Number [n] of a control in the task's last inspect_window result: 1 to 80."""
    return isinstance(value, int) and not isinstance(value, bool) and 1 <= value <= 80


def is_service_name(value: object) -> bool:
    """Windows service key name: 1 to 64 ASCII letters, digits, '_', '.' or '-'."""
    return (
        isinstance(value, str)
        and 0 < len(value) <= 64
        and all(char.isascii() and (char.isalnum() or char in "_.-") for char in value)
    )


MAX_PLAN_MOVES = 20


def is_move_plan(value: object) -> bool:
    """1 to 20 renames with valid, distinct names; no name is both a source and a destination."""
    if not isinstance(value, list) or not 0 < len(value) <= MAX_PLAN_MOVES:
        return False
    sources: set[str] = set()
    destinations: set[str] = set()
    for move in value:
        if not isinstance(move, dict) or set(move) != {"source", "destination"}:
            return False
        source, destination = move["source"], move["destination"]
        if not (is_text_file_name(source) and is_text_file_name(destination)):
            return False
        sources.add(source.lower())
        destinations.add(destination.lower())
    return len(sources) == len(value) and len(destinations) == len(value) and not sources & destinations


def is_mcp_server(value: object) -> bool:
    """Declared MCP server name: 1 to 32 ASCII letters, digits, '_' or '-'."""
    return (
        isinstance(value, str)
        and 0 < len(value) <= 32
        and all(char.isascii() and (char.isalnum() or char in "_-") for char in value)
    )


def is_mcp_tool(value: object) -> bool:
    """MCP tool name: 1 to 64 ASCII letters, digits, '_', '-', '.' or '/'."""
    return (
        isinstance(value, str)
        and 0 < len(value) <= 64
        and all(char.isascii() and (char.isalnum() or char in "_-./") for char in value)
    )


def is_mcp_arguments(value: object) -> bool:
    """MCP tool arguments: a JSON object of at most 4000 characters once serialized."""
    return isinstance(value, dict) and len(json.dumps(value)) <= 4000


# Tool name -> argument name -> allowed values (tuple) or validator (callable).
# Every declared argument is required and no other argument is accepted.
Rule = tuple[str, ...] | Callable[[object], bool]
TOOLS: dict[str, dict[str, Rule]] = {
    "launch_application": {"application": ("notepad", "charmap", "paint")},
    "list_text_files": {},
    "search_text_files": {"query": is_search_query},
    "read_text_file": {"name": is_text_file_name},
    "copy_text_file": {"source": is_text_file_name, "destination": is_text_file_name},
    "move_text_file": {"source": is_text_file_name, "destination": is_text_file_name},
    "trash_text_file": {"name": is_text_file_name},
    "inspect_window": {},
    "describe_window": {},
    "describe_screen": {},
    "type_in_notepad": {"text": is_typed_text},
    "list_processes": {},
    "get_system_status": {},
    "read_clipboard": {},
    "write_clipboard": {"text": is_typed_text},
    "list_windows": {},
    "list_services": {},
    "organize_files": {"moves": is_move_plan},
    "mcp_list_tools": {"server": is_mcp_server},
    "generate_image": {"prompt": is_note, "format": ("square", "landscape", "portrait")},
    "describe_image": {"name": is_image_name},
    "delegate": {
        "agent": ("office", "windows_admin", "development", "research", "multimedia", "read_only"),
        "request": is_note,
    },
    "mcp_call": {"server": is_mcp_server, "tool": is_mcp_tool, "arguments": is_mcp_arguments},
    "run_diagnostic": {"diagnostic": ("network", "wifi", "time")},
    "run_powershell_diagnostic": {},
    "admin_service": {
        "service": is_service_name,
        "operation": ("start", "stop", "restart", "automatic", "manual", "disabled"),
    },
    "admin_clean_temp": {},
    "find_control": {"query": is_search_query},
    "inspect_process": {"process_id": is_process_id},
    "list_applications": {},
    "stop_process": {"process_id": is_stoppable_process_id},
    "click_control": {"control": is_control_number},
    "remember": {"text": is_note},
    "recall": {},
    "search_memory": {"query": is_note},
    "set_control_value": {"control": is_control_number, "text": is_typed_text},
    "arrange_window": {
        "window": is_window_number,
        "arrangement": ("focus", "minimize", "restore", "maximize", "snap_left", "snap_right", "center"),
    },
}

# Arguments a tool may leave out (the host applies its default).
OPTIONAL: dict[str, set[str]] = {"generate_image": {"format"}}

# Tools whose two file names must differ (ignoring case).
TRANSFERS = {"copy_text_file", "move_text_file"}


def _tool_request(calls: object) -> dict[str, Any]:
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
    arguments: object = function.get("arguments")
    optional = OPTIONAL.get(str(name), set())
    if not isinstance(arguments, dict) or not set(schema) - optional <= set(arguments) <= set(schema):
        raise ValueError("invalid arguments")
    for key, rule in schema.items():
        if key not in arguments:
            continue
        value = arguments[key]
        valid = rule(value) if callable(rule) else isinstance(value, str) and value in rule
        if not valid:
            raise ValueError("argument outside schema")
    if name in TRANSFERS and arguments["source"].lower() == arguments["destination"].lower():
        raise ValueError("transfer onto itself")
    return {"version": VERSION, "action": "tool", "tool": name, "arguments": dict(arguments)}


def _reply(text: object) -> dict[str, Any]:
    if not isinstance(text, str):
        raise ValueError("invalid reply")
    text = text.strip()
    if not text or len(text) > MAX_REPLY_CHARS or "\x00" in text:
        raise ValueError("invalid reply")
    return {"version": VERSION, "action": "reply", "text": text}


def decide(message: object) -> dict[str, Any]:
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


def main() -> int:
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
