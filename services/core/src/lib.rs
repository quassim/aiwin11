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

use serde::{Deserialize, Serialize};

use crate::policy::{Autonomy, PolicySettings};

/// Remembered model choices. Only public identifiers: never a key, a prompt or a reply.
/// The cloud approval is deliberately not remembered: it is given again for each request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreferences {
    /// `ollama` or `openai`.
    pub provider: String,
    pub local_model: String,
    pub cloud_model: String,
    pub vision_model: String,
}

const PREFERENCE_KEYS: [&str; 4] = ["model.provider", "model.local", "model.cloud", "model.vision"];

impl ModelPreferences {
    /// A model identifier: at most 256 characters, printable, no path or control characters.
    fn valid_identifier(value: &str) -> bool {
        value.chars().count() <= 256 && value.chars().all(|c| !c.is_control())
    }

    fn validated(self) -> Option<Self> {
        let provider_ok = matches!(self.provider.as_str(), "ollama" | "openai");
        let identifiers_ok = [&self.local_model, &self.cloud_model, &self.vision_model]
            .iter().all(|value| Self::valid_identifier(value));
        (provider_ok && identifiers_ok).then_some(self)
    }
}

pub struct Core {
    db: Connection,
    session_id: String,
}

#[derive(Debug)]
pub struct Activity {
    pub sequence: u64,
    pub kind: String,
    pub timestamp: String,
    pub task_id: Option<String>,
    /// Stable code (decision, rule or error), never user content.
    pub code: Option<String>,
}

