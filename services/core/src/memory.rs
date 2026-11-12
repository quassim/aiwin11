//! Durable memory (ROADMAP R6): short notes the user lets the agent keep ("souviens-toi
//! que…"). Each note is encrypted with Windows DPAPI for the current user before it reaches
//! SQLite (ROADMAP §6 rule 2); the journal only records that a note was saved or deleted.

use std::{ptr, slice};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB},
};

use crate::Core;

/// id, created_at, ciphertext, schedule.
type StoredEntry = (String, String, Vec<u8>, Option<String>);

pub const MAX_NOTE_CHARS: usize = 1000;
pub const MAX_NOTES: u32 = 200;
const MAX_INDEXED_FILES: usize = 32;
const MAX_INDEXED_CHARS: usize = 4000;
const ENTROPY: &[u8] = b"AiWin11.memory.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub created_at: String,
    pub text: String,
    /// Local time "HH:MM", optionally followed by "@12345" for selected weekdays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct IndexedFile {
    pub name: String,
    pub excerpt: String,
}

pub fn valid_schedule(value: &str) -> bool {
    value == "startup" || schedule_parts(value).is_some()
}

/// SQLite weekday digits run from Sunday (0) through Saturday (6).
fn schedule_parts(value: &str) -> Option<(&str, &str)> {
    let (time, days) = value.split_once('@').unwrap_or((value, "0123456"));
    if days.is_empty()
        || !days.bytes().all(|day| (b'0'..=b'6').contains(&day))
        || !days.as_bytes().windows(2).all(|pair| pair[0] < pair[1])
    {
        return None;
    }
    let bytes = time.as_bytes();
    (bytes.len() == 5
        && bytes[2] == b':'
        && bytes.iter().enumerate().all(|(index, byte)| index == 2 || byte.is_ascii_digit())
        && time[..2].parse::<u8>().is_ok_and(|hours| hours < 24)
        && time[3..].parse::<u8>().is_ok_and(|minutes| minutes < 60))
    .then_some((time, days))
}

fn schedule_due(value: &str, now: &str, weekday: &str) -> bool {
    schedule_parts(value).is_some_and(|(time, days)| time <= now && days.contains(weekday))
}

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr().cast_mut() }
}

/// Copies and frees a blob allocated by DPAPI.
fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with LocalAlloc.
    let bytes = unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    bytes
}

pub(crate) fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    let (input, entropy) = (blob(plain), blob(ENTROPY));
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    // SAFETY: input and entropy borrow live slices for the duration of the call.
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    (ok != 0).then(|| take(output))
}

pub(crate) fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
    let (input, entropy) = (blob(cipher), blob(ENTROPY));
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    // SAFETY: as above.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    (ok != 0).then(|| take(output))
}

pub fn valid_note(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty() && text.chars().count() <= MAX_NOTE_CHARS && !text.contains('\0')
}

/// Cosine similarity; 0 for empty or mismatched vectors.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}

/// Keyword score: share of the query's words (3 letters or more) found in the note.
fn keyword_score(query: &str, note: &str) -> f32 {
    let note = note.to_lowercase();
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return 0.0;
    }
    words.iter().filter(|word| note.contains(word.as_str())).count() as f32 / words.len() as f32
}

