-- Durable memory the user asked the agent to keep (ROADMAP R6). The content is encrypted with
-- Windows DPAPI for the current user (ROADMAP §6 rule 2): SQLite only holds ciphertext.
CREATE TABLE memory_entries (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    content BLOB NOT NULL
);

PRAGMA user_version = 4;
