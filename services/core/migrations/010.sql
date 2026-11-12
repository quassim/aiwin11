-- Vector index for semantic memory search (ROADMAP R6): one vector per memory entry and local
-- embedding model, encrypted with DPAPI like the entries themselves. A vector goes with its
-- entry, and is dropped as soon as the entry's content changes (a file indexed again).
CREATE TABLE memory_vectors (
    entry_id TEXT NOT NULL REFERENCES memory_entries (id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (entry_id, model)
);

CREATE TRIGGER memory_vectors_stale AFTER UPDATE OF content ON memory_entries
BEGIN
    DELETE FROM memory_vectors WHERE entry_id = OLD.id;
END;

PRAGMA user_version = 10;
