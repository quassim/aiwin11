-- Daily schedule of saved requests (ROADMAP R8): local time "HH:MM" and the last local day run.
ALTER TABLE memory_entries ADD COLUMN schedule TEXT;
ALTER TABLE memory_entries ADD COLUMN last_run TEXT;

PRAGMA user_version = 6;
