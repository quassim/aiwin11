-- Saved requests ("routines", first step of ROADMAP R8) share the encrypted memory table.
ALTER TABLE memory_entries ADD COLUMN kind TEXT NOT NULL DEFAULT 'note';

PRAGMA user_version = 5;
