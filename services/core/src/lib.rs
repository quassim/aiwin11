pub mod broker;
pub mod model_gateway;
pub mod policy;
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

use crate::policy::{Autonomy, PolicySettings};

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
            0 => {
                transaction.execute_batch(include_str!("../migrations/001.sql"))?;
                transaction.execute_batch(include_str!("../migrations/002.sql"))?;
            }
            1 => transaction.execute_batch(include_str!("../migrations/002.sql"))?,
            2 => {}
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

    /// Current policy settings. Missing settings mean the defaults (Assisted, no simulation).
    /// An unreadable store or an unknown value resolves to the most restrictive profile.
    pub fn policy_settings(&self) -> PolicySettings {
        let read = |key: &str| -> rusqlite::Result<Option<String>> {
            self.db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    error => Err(error),
                })
        };
        let restrictive = PolicySettings { autonomy: Autonomy::Observation, simulate: false };
        let (Ok(autonomy), Ok(simulate)) = (read("policy.autonomy"), read("policy.simulate")) else {
            return restrictive;
        };
        let autonomy = match autonomy {
            None => Autonomy::default(),
            Some(code) => match Autonomy::from_code(&code) {
                Some(autonomy) => autonomy,
                None => return restrictive,
            },
        };
        PolicySettings { autonomy, simulate: simulate.as_deref() == Some("true") }
    }

    /// Stores the policy settings and journals the change (profile code only).
    pub fn set_policy_settings(&mut self, settings: PolicySettings) -> rusqlite::Result<()> {
        let transaction = self.db.transaction()?;
        for (key, value) in [
            ("policy.autonomy", settings.autonomy.code()),
            ("policy.simulate", if settings.simulate { "true" } else { "false" }),
        ] {
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![key, value],
            )?;
        }
        let code = format!("{}{}", settings.autonomy.code(), if settings.simulate { "+simulate" } else { "" });
        transaction.execute(
            "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.policy.changed', 'core', ?3)",
            params![Uuid::new_v4().to_string(), self.session_id, code],
        )?;
        transaction.commit()
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
        db.pragma_update(None, "user_version", 3).unwrap();
        assert!(Core::start(db).is_err());
    }

    #[test]
    fn a_version_1_database_is_migrated_in_place() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/001.sql")).unwrap();
        db.execute("INSERT INTO sessions (id) VALUES ('old')", []).unwrap();
        let core = Core::start(db).unwrap();
        let version: u32 = core.db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 2);
        let sessions: u32 = core.db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(sessions, 2, "existing data is kept");
        assert_eq!(core.policy_settings(), PolicySettings::default());
    }

    #[test]
    fn policy_settings_persist_and_unreadable_values_fall_back_to_observation() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.policy_settings(), PolicySettings::default());
        let chosen = PolicySettings { autonomy: Autonomy::Limited, simulate: true };
        core.set_policy_settings(chosen).unwrap();
        assert_eq!(core.policy_settings(), chosen);
        assert_eq!(core.activity().unwrap()[0].kind, "settings.policy.changed");
        core.db.execute("UPDATE settings SET value = 'root' WHERE key = 'policy.autonomy'", []).unwrap();
        assert_eq!(core.policy_settings().autonomy, Autonomy::Observation);
        core.db.execute("DROP TABLE settings", []).unwrap();
        assert_eq!(core.policy_settings().autonomy, Autonomy::Observation);
    }
}
