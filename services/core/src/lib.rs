#[cfg(windows)]
pub mod admin;
pub mod broker;
pub mod grants;
#[cfg(windows)]
pub mod local_files;
pub mod manifest;
#[cfg(windows)]
pub mod memory;
pub mod model_gateway;
#[cfg(windows)]
pub mod openai;
pub mod policy;
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

/// Default retention of the history (tasks, events) of past sessions, in days.
pub const DEFAULT_RETENTION_DAYS: u32 = 90;
/// Accepted retention range, in days.
pub const RETENTION_RANGE: std::ops::RangeInclusive<u32> = 1..=3650;

/// One exported audit event: codes and identifiers only, never user content.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub sequence: u64,
    pub id: String,
    pub session_id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub source: String,
    pub code: Option<String>,
    pub timestamp: String,
}

/// Journal guard: event kinds, codes and task IDs are short machine codes. Anything else
/// (free text, a path, a prompt fragment) is replaced by `REDACTED` before it is stored, so a
/// programming error can never write user content into the audit journal.
pub(crate) fn journal_code(value: &str) -> &str {
    let safe = !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:+/-".contains(&b));
    if safe {
        value
    } else {
        "REDACTED"
    }
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
            .iter()
            .all(|value| Self::valid_identifier(value));
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
                transaction.execute_batch(include_str!("../migrations/003.sql"))?;
                transaction.execute_batch(include_str!("../migrations/004.sql"))?;
                transaction.execute_batch(include_str!("../migrations/005.sql"))?;
            }
            1 => {
                transaction.execute_batch(include_str!("../migrations/002.sql"))?;
                transaction.execute_batch(include_str!("../migrations/003.sql"))?;
                transaction.execute_batch(include_str!("../migrations/004.sql"))?;
                transaction.execute_batch(include_str!("../migrations/005.sql"))?;
            }
            2 => {
                transaction.execute_batch(include_str!("../migrations/003.sql"))?;
                transaction.execute_batch(include_str!("../migrations/004.sql"))?;
                transaction.execute_batch(include_str!("../migrations/005.sql"))?;
            }
            3 => {
                transaction.execute_batch(include_str!("../migrations/004.sql"))?;
                transaction.execute_batch(include_str!("../migrations/005.sql"))?;
            }
            4 => transaction.execute_batch(include_str!("../migrations/005.sql"))?,
            5 => {}
            _ => return Err(rusqlite::Error::InvalidQuery),
        }
        let session_id = Uuid::new_v4().to_string();
        transaction.execute("INSERT INTO sessions (id) VALUES (?1)", [&session_id])?;
        transaction.execute(
            "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'session.started', 'core')",
            params![Uuid::new_v4().to_string(), session_id],
        )?;
        Self::recover_interrupted_tasks(&transaction, &session_id)?;
        Self::apply_retention(&transaction, &session_id)?;
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

    fn retention_days_in(db: &Connection) -> u32 {
        db.query_row("SELECT value FROM settings WHERE key = 'history.retention_days'", [], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|days| RETENTION_RANGE.contains(days))
        .unwrap_or(DEFAULT_RETENTION_DAYS)
    }

    /// Deletes the history of past sessions older than the retention period. The current
    /// session is never purged. Journals `history.purged` with the number of sessions removed.
    fn apply_retention(db: &Connection, session_id: &str) -> rusqlite::Result<usize> {
        let days = Self::retention_days_in(db);
        let cutoff = format!("-{days} days");
        let old = "SELECT id FROM sessions WHERE id <> ?1
                   AND created_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)";
        for table in ["events", "tasks", "tool_executions"] {
            db.execute(&format!("DELETE FROM {table} WHERE session_id IN ({old})"), params![session_id, cutoff])?;
        }
        let removed = db.execute(
            "DELETE FROM sessions WHERE id <> ?1 AND created_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)",
            params![session_id, cutoff],
        )?;
        if removed > 0 {
            db.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'history.purged', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session_id, format!("sessions:{removed}")],
            )?;
        }
        Ok(removed)
    }

    /// Private mode: no request ever goes to a cloud provider (T-011). Off by default; an
    /// unreadable setting counts as on, the most restrictive choice.
    pub fn private_mode(&self) -> bool {
        match self.setting("privacy.strict") {
            Ok(Some(value)) => value != "false",
            Ok(None) => false,
            Err(_) => true,
        }
    }

    pub fn set_private_mode(&mut self, enabled: bool) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('privacy.strict', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [if enabled { "true" } else { "false" }],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.privacy.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, if enabled { "strict" } else { "hybrid" }],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Routing `local_first` with an explicit cloud fallback: when on, a task whose local
    /// provider is unreachable may continue with OpenAI, only before any step ran, only with
    /// the request's cloud approval and never in private mode. Off by default.
    pub fn cloud_fallback_enabled(&self) -> bool {
        matches!(self.setting("routing.cloud_fallback"), Ok(Some(value)) if value == "true")
    }

    pub fn set_cloud_fallback(&mut self, enabled: bool) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('routing.cloud_fallback', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [if enabled { "true" } else { "false" }],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.routing.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, if enabled { "local_first" } else { "manual" }],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// The cloud provider used for cloud tasks (default OpenAI).
    #[cfg(windows)]
    pub fn cloud_vendor(&self) -> openai::Vendor {
        self.setting("cloud.vendor")
            .ok()
            .flatten()
            .and_then(|code| openai::Vendor::from_code(&code))
            .unwrap_or(openai::Vendor::OpenAi)
    }

    #[cfg(windows)]
    pub fn set_cloud_vendor(&mut self, vendor: openai::Vendor) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('cloud.vendor', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [vendor.code()],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.cloud.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, vendor.code()],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// The folder the user chose for the agent's file tools, if any (applied at startup).
    pub fn workspace(&self) -> Option<std::path::PathBuf> {
        self.setting("files.workspace").ok().flatten().filter(|path| !path.is_empty()).map(Into::into)
    }

    /// Stores the agent's folder (an empty path restores AiWin11's dedicated folder). The path
    /// must already be validated by `LocalFiles::validate_workspace`.
    pub fn set_workspace(&mut self, path: &str) -> Result<(), &'static str> {
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('files.workspace', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [path],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.journal("settings.workspace.changed", Some(if path.is_empty() { "dedicated" } else { "user_folder" }))
            .map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Most tokens one task may spend with a cloud provider (default 50 000, 1 000 to 2 000 000).
    pub fn cloud_token_budget(&self) -> u64 {
        self.setting("routing.cloud_token_budget")
            .ok()
            .flatten()
            .and_then(|value| value.parse().ok())
            .filter(|budget| (1_000..=2_000_000).contains(budget))
            .unwrap_or(50_000)
    }

    pub fn set_cloud_token_budget(&mut self, budget: u64) -> Result<(), &'static str> {
        if !(1_000..=2_000_000).contains(&budget) {
            return Err("SETTINGS_INVALID");
        }
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('routing.cloud_token_budget', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [budget.to_string()],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.budget.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, format!("tokens:{budget}")],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Retention of past sessions, in days.
    pub fn retention_days(&self) -> u32 {
        Self::retention_days_in(&self.db)
    }

    /// Changes the retention and applies it at once.
    pub fn set_retention_days(&mut self, days: u32) -> Result<(), &'static str> {
        if !RETENTION_RANGE.contains(&days) {
            return Err("SETTINGS_INVALID");
        }
        let session_id = self.session_id.clone();
        let apply = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('history.retention_days', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [days.to_string()],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.retention.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session_id, format!("days:{days}")],
            )?;
            Self::apply_retention(&transaction, &session_id)?;
            transaction.commit()
        };
        apply(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Erases the whole history of past sessions (tasks, events, dispatch reservations). The
    /// current session keeps its journal, ending with `history.cleared`.
    pub fn clear_history(&mut self) -> Result<usize, &'static str> {
        let session_id = self.session_id.clone();
        let clear = |db: &mut Connection| -> rusqlite::Result<usize> {
            let transaction = db.transaction()?;
            for table in ["events", "tasks", "tool_executions"] {
                transaction.execute(&format!("DELETE FROM {table} WHERE session_id <> ?1"), [&session_id])?;
            }
            let removed = transaction.execute("DELETE FROM sessions WHERE id <> ?1", [&session_id])?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'history.cleared', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session_id, format!("sessions:{removed}")],
            )?;
            transaction.commit()?;
            Ok(removed)
        };
        clear(&mut self.db).map_err(|_| "HISTORY_UNAVAILABLE")
    }

    /// The whole audit journal (all kept sessions), oldest first, for export. Journals
    /// `history.exported` with the number of entries.
    pub fn export_audit(&self) -> Result<Vec<AuditEntry>, &'static str> {
        let read = || -> rusqlite::Result<Vec<AuditEntry>> {
            let mut statement = self.db.prepare(
                "SELECT sequence, id, session_id, task_id, kind, source, code, timestamp FROM events ORDER BY sequence",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(AuditEntry {
                    sequence: row.get(0)?,
                    id: row.get(1)?,
                    session_id: row.get(2)?,
                    task_id: row.get(3)?,
                    kind: row.get(4)?,
                    source: row.get(5)?,
                    code: row.get(6)?,
                    timestamp: row.get(7)?,
                })
            })?;
            rows.collect()
        };
        let entries = read().map_err(|_| "HISTORY_UNAVAILABLE")?;
        self.db.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'history.exported', 'core', ?3)",
            params![Uuid::new_v4().to_string(), self.session_id, format!("entries:{}", entries.len())],
        ).map_err(|_| "HISTORY_UNAVAILABLE")?;
        Ok(entries)
    }

    /// Writes the audit journal as JSON into `directory` (created if needed) and returns the
    /// file path. The file name only carries the UTC export time.
    pub fn export_audit_to(&self, directory: &Path) -> Result<std::path::PathBuf, &'static str> {
        let stamp: String = self
            .db
            .query_row("SELECT strftime('%Y%m%dT%H%M%SZ', 'now')", [], |row| row.get(0))
            .map_err(|_| "HISTORY_UNAVAILABLE")?;
        let entries = self.export_audit()?;
        let json = serde_json::to_vec_pretty(&serde_json::json!({ "format": "aiwin11.audit.v1", "events": entries }))
            .map_err(|_| "HISTORY_UNAVAILABLE")?;
        std::fs::create_dir_all(directory).map_err(|_| "EXPORT_FAILED")?;
        let path = directory.join(format!("aiwin11-audit-{stamp}.json"));
        std::fs::write(&path, json).map_err(|_| "EXPORT_FAILED")?;
        Ok(path)
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
        let rows = statement.query_map([limit], |row| {
            Ok(TaskRecord {
                id: row.get(0)?,
                state: row.get(1)?,
                outcome: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Journals a session-level event. Kind and code are static codes, never user content.
    pub fn journal(&self, kind: &'static str, code: Option<&'static str>) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, ?3, 'core', ?4)",
            params![Uuid::new_v4().to_string(), self.session_id, journal_code(kind), code.map(journal_code)],
        )?;
        Ok(())
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0)).map(Some).or_else(
            |error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                error => Err(error),
            },
        )
    }

    /// Remembered model choices; defaults (local provider, nothing chosen) when absent or invalid.
    pub fn model_preferences(&self) -> ModelPreferences {
        let mut values = PREFERENCE_KEYS.iter().map(|key| self.setting(key).ok().flatten().unwrap_or_default());
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
                &preferences.provider,
                &preferences.local_model,
                &preferences.cloud_model,
                &preferences.vision_model,
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
            assert_eq!(
                core.recent_tasks(10).unwrap().iter().find(|task| task.id == "c").unwrap().state,
                "completed",
                "a final state never changes"
            );
        } // The process "crashes" with a and b still in progress.
        let core = Core::open(&path).unwrap();
        let tasks = core.recent_tasks(10).unwrap();
        let state = |id: &str| {
            tasks.iter().find(|task| task.id == id).map(|task| (task.state.clone(), task.outcome.clone())).unwrap()
        };
        assert_eq!(state("a"), ("interrupted".into(), Some("dispatching".into())));
        assert_eq!(state("b"), ("interrupted".into(), Some("awaiting_approval".into())));
        assert_eq!(state("c"), ("completed".into(), Some("ok".into())));
        let recovered: Vec<_> = core
            .activity()
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "task.recovered")
            .map(|event| (event.task_id.unwrap(), event.code.unwrap()))
            .collect();
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
            provider: "openai".into(),
            local_model: "qwen3:8b".into(),
            cloud_model: "gpt-x".into(),
            vision_model: "gemma3:4b".into(),
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
    fn old_sessions_are_purged_at_start_and_history_can_be_cleared_and_exported() {
        let path = std::env::temp_dir().join(format!("aiwin11-retention-{}.sqlite3", Uuid::new_v4()));
        let old_session = {
            let mut core = Core::open(&path).unwrap();
            core.task_created("old-task").unwrap();
            assert_eq!(core.set_retention_days(0), Err("SETTINGS_INVALID"));
            core.set_retention_days(30).unwrap();
            // Age this session beyond the retention period.
            core.db.execute("UPDATE sessions SET created_at = '2000-01-01T00:00:00.000Z'", []).unwrap();
            core.session_id().to_owned()
        };
        let core = Core::open(&path).unwrap();
        assert_eq!(core.retention_days(), 30);
        for sql in [
            "SELECT COUNT(*) FROM events WHERE session_id = ?1",
            "SELECT COUNT(*) FROM tasks WHERE session_id = ?1",
            "SELECT COUNT(*) FROM sessions WHERE id = ?1",
        ] {
            let count: u32 = core.db.query_row(sql, [&old_session], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "{sql}");
        }
        assert!(core
            .activity()
            .unwrap()
            .iter()
            .any(|event| event.kind == "history.purged" && event.code.as_deref() == Some("sessions:1")));

        // A recent session is kept by retention but removed by an explicit clear.
        drop(core);
        let mut core = Core::open(&path).unwrap();
        let sessions: u32 = core.db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap();
        assert_eq!(sessions, 2);
        assert_eq!(core.clear_history(), Ok(1));
        let exported = core.export_audit().unwrap();
        assert!(exported.iter().all(|entry| entry.session_id == core.session_id()));
        assert_eq!(exported.last().unwrap().kind, "history.cleared");
        assert_eq!(core.activity().unwrap()[0].kind, "history.exported");
        let directory = std::env::temp_dir().join(format!("aiwin11-export-{}", Uuid::new_v4()));
        let file = core.export_audit_to(&directory).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(json["format"], "aiwin11.audit.v1");
        assert!(json["events"].as_array().unwrap().iter().any(|event| event["kind"] == "history.cleared"));
        drop(core);
        let _ = std::fs::remove_dir_all(&directory);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn journal_codes_reject_free_text() {
        for code in [
            "policy.evaluated",
            "ask:launch_application:profile",
            "step:1/8",
            "MODEL_REQUIRED",
            "assisted+simulate",
            "0b6f7c1e-0f3a-4b8e-9a51-2a3c4d5e6f70",
        ] {
            assert_eq!(journal_code(code), code);
        }
        for text in ["", "Mon mot de passe est hunter2", "C:\\Users\\x\\secret.txt", "a\nb", "é", &"a".repeat(97)] {
            assert_eq!(journal_code(text), "REDACTED", "{text}");
        }
    }

    /// Invariant "nothing sensitive in SQLite" (ROADMAP §6): every column is listed here with
    /// what it holds. A new column fails this test until it is reviewed and added.
    #[test]
    fn the_schema_only_holds_identifiers_codes_and_timestamps() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let reviewed = [
            // Temporary grants: action code and timestamps only.
            "grants.action",
            "grants.created_at",
            "grants.expires_at",
            "grants.id",
            "grants.revoked_at",
            // Memory: the content column only holds DPAPI ciphertext (see memory.rs tests).
            "memory_entries.content",
            "memory_entries.created_at",
            "memory_entries.id",
            "memory_entries.kind",
            "events.code",
            "events.id",
            "events.kind",
            "events.sequence",
            "events.session_id",
            "events.source",
            "events.task_id",
            "events.timestamp",
            "sessions.created_at",
            "sessions.id",
            // Keys are fixed by the core; values are profile codes, booleans, retention days and
            // public model identifiers. Secrets stay in Windows Credential Manager.
            "settings.key",
            "settings.updated_at",
            "settings.value",
            "tasks.created_at",
            "tasks.id",
            "tasks.outcome",
            "tasks.session_id",
            "tasks.state",
            "tasks.updated_at",
            "tool_executions.idempotency_key",
            "tool_executions.session_id",
            "tool_executions.task_id",
        ];
        let mut columns: Vec<String> = Vec::new();
        let tables: Vec<String> = {
            let mut statement = core
                .db
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
                .unwrap();
            let rows = statement.query_map([], |row| row.get(0)).unwrap();
            rows.collect::<rusqlite::Result<_>>().unwrap()
        };
        for table in tables {
            let mut statement = core.db.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
            let names: Vec<String> =
                statement.query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            columns.extend(names.into_iter().map(|name| format!("{table}.{name}")));
        }
        columns.sort();
        let mut reviewed: Vec<String> = reviewed.iter().map(|column| column.to_string()).collect();
        reviewed.sort();
        assert_eq!(columns, reviewed);
    }

    #[test]
    fn free_text_never_reaches_the_database_file() {
        let path = std::env::temp_dir().join(format!("aiwin11-sensitive-{}.sqlite3", Uuid::new_v4()));
        let secret = "Ma-carte-4970101234567890-hunter2";
        {
            let core = Core::open(&path).unwrap();
            core.task_created("t").unwrap();
            core.journal("task.started", None).unwrap();
            // A faulty caller passing user content as a code is redacted.
            let leaked = format!("prompt: {secret} !");
            core.audit(Some("t"), "policy.denied", &leaked).unwrap();
            core.export_audit().unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("hunter2"), "free text written to SQLite");
        assert!(text.contains("REDACTED"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn private_mode_is_off_by_default_and_persists() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert!(!core.private_mode());
        core.set_private_mode(true).unwrap();
        assert!(core.private_mode());
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("strict"));
        core.db.execute("DROP TABLE settings", []).unwrap();
        assert!(core.private_mode(), "an unreadable setting blocks the cloud");
    }

    #[test]
    fn cloud_fallback_is_off_by_default_and_persists() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert!(!core.cloud_fallback_enabled());
        core.set_cloud_fallback(true).unwrap();
        assert!(core.cloud_fallback_enabled());
        core.db.execute("DROP TABLE settings", []).unwrap();
        assert!(!core.cloud_fallback_enabled(), "an unreadable setting never allows the cloud");
    }

    #[test]
    fn cloud_token_budget_defaults_and_bounds() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.cloud_token_budget(), 50_000);
        assert_eq!(core.set_cloud_token_budget(10), Err("SETTINGS_INVALID"));
        core.set_cloud_token_budget(20_000).unwrap();
        assert_eq!(core.cloud_token_budget(), 20_000);
    }

    #[test]
    fn a_newer_database_is_rejected() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 6).unwrap();
        assert!(Core::start(db).is_err());
    }

    #[test]
    fn a_version_1_database_is_migrated_in_place() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/001.sql")).unwrap();
        db.execute("INSERT INTO sessions (id) VALUES ('old')", []).unwrap();
        let core = Core::start(db).unwrap();
        let version: u32 = core.db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 5);
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
