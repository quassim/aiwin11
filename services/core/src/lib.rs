pub mod broker;
pub mod model_gateway;
#[cfg(windows)]
pub mod local_files;
#[cfg(windows)]
pub mod openai;
#[cfg(windows)]
pub mod task;
#[cfg(windows)]
pub mod windows_bridge;

use std::{path::Path, time::Duration};

use rusqlite::{params, Connection, TransactionBehavior};
use uuid::Uuid;

pub struct Core {
    db: Connection,
    session_id: String,
}

#[derive(Debug)]
pub struct Activity {
    pub sequence: u64,
    pub kind: String,
    pub timestamp: String,
}

impl Core {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::start(Connection::open(path)?)
    }

    fn start(mut db: Connection) -> rusqlite::Result<Self> {
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => transaction.execute_batch(include_str!("../migrations/001.sql"))?,
            1 => {}
            _ => return Err(rusqlite::Error::InvalidQuery),
        }
        let session_id = Uuid::new_v4().to_string();
        transaction.execute("INSERT INTO sessions (id) VALUES (?1)", [&session_id])?;
        transaction.execute(
            "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'session.started', 'core')",
            params![Uuid::new_v4().to_string(), session_id],
        )?;
        transaction.commit()?;
        Ok(Self { db, session_id })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn activity(&self) -> rusqlite::Result<Vec<Activity>> {
        let mut statement = self.db.prepare(
            "SELECT sequence, kind, timestamp FROM events WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 100",
        )?;
        let events = statement.query_map([&self.session_id], |row| {
            Ok(Activity {
                sequence: row.get(0)?,
                kind: row.get(1)?,
                timestamp: row.get(2)?,
            })
        })?;
        events.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_starts_with_a_persisted_event() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let events = core.activity().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.started");
        assert!(events[0].timestamp.ends_with('Z'));
    }

    #[test]
    fn a_newer_database_is_rejected() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 2).unwrap();
        assert!(Core::start(db).is_err());
    }
}