/// Ranks notes for a query: by local embeddings when `vectors` holds the query's vector then
/// one per note, otherwise by keywords. Returns at most `limit` notes with a positive score.
pub(crate) fn rank(
    query: &str,
    notes: Vec<MemoryEntry>,
    vectors: Option<&[Vec<f32>]>,
    limit: usize,
) -> Vec<MemoryEntry> {
    let mut scored: Vec<(f32, MemoryEntry)> = notes
        .into_iter()
        .enumerate()
        .map(|(index, note)| {
            let score = match vectors {
                Some(vectors) => cosine(&vectors[0], &vectors[index + 1]),
                None => keyword_score(query, &note.text),
            };
            (score, note)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().take(limit).map(|(_, note)| note).collect()
}

impl Core {
    pub fn index_file(&mut self, files: &crate::local_files::LocalFiles, name: &str) -> Result<(), &'static str> {
        let excerpt: String = files.preview(self, name)?.chars().take(MAX_INDEXED_CHARS).collect();
        if excerpt.trim().is_empty() {
            return Err("MEMORY_INVALID");
        }
        let document = IndexedFile { name: name.into(), excerpt };
        let json = serde_json::to_vec(&document).map_err(|_| "MEMORY_UNAVAILABLE")?;
        let cipher = protect(&json).ok_or("MEMORY_UNAVAILABLE")?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        let entries = Self::indexed_entries(&tx)?;
        let existing = entries.iter().find(|(_, file)| file.name.eq_ignore_ascii_case(name));
        if existing.is_none() && entries.len() >= MAX_INDEXED_FILES {
            return Err("MEMORY_FULL");
        }
        let id = existing.map(|(id, _)| id.clone()).unwrap_or_else(|| Uuid::new_v4().to_string());
        tx.execute(
            "INSERT INTO memory_entries (id, content, kind) VALUES (?1, ?2, 'indexed_file')
             ON CONFLICT(id) DO UPDATE SET content = excluded.content",
            params![id, cipher],
        )
        .map_err(|_| "MEMORY_UNAVAILABLE")?;
        tx.execute(
            "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'memory.file.indexed', 'core')",
            params![Uuid::new_v4().to_string(), self.session_id],
        )
        .map_err(|_| "MEMORY_UNAVAILABLE")?;
        tx.commit().map_err(|_| "MEMORY_UNAVAILABLE")
    }

    fn indexed_entries(db: &rusqlite::Connection) -> Result<Vec<(String, IndexedFile)>, &'static str> {
        let mut statement = db
            .prepare("SELECT id, content FROM memory_entries WHERE kind = 'indexed_file' ORDER BY rowid")
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, cipher) = row.map_err(|_| "MEMORY_UNAVAILABLE")?;
            if let Some(file) = unprotect(&cipher).and_then(|plain| serde_json::from_slice(&plain).ok()) {
                entries.push((id, file));
            }
        }
        Ok(entries)
    }

    pub fn indexed_file_names(&self) -> Result<Vec<String>, &'static str> {
        Ok(Self::indexed_entries(&self.db)?.into_iter().map(|(_, file)| file.name).collect())
    }

    pub fn indexed_files(&self) -> Result<Vec<IndexedFile>, &'static str> {
        Ok(Self::indexed_entries(&self.db)?.into_iter().map(|(_, file)| file).collect())
    }

    pub fn remove_indexed_file(&mut self, name: &str) -> Result<(), &'static str> {
        let (id, _) = Self::indexed_entries(&self.db)?
            .into_iter()
            .find(|(_, file)| file.name == name)
            .ok_or("MEMORY_NOT_FOUND")?;
        self.forget(Some(&id))?;
        Ok(())
    }

    /// Optional local embedding model for semantic memory search (none: keyword search).
    pub fn embedding_model(&self) -> Option<String> {
        self.setting("memory.embedding_model")
            .ok()
            .flatten()
            .filter(|model| !model.is_empty() && model.len() <= 256 && model.bytes().all(|b| b.is_ascii_graphic()))
    }

    pub fn set_embedding_model(&mut self, model: &str) -> Result<(), &'static str> {
        let model = model.trim();
        if model.len() > 256 || !model.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("SETTINGS_INVALID");
        }
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('memory.embedding_model', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [model],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.journal("settings.embedding.changed", Some(if model.is_empty() { "keywords" } else { "local" }))
            .map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Encrypts and stores one note. Returns its ID.
    pub fn remember(&mut self, text: &str) -> Result<String, &'static str> {
        self.store(text, "note")
    }

    /// Saves a request the user wants to run again later (never run automatically).
    pub fn save_routine(&mut self, text: &str) -> Result<String, &'static str> {
        self.store(text, "routine")
    }

    pub fn update_routine(&mut self, id: &str, text: &str) -> Result<(), &'static str> {
        if !valid_note(text) {
            return Err("MEMORY_INVALID");
        }
        let cipher = protect(text.trim().as_bytes()).ok_or("MEMORY_UNAVAILABLE")?;
        let transaction = self.db.transaction().map_err(|_| "MEMORY_UNAVAILABLE")?;
        let changed = transaction
            .execute("UPDATE memory_entries SET content = ?2 WHERE id = ?1 AND kind = 'routine'", params![id, cipher])
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        if changed == 0 {
            return Err("MEMORY_NOT_FOUND");
        }
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'memory.updated', 'core', 'routine')",
                params![Uuid::new_v4().to_string(), self.session_id],
            )
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "MEMORY_UNAVAILABLE")
    }

    fn store(&mut self, text: &str, kind: &'static str) -> Result<String, &'static str> {
        if !valid_note(text) {
            return Err("MEMORY_INVALID");
        }
        let count: u32 = self
            .db
            .query_row("SELECT COUNT(*) FROM memory_entries WHERE kind <> 'indexed_file'", [], |row| row.get(0))
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        if count >= MAX_NOTES {
            return Err("MEMORY_FULL");
        }
        let cipher = protect(text.trim().as_bytes()).ok_or("MEMORY_UNAVAILABLE")?;
        let id = Uuid::new_v4().to_string();
        let session = self.session_id.clone();
        let write = |db: &mut rusqlite::Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO memory_entries (id, content, kind) VALUES (?1, ?2, ?3)",
                params![id, cipher, kind],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'memory.saved', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, kind],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "MEMORY_UNAVAILABLE")?;
        Ok(id)
    }

    /// All notes, oldest first. A note that no longer decrypts (other Windows user) is skipped.
    pub fn memories(&self) -> Result<Vec<MemoryEntry>, &'static str> {
        self.entries("note")
    }

    pub fn routines(&self) -> Result<Vec<MemoryEntry>, &'static str> {
        self.entries("routine")
    }

    /// Results of scheduled requests, oldest first.
    pub fn reports(&self) -> Result<Vec<MemoryEntry>, &'static str> {
        self.entries("report")
    }

    /// Keeps the result of a scheduled request (encrypted). At most 50 are kept.
    pub fn save_report(&mut self, text: &str) -> Result<String, &'static str> {
        let text: String = text.chars().take(MAX_NOTE_CHARS).collect();
        let _ = self.db.execute(
            "DELETE FROM memory_entries WHERE kind = 'report' AND id NOT IN
             (SELECT id FROM memory_entries WHERE kind = 'report' ORDER BY created_at DESC LIMIT 49)",
            [],
        );
        self.store(if valid_note(&text) { &text } else { "(résultat vide)" }, "report")
    }

    /// Sets or removes the local time and weekdays of a saved request.
    pub fn set_routine_schedule(&mut self, id: &str, schedule: Option<&str>) -> Result<(), &'static str> {
        if schedule.is_some_and(|value| !valid_schedule(value)) {
            return Err("SCHEDULE_INVALID");
        }
        let changed = self
            .db
            .execute(
                "UPDATE memory_entries SET schedule = ?2 WHERE id = ?1 AND kind = 'routine'",
                params![id, schedule],
            )
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        if changed == 0 {
            return Err("ROUTINE_NOT_FOUND");
        }
        self.journal("routine.schedule.changed", Some(if schedule.is_some() { "scheduled" } else { "unscheduled" }))
            .map_err(|_| "MEMORY_UNAVAILABLE")
    }

    /// Saved requests due now (local time), not yet run today; marks them as run.
    pub fn take_due_routine(&mut self, startup: bool) -> Result<Option<MemoryEntry>, &'static str> {
        let (today, now, weekday): (String, String, String) = self
            .db
            .query_row(
                "SELECT strftime('%Y-%m-%d', 'now', 'localtime'), strftime('%H:%M', 'now', 'localtime'),
                        strftime('%w', 'now', 'localtime')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "MEMORY_UNAVAILABLE")?;
        let due: Vec<MemoryEntry> = self
            .entries("routine")?
            .into_iter()
            .filter(|routine| {
                routine
                    .schedule
                    .as_deref()
                    .is_some_and(|value| (startup && value == "startup") || schedule_due(value, &now, &weekday))
            })
            .collect();
        for routine in due {
            let marked = self
                .db
                .execute(
                    "UPDATE memory_entries SET last_run = ?2 WHERE id = ?1 AND (last_run IS NULL OR last_run <> ?2)",
                    params![routine.id, today],
                )
                .map_err(|_| "MEMORY_UNAVAILABLE")?;
            if marked == 1 {
                return Ok(Some(routine));
            }
        }
        Ok(None)
    }

    fn entries(&self, kind: &str) -> Result<Vec<MemoryEntry>, &'static str> {
        let read = || -> rusqlite::Result<Vec<StoredEntry>> {
            let mut statement = self.db.prepare(
                "SELECT id, created_at, content, schedule FROM memory_entries WHERE kind = ?1 ORDER BY created_at, rowid",
            )?;
            let rows = statement.query_map([kind], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
            rows.collect()
        };
        Ok(read()
            .map_err(|_| "MEMORY_UNAVAILABLE")?
            .into_iter()
            .filter_map(|(id, created_at, cipher, schedule)| {
                let text = String::from_utf8(unprotect(&cipher)?).ok()?;
                Some(MemoryEntry { id, created_at, text, schedule })
            })
            .collect())
    }

    /// Deletes one note, or every note when `id` is `None`.
    pub fn forget(&mut self, id: Option<&str>) -> Result<usize, &'static str> {
        let session = self.session_id.clone();
        let delete = |db: &mut rusqlite::Connection| -> rusqlite::Result<usize> {
            let transaction = db.transaction()?;
            let removed = match id {
                Some(id) => transaction.execute("DELETE FROM memory_entries WHERE id = ?1", [id])?,
                None => transaction.execute("DELETE FROM memory_entries WHERE kind = 'note'", [])?,
            };
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'memory.deleted', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, format!("entries:{removed}")],
            )?;
            transaction.commit()?;
            Ok(removed)
        };
        delete(&mut self.db).map_err(|_| "MEMORY_UNAVAILABLE")
    }
}