/// One chat task as persisted, for the activity page and recovery.
#[derive(Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: String,
    pub state: String,
    /// Final code, or for an interrupted task the state it was in when interrupted.
    pub outcome: Option<String>,
    pub created_at: String,
    pub updated_at: String,
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
        Self::recover_interrupted_tasks(&transaction, &session_id)?;
        transaction.commit()?;
        Ok(Self { db, session_id })
    }

    /// A task still in progress at startup was interrupted by a crash or a forced exit. It is
    /// closed as `interrupted` (its last state kept as outcome) and journaled in the new
    /// session. Nothing is re-run: a side effect that may have happened is never repeated.
    fn recover_interrupted_tasks(transaction: &rusqlite::Transaction<'_>, session_id: &str) -> rusqlite::Result<()> {
        let open: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT id, state FROM tasks WHERE state NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
            )?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (task, state) in open {
            transaction.execute(
                "UPDATE tasks SET state = 'interrupted', outcome = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![task, state],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, task_id, kind, source, code)
                 VALUES (?1, ?2, ?3, 'task.recovered', 'core', ?4)",
                params![Uuid::new_v4().to_string(), session_id, task, state],
            )?;
        }
        Ok(())
    }

    /// Persists a new task in this session.
    pub(crate) fn task_created(&self, task: &str) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO tasks (id, session_id, state) VALUES (?1, ?2, 'created')",
            params![task, self.session_id],
        )?;
        Ok(())
    }

    /// Updates a task's state. A final state is never changed afterwards.
    pub(crate) fn task_state(&self, task: &str, state: &str, outcome: Option<&str>) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE tasks SET state = ?2, outcome = COALESCE(?3, outcome),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND session_id = ?4
               AND state NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
            params![task, state, outcome, self.session_id],
        )?;
        Ok(())
    }

    /// Most recent tasks across sessions, newest first.
    pub fn recent_tasks(&self, limit: u32) -> rusqlite::Result<Vec<TaskRecord>> {
        let mut statement = self.db.prepare(
            "SELECT id, state, outcome, created_at, updated_at FROM tasks
             ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| Ok(TaskRecord {
            id: row.get(0)?, state: row.get(1)?, outcome: row.get(2)?,
            created_at: row.get(3)?, updated_at: row.get(4)?,
        }))?;
        rows.collect()
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                error => Err(error),
            })
    }

    /// Remembered model choices; defaults (local provider, nothing chosen) when absent or invalid.
    pub fn model_preferences(&self) -> ModelPreferences {
        let mut values = PREFERENCE_KEYS.iter()
            .map(|key| self.setting(key).ok().flatten().unwrap_or_default());
        let preferences = ModelPreferences {
            provider: values.next().unwrap_or_default(),
            local_model: values.next().unwrap_or_default(),
            cloud_model: values.next().unwrap_or_default(),
            vision_model: values.next().unwrap_or_default(),
        };
        preferences.validated().unwrap_or(ModelPreferences { provider: "ollama".into(), ..Default::default() })
    }

    /// Stores the model choices and journals the change (provider code only, never a name).
    pub fn set_model_preferences(&mut self, preferences: ModelPreferences) -> Result<(), &'static str> {
        let preferences = preferences.validated().ok_or("SETTINGS_INVALID")?;
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            for (key, value) in PREFERENCE_KEYS.iter().zip([
                &preferences.provider, &preferences.local_model, &preferences.cloud_model, &preferences.vision_model,
            ]) {
                transaction.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                    params![key, value],
                )?;
            }
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.models.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), self.session_id, preferences.provider],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Current policy settings. Missing settings mean the defaults (Assisted, no simulation).
    /// An unreadable store or an unknown value resolves to the most restrictive profile.
    pub fn policy_settings(&self) -> PolicySettings {
        let read = |key: &str| self.setting(key);
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
            "SELECT sequence, kind, timestamp, task_id, code FROM events
             WHERE session_id = ?1 ORDER BY sequence DESC LIMIT 100",
        )?;
        let events = statement.query_map([&self.session_id], |row| {
            Ok(Activity {
                sequence: row.get(0)?,
                kind: row.get(1)?,
                timestamp: row.get(2)?,
                task_id: row.get(3)?,
                code: row.get(4)?,
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
    fn interrupted_tasks_are_closed_at_the_next_start_and_never_reopened() {
        let path = std::env::temp_dir().join(format!("aiwin11-recovery-{}.sqlite3", Uuid::new_v4()));
        {
            let core = Core::open(&path).unwrap();
            for (task, state) in [("a", "dispatching"), ("b", "awaiting_approval"), ("c", "completed")] {
                core.task_created(task).unwrap();
                core.task_state(task, state, if state == "completed" { Some("ok") } else { None }).unwrap();
            }
            core.task_state("c", "planning", None).unwrap();
            assert_eq!(core.recent_tasks(10).unwrap().iter().find(|task| task.id == "c").unwrap().state,
                "completed", "a final state never changes");
        } // The process "crashes" with a and b still in progress.
        let core = Core::open(&path).unwrap();
        let tasks = core.recent_tasks(10).unwrap();
        let state = |id: &str| tasks.iter().find(|task| task.id == id)
            .map(|task| (task.state.clone(), task.outcome.clone())).unwrap();
        assert_eq!(state("a"), ("interrupted".into(), Some("dispatching".into())));
        assert_eq!(state("b"), ("interrupted".into(), Some("awaiting_approval".into())));
        assert_eq!(state("c"), ("completed".into(), Some("ok".into())));
        let recovered: Vec<_> = core.activity().unwrap().into_iter()
            .filter(|event| event.kind == "task.recovered")
            .map(|event| (event.task_id.unwrap(), event.code.unwrap())).collect();
        assert_eq!(recovered.len(), 2);
        assert!(recovered.contains(&("a".into(), "dispatching".into())));
        core.task_state("a", "planning", None).unwrap();
        assert_eq!(core.recent_tasks(10).unwrap().iter().find(|task| task.id == "a").unwrap().state, "interrupted");
        drop(core);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn model_preferences_persist_and_invalid_values_are_refused() {
        let path = std::env::temp_dir().join(format!("aiwin11-prefs-{}.sqlite3", Uuid::new_v4()));
        let chosen = ModelPreferences {
            provider: "openai".into(), local_model: "qwen3:8b".into(),
            cloud_model: "gpt-x".into(), vision_model: "gemma3:4b".into(),
        };
        {
            let mut core = Core::open(&path).unwrap();
            assert_eq!(core.model_preferences().provider, "ollama", "local by default");
            core.set_model_preferences(chosen.clone()).unwrap();
            let event = &core.activity().unwrap()[0];
            assert_eq!((event.kind.as_str(), event.code.as_deref()), ("settings.models.changed", Some("openai")));
            let bad = ModelPreferences { provider: "other".into(), ..chosen.clone() };
            assert_eq!(core.set_model_preferences(bad), Err("SETTINGS_INVALID"));
            let bad = ModelPreferences { local_model: "a\nb".into(), ..chosen.clone() };
            assert_eq!(core.set_model_preferences(bad), Err("SETTINGS_INVALID"));
        }
        let core = Core::open(&path).unwrap();
        assert_eq!(core.model_preferences(), chosen, "remembered across restarts");
        core.db.execute("UPDATE settings SET value = 'x' WHERE key = 'model.provider'", []).unwrap();
        assert_eq!(core.model_preferences().provider, "ollama");
        drop(core);
        let _ = std::fs::remove_file(&path);
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
