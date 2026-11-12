-- Temporary, revocable permissions given by the user: "allow this action without asking
-- for N minutes". Only action codes and timestamps: no resource, no user content.
CREATE TABLE grants (
    id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);

CREATE INDEX grants_by_action ON grants (action, expires_at);

PRAGMA user_version = 3;
