-- User settings (policy profile). Secrets never go here: they stay in Windows Credential Manager.
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- One row per chat task, kept up to date at each lifecycle step so that a task interrupted
-- by a crash can be recognised and closed consistently at the next start.
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    state TEXT NOT NULL,
    outcome TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX tasks_by_state ON tasks (state);

PRAGMA user_version = 2;
