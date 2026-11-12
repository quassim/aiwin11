-- Model manifest measurements (ROADMAP R3, [S1 §4.2]): generation speed measured during the
-- tool-call probe and GPU memory used once loaded (from Ollama /api/ps). Numbers only.
ALTER TABLE models ADD COLUMN tokens_per_second INTEGER;
ALTER TABLE models ADD COLUMN vram_bytes INTEGER;

PRAGMA user_version = 9;