/// Ranks indexed files for a query: by local embeddings when `vectors` holds the query's vector
/// then one per file (see `semantic_vectors`), otherwise by keywords. At most 8 files.
pub fn search_indexed_files(
    query: &str,
    mut files: Vec<IndexedFile>,
    vectors: Option<&[Vec<f32>]>,
) -> Vec<IndexedFile> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_NOTE_CHARS {
        return Vec::new();
    }
    let vectors = vectors.filter(|vectors| vectors.len() == files.len() + 1);
    let mut scores: Vec<_> = files
        .drain(..)
        .enumerate()
        .filter_map(|(index, file)| {
            let score = vectors.map_or_else(
                || keyword_score(query, &format!("{} {}", file.name, file.excerpt)),
                |vectors| cosine(&vectors[0], &vectors[index + 1]),
            );
            (score > 0.0).then_some((score, file))
        })
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    scores.into_iter().take(8).map(|(_, file)| file).collect()
}

fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn bytes_vector(bytes: &[u8]) -> Option<Vec<f32>> {
    (bytes.len().is_multiple_of(4) && !bytes.is_empty()).then(|| {
        bytes.chunks_exact(4).map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])).collect()
    })
}

impl Core {
    /// Cached vectors of `ids` for `model`; `None` where missing or unreadable.
    fn cached_vectors(&self, model: &str, ids: &[&str]) -> Vec<Option<Vec<f32>>> {
        let Ok(mut statement) = self.db.prepare("SELECT vector FROM memory_vectors WHERE entry_id = ?1 AND model = ?2")
        else {
            return vec![None; ids.len()];
        };
        ids.iter()
            .map(|id| {
                let cipher: Vec<u8> = statement.query_row(params![id, model], |row| row.get(0)).ok()?;
                unprotect(&cipher).as_deref().and_then(bytes_vector)
            })
            .collect()
    }

