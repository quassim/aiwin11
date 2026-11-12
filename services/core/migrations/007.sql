-- Model manifests (ROADMAP R3, [S1 §4.2]): what is known about each local model and whether
-- it was verified. Only public model identifiers, status codes, numbers and timestamps.
CREATE TABLE models (
    id TEXT PRIMARY KEY,                -- "<provider>:<model identifier>"
    provider TEXT NOT NULL,             -- "ollama"
    name TEXT NOT NULL,                 -- public model identifier, as listed by the provider
    tool_calling TEXT NOT NULL,         -- unverified | not_advertised | verified | failed
    vision TEXT NOT NULL,               -- advertised | not_advertised
    probe_ms INTEGER,                   -- duration of the last tool-call probe
    probed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

PRAGMA user_version = 7;
