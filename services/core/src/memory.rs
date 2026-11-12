//! Durable memory (ROADMAP R6): short notes the user lets the agent keep ("souviens-toi
//! que…"). Each note is encrypted with Windows DPAPI for the current user before it reaches
//! SQLite (ROADMAP §6 rule 2); the journal only records that a note was saved or deleted.

use std::{ptr, slice};

use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB},
};

use crate::Core;

pub const MAX_NOTE_CHARS: usize = 1000;
pub const MAX_NOTES: u32 = 200;
const ENTROPY: &[u8] = b"AiWin11.memory.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub created_at: String,
    pub text: String,
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

    fn store(&mut self, text: &str, kind: &'static str) -> Result<String, &'static str> {
        if !valid_note(text) {
            return Err("MEMORY_INVALID");
        }
        let count: u32 = self
            .db
            .query_row("SELECT COUNT(*) FROM memory_entries", [], |row| row.get(0))
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

    fn entries(&self, kind: &str) -> Result<Vec<MemoryEntry>, &'static str> {
        let read = || -> rusqlite::Result<Vec<(String, String, Vec<u8>)>> {
            let mut statement = self.db.prepare(
                "SELECT id, created_at, content FROM memory_entries WHERE kind = ?1 ORDER BY created_at, rowid",
            )?;
            let rows = statement.query_map([kind], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect()
        };
        Ok(read()
            .map_err(|_| "MEMORY_UNAVAILABLE")?
            .into_iter()
            .filter_map(|(id, created_at, cipher)| {
                let text = String::from_utf8(unprotect(&cipher)?).ok()?;
                Some(MemoryEntry { id, created_at, text })
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

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
        let note = |text: &str| MemoryEntry { id: text.into(), created_at: String::new(), text: text.into() };
        let notes = vec![note("Mon chat s'appelle Zéphyrin"), note("Je préfère le thème sombre"), note("Rien")];
        let found = rank("comment s'appelle mon chat ?", notes.clone(), None, 5);
        assert_eq!(found[0].text, "Mon chat s'appelle Zéphyrin");
        assert_eq!(found.len(), 1);
        let vectors = vec![vec![0.0, 1.0], vec![1.0, 0.0], vec![0.1, 0.9], vec![0.0, 0.0]];
        let found = rank("thème", notes, Some(&vectors), 1);
        assert_eq!(found[0].text, "Je préfère le thème sombre");
    }

    #[test]
    fn notes_are_bounded() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.remember(&"a".repeat(MAX_NOTE_CHARS + 1)), Err("MEMORY_INVALID"));
        assert!(valid_note("ok") && !valid_note("a\0b"));
    }
}