    /// Stores vectors, encrypted. An entry deleted meanwhile is skipped.
    fn store_vectors(&mut self, model: &str, vectors: &[(&str, &[f32])]) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let tx = self.db.transaction().map_err(|_| "MEMORY_UNAVAILABLE")?;
        let mut stored = 0;
        for (id, vector) in vectors {
            let cipher = protect(&vector_bytes(vector)).ok_or("MEMORY_UNAVAILABLE")?;
            stored += tx
                .execute(
                    "INSERT INTO memory_vectors (entry_id, model, vector)
                     SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM memory_entries WHERE id = ?1)
                     ON CONFLICT (entry_id, model) DO UPDATE SET vector = excluded.vector",
                    params![id, model, cipher],
                )
                .map_err(|_| "MEMORY_UNAVAILABLE")?;
        }
        tx.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'memory.vectors.stored', 'core', ?3)",
            params![Uuid::new_v4().to_string(), session, format!("count:{stored}")],
        )
        .map_err(|_| "MEMORY_UNAVAILABLE")?;
        tx.commit().map_err(|_| "MEMORY_UNAVAILABLE")
    }

    /// Indexed files with the id of their memory entry (for the vector index).
    pub fn indexed_files_with_ids(&self) -> Result<Vec<(String, IndexedFile)>, &'static str> {
        Self::indexed_entries(&self.db)
    }
}

