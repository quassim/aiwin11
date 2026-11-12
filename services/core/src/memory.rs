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

impl Core {
    /// Encrypts and stores one note. Returns its ID.
    pub fn remember(&mut self, text: &str) -> Result<String, &'static str> {
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
            transaction.execute("INSERT INTO memory_entries (id, content) VALUES (?1, ?2)", params![id, cipher])?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'memory.saved', 'core')",
                params![Uuid::new_v4().to_string(), session],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "MEMORY_UNAVAILABLE")?;
        Ok(id)
    }

    /// All notes, oldest first. A note that no longer decrypts (other Windows user) is skipped.
    pub fn memories(&self) -> Result<Vec<MemoryEntry>, &'static str> {
        let read = || -> rusqlite::Result<Vec<(String, String, Vec<u8>)>> {
            let mut statement =
                self.db.prepare("SELECT id, created_at, content FROM memory_entries ORDER BY created_at, rowid")?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
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
                None => transaction.execute("DELETE FROM memory_entries", [])?,
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
        assert_eq!(core.forget(None), Ok(1));
        drop(core);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn notes_are_bounded() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.remember(&"a".repeat(MAX_NOTE_CHARS + 1)), Err("MEMORY_INVALID"));
        assert!(valid_note("ok") && !valid_note("a\0b"));
    }
}
