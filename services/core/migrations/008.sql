-- Model manifest limits and hardware (ROADMAP R3, [S1 §4.2]), as reported by Ollama:
-- context window, parameter count label, quantization label and size on disk.
ALTER TABLE models ADD COLUMN context_length INTEGER;
ALTER TABLE models ADD COLUMN parameter_size TEXT;   -- e.g. "8.2B"
ALTER TABLE models ADD COLUMN quantization TEXT;     -- e.g. "Q4_K_M"
ALTER TABLE models ADD COLUMN size_bytes INTEGER;

PRAGMA user_version = 8;