/// Vectors for semantic search: the query's, then one per `(entry id, text)`. Vectors already
/// in the index are reused; the query and the missing ones are computed by `embed` in one call
/// and stored. `None` when `embed` fails or vectors disagree in size: the caller then ranks by
/// keywords. The core is only locked to read and write the index, never while embedding.
pub fn semantic_vectors(
    core: &std::sync::Mutex<Core>,
    model: &str,
    query: &str,
    entries: &[(&str, &str)],
    embed: impl FnOnce(&[String]) -> Option<Vec<Vec<f32>>>,
) -> Option<Vec<Vec<f32>>> {
    let lock = || core.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ids: Vec<&str> = entries.iter().map(|(id, _)| *id).collect();
    let mut cached = lock().cached_vectors(model, &ids);
    let missing: Vec<usize> = (0..entries.len()).filter(|index| cached[*index].is_none()).collect();
    let texts: Vec<String> =
        std::iter::once(query.to_owned()).chain(missing.iter().map(|index| entries[*index].1.to_owned())).collect();
    let mut computed = embed(&texts).filter(|vectors| vectors.len() == texts.len())?.into_iter();
    let query_vector = computed.next()?;
    let fresh: Vec<(usize, Vec<f32>)> = missing.into_iter().zip(computed).collect();
    if fresh.iter().any(|(_, vector)| vector.len() != query_vector.len())
        || cached.iter().flatten().any(|vector| vector.len() != query_vector.len())
    {
        return None;
    }
    if !fresh.is_empty() {
        let rows: Vec<(&str, &[f32])> = fresh.iter().map(|(index, vector)| (ids[*index], vector.as_slice())).collect();
        // A failed write only costs a recomputation next time.
        let _ = lock().store_vectors(model, &rows);
    }
    for (index, vector) in fresh {
        cached[index] = Some(vector);
    }
    std::iter::once(Some(query_vector)).chain(cached).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn vectors_are_cached_encrypted_and_dropped_with_their_entry() {
        let directory = std::env::temp_dir().join(format!("aiwin11-vectors-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("store.sqlite3");
        let files = crate::local_files::LocalFiles::open(&directory).unwrap();
        let mut core = Core::open(&path).unwrap();
        let note = core.remember("Le chat s'appelle Minou").unwrap();
        std::fs::write(files.root().join("a.txt"), "Rapport Zéphyrin").unwrap();
        core.index_file(&files, "a.txt").unwrap();
        let file = core.indexed_files_with_ids().unwrap().remove(0).0;
        let core = std::sync::Mutex::new(core);
        let entries = [(note.as_str(), "Le chat s'appelle Minou"), (file.as_str(), "Rapport Zéphyrin")];
        let embed = |texts: &[String]| Some(texts.iter().map(|text| vec![text.len() as f32, 1.0]).collect::<Vec<_>>());
        let first = semantic_vectors(&core, "m", "chat", &entries, embed).unwrap();
        assert_eq!(first.len(), 3);
        // Second search: only the query is embedded.
        let seen = std::cell::RefCell::new(Vec::new());
        let again = semantic_vectors(&core, "m", "chat", &entries, |texts: &[String]| {
            seen.borrow_mut().extend_from_slice(texts);
            Some(vec![vec![4.0, 1.0]])
        })
        .unwrap();
        assert_eq!(seen.borrow().as_slice(), ["chat"]);
        assert_eq!(again[1..], first[1..]);
        // Vectors of another size are never mixed; a failed embedding means keyword search.
        assert!(semantic_vectors(&core, "m", "chat", &entries, |_| Some(vec![vec![1.0, 2.0, 3.0]])).is_none());
        assert!(semantic_vectors(&core, "m", "chat", &entries, |_| None).is_none());
        let mut core = core.into_inner().unwrap();
        let count = |core: &Core| -> i64 {
            core.db.query_row("SELECT COUNT(*) FROM memory_vectors", [], |row| row.get(0)).unwrap()
        };
        assert_eq!(count(&core), 2);
        // Indexing the file again drops its stale vector; forgetting the note drops the other.
        std::fs::write(files.root().join("a.txt"), "Autre rapport").unwrap();
        core.index_file(&files, "a.txt").unwrap();
        assert_eq!(count(&core), 1);
        core.forget(Some(&note)).unwrap();
        assert_eq!(count(&core), 0);
        drop(core);
        let bytes = std::fs::read(&path).unwrap();
        let plain = vector_bytes(&[24.0, 1.0]);
        assert!(!bytes.windows(plain.len()).any(|window| window == plain), "vectors are encrypted");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn indexing_is_bounded_encrypted_and_removable() {
        let directory = std::env::temp_dir().join(format!("aiwin11-index-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("store.sqlite3");
        let files = crate::local_files::LocalFiles::open(&directory).unwrap();
        let mut core = Core::open(&path).unwrap();
        std::fs::write(files.root().join("rapport.txt"), "État de la collection Zéphyrin").unwrap();
        assert!(core.index_file(&files, "../rapport.txt").is_err());
        core.index_file(&files, "rapport.txt").unwrap();
        let results = search_indexed_files("Zéphyrin", core.indexed_files().unwrap(), None);
        assert_eq!(results[0].name, "rapport.txt");
        assert!(core.memories().unwrap().is_empty());
        std::fs::write(files.root().join("rapport.txt"), "Nouveau rapport").unwrap();
        core.index_file(&files, "rapport.txt").unwrap();
        assert_eq!(core.indexed_file_names().unwrap(), ["rapport.txt"]);
        assert_eq!(core.indexed_files().unwrap()[0].excerpt, "Nouveau rapport");
        drop(core);
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("Nouveau rapport"));
        let mut core = Core::open(&path).unwrap();
        core.remove_indexed_file("rapport.txt").unwrap();
        assert!(core.indexed_files().unwrap().is_empty());
        drop(core);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn notes_are_encrypted_at_rest_and_can_be_forgotten() {
        let path = std::env::temp_dir().join(format!("aiwin11-memory-{}.sqlite3", Uuid::new_v4()));
        {
            let mut core = Core::open(&path).unwrap();
            assert_eq!(core.remember("  "), Err("MEMORY_INVALID"));
            core.remember("Mon chat s'appelle Zéphyrin").unwrap();
            let id = core.remember("Je préfère le thème sombre").unwrap();
            let notes = core.memories().unwrap();
            assert_eq!(notes.len(), 2);
            assert_eq!(notes[0].text, "Mon chat s'appelle Zéphyrin");
            assert_eq!(core.forget(Some(&id)), Ok(1));
            assert_eq!(core.memories().unwrap().len(), 1);
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("Zéphyrin"), "plain text written to SQLite");
        let mut core = Core::open(&path).unwrap();
        assert_eq!(core.memories().unwrap()[0].text, "Mon chat s'appelle Zéphyrin");
        let routine = core.save_routine("Résume mes fichiers").unwrap();
        assert_eq!(core.routines().unwrap()[0].text, "Résume mes fichiers");
        assert_eq!(core.memories().unwrap().len(), 1, "routines are not notes");
        assert_eq!(core.forget(None), Ok(1), "clearing notes keeps routines");
        assert_eq!(core.forget(Some(&routine)), Ok(1));
        drop(core);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn notes_are_ranked_by_embeddings_or_keywords() {
        let note =
            |text: &str| MemoryEntry { id: text.into(), created_at: String::new(), text: text.into(), schedule: None };
        let notes = vec![note("Mon chat s'appelle Zéphyrin"), note("Je préfère le thème sombre"), note("Rien")];
        let found = rank("comment s'appelle mon chat ?", notes.clone(), None, 5);
        assert_eq!(found[0].text, "Mon chat s'appelle Zéphyrin");
        assert_eq!(found.len(), 1);
        let vectors = vec![vec![0.0, 1.0], vec![1.0, 0.0], vec![0.1, 0.9], vec![0.0, 0.0]];
        let found = rank("thème", notes, Some(&vectors), 1);
        assert_eq!(found[0].text, "Je préfère le thème sombre");
    }

    #[test]
    fn scheduled_routines_run_once_a_day_and_reports_are_kept() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let id = core.save_routine("Quel est l'état du PC ?").unwrap();
        assert_eq!(core.set_routine_schedule(&id, Some("25:00")), Err("SCHEDULE_INVALID"));
        assert!(core.take_due_routine(false).unwrap().is_none(), "not scheduled");
        core.set_routine_schedule(&id, Some("00:00")).unwrap();
        assert_eq!(core.routines().unwrap()[0].schedule.as_deref(), Some("00:00"));
        assert_eq!(core.take_due_routine(false).unwrap().unwrap().id, id);
        core.update_routine(&id, "Lis le rapport du PC.").unwrap();
        let updated = &core.routines().unwrap()[0];
        assert_eq!(updated.text, "Lis le rapport du PC.");
        assert_eq!(updated.schedule.as_deref(), Some("00:00"));
        assert!(core.take_due_routine(false).unwrap().is_none(), "once per day");
        core.set_routine_schedule(&id, Some("00:00@0123456")).unwrap();
        assert!(core.take_due_routine(false).unwrap().is_none(), "editing keeps today's execution");
        let startup = core.save_routine("Vérifie l’état du PC").unwrap();
        core.set_routine_schedule(&startup, Some("startup")).unwrap();
        assert!(core.take_due_routine(false).unwrap().is_none());
        assert_eq!(core.take_due_routine(true).unwrap().unwrap().id, startup);
        assert!(core.take_due_routine(true).unwrap().is_none());
        core.save_report("Tout va bien").unwrap();
        assert_eq!(core.reports().unwrap()[0].text, "Tout va bien");
        assert!(core.memories().unwrap().is_empty() && core.routines().unwrap().len() == 2);
        assert!(valid_schedule("07:30") && !valid_schedule("7:30") && !valid_schedule("07:60"));
        assert!(schedule_due("07:30@15", "07:30", "1"));
        assert!(!schedule_due("07:30@15", "07:30", "2"));
        assert!(!schedule_due("07:30@15", "07:29", "1"));
        for invalid in ["07:30@", "07:30@11", "07:30@7", "07:30@51", "07:30@1@2"] {
            assert!(!valid_schedule(invalid));
        }
    }

    #[test]
    fn notes_are_bounded() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.remember(&"a".repeat(MAX_NOTE_CHARS + 1)), Err("MEMORY_INVALID"));
        assert!(valid_note("ok") && !valid_note("a\0b"));
    }
}
