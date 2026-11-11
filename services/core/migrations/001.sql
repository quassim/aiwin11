CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    task_id TEXT,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    code TEXT,
    timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX events_by_session ON events (session_id, sequence);

CREATE TABLE tool_executions (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    idempotency_key TEXT NOT NULL,
    task_id TEXT NOT NULL,
    PRIMARY KEY (session_id, idempotency_key)
);

PRAGMA user_version = 1;
