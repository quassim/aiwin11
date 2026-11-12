#[cfg(windows)]
pub mod admin;
pub mod backup;
pub mod broker;
pub mod folders;
pub mod grants;
pub mod images;
#[cfg(windows)]
pub mod local_files;
pub mod manifest;
#[cfg(windows)]
pub mod mcp;
#[cfg(windows)]
pub mod memory;
pub mod model_gateway;
#[cfg(windows)]
pub mod openai;
pub mod policy;
#[cfg(windows)]
pub mod task;
pub mod video;
#[cfg(windows)]
pub mod windows_bridge;

use std::{path::Path, time::Duration};

use rusqlite::{params, Connection, TransactionBehavior};
use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::{
    model_gateway::LocalStrategy,
    policy::{Autonomy, PolicySettings},
};

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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentModels {
    pub office: String,
    pub windows_admin: String,
    pub development: String,
    pub research: String,
    pub multimedia: String,
    pub read_only: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalRuntimePreferences {
    pub runtime: model_gateway::LocalRuntime,
    pub lm_studio_model: String,
}

impl AgentModels {
    pub fn for_profile(&self, profile: policy::AgentProfile) -> Option<&str> {
        let model = match profile {
            policy::AgentProfile::General => "",
            policy::AgentProfile::Office => self.office.as_str(),
            policy::AgentProfile::WindowsAdmin => self.windows_admin.as_str(),
            policy::AgentProfile::Development => self.development.as_str(),
            policy::AgentProfile::Research => self.research.as_str(),
            policy::AgentProfile::Multimedia => self.multimedia.as_str(),
            policy::AgentProfile::ReadOnly => self.read_only.as_str(),
        };
        valid_model_name(model).then_some(model)
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<AuditPayload>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AuditPayload {
    Provider { provider: &'static str },
    TaskStep { current: u8, total: u8 },
    RenderTime { elapsed_ms: u32 },
}

fn audit_payload(kind: &str, code: Option<&str>) -> Option<AuditPayload> {
    let code = code?;
    match kind {
        "task.started" | "model.provider.unavailable" | "model.cloud.request" => {
            let provider = match code {
                "ollama" => "ollama",
                "lm_studio" => "lm_studio",
                "openai" => "openai",
                "deepseek" => "deepseek",
                "gemini" => "gemini",
                "anthropic" => "anthropic",
                _ => return None,
            };
            Some(AuditPayload::Provider { provider })
        }
        "task.progress" => {
            let (current, total) = code.strip_prefix("step:")?.split_once('/')?;
            let current = current.parse().ok()?;
            let total = total.parse().ok()?;
            (current > 0 && current <= total).then_some(AuditPayload::TaskStep { current, total })
        }
        "ui.ready" | "ui.navigation" | "ui.palette.ready" => {
            Some(AuditPayload::RenderTime { elapsed_ms: code.strip_prefix("ms:")?.parse().ok()? })
        }
        _ => None,
    }
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

/// Tool-calling status of a model manifest ([S1 §4.2]: `unverified` until a test says otherwise).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolCalling {
    Unverified,
    /// The provider does not advertise tool calling for this model.
    NotAdvertised,
    /// The synthetic tool-call probe succeeded.
    Verified,
    /// Tool calling is advertised but the probe failed.
    Failed,
}

impl ToolCalling {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::NotAdvertised => "not_advertised",
            Self::Verified => "verified",
            Self::Failed => "failed",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Unverified, Self::NotAdvertised, Self::Verified, Self::Failed]
            .into_iter()
            .find(|status| status.code() == code)
    }
}

/// What is known about one local model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelManifest {
    pub name: String,
    pub tool_calling: ToolCalling,
    pub vision_advertised: bool,
    pub probe_ms: Option<u64>,
    pub probed_at: Option<String>,
    pub context_length: Option<u64>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub size_bytes: Option<u64>,
    pub tokens_per_second: Option<u64>,
    pub vram_bytes: Option<u64>,
}

/// What a full probe learnt about a model besides tool calling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelFacts {
    pub vision_advertised: bool,
    pub context_length: Option<u64>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub size_bytes: Option<u64>,
    /// Generation speed measured during the probe.
    pub tokens_per_second: Option<u64>,
    /// GPU memory used once loaded (from `/api/ps`).
    pub vram_bytes: Option<u64>,
}

/// Codes of the closed list of applications the agent can open.
pub const APPLICATION_CODES: [&str; 3] = ["notepad", "charmap", "paint"];

/// At most this many protected file patterns, each 1 to 64 characters.
pub const MAX_PROTECTED_PATTERNS: usize = 20;

/// A file name pattern: `*` and `?` wildcards, no path separator, no control character.
fn valid_pattern(pattern: &str) -> bool {
    let length = pattern.chars().count();
    (1..=64).contains(&length) && !pattern.chars().any(|c| c.is_control() || "\\/:<>\"|".contains(c))
}

/// Case-insensitive match of a whole file name against a `*` / `?` pattern.
pub fn matches_pattern(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    // Iterative matcher with backtracking on the last `*`: linear in practice, no recursion.
    let (mut p, mut n, mut star, mut mark) = (0, 0, None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = n;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// Public model identifiers as Ollama lists them: letters, digits and `-_.:/`, 1 to 256 bytes.
pub fn valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

/// Schema migrations in order; migration N sets `user_version` to N.
const MIGRATIONS: [&str; 10] = [
    include_str!("../migrations/001.sql"),
    include_str!("../migrations/002.sql"),
    include_str!("../migrations/003.sql"),
    include_str!("../migrations/004.sql"),
    include_str!("../migrations/005.sql"),
    include_str!("../migrations/006.sql"),
    include_str!("../migrations/007.sql"),
    include_str!("../migrations/008.sql"),
    include_str!("../migrations/009.sql"),
    include_str!("../migrations/010.sql"),
];

pub struct Core {
    db: Connection,
    session_id: String,
    desktop_pid: u32,
    #[cfg(windows)]
    broker_transport: Option<broker::BrokerTransport>,
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

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudUsage {
    pub tokens: u64,
    pub tasks: u64,
}

/// Cloud cost settings, entered by the user: no price is hard-coded ([S1 §9]).
/// Amounts are in thousandths of the billing currency (shown in euros); 0 means "not set".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCost {
    /// Price of one million tokens (input and output blended), as charged by the provider.
    pub price_milli_per_million: u64,
    /// Most one task may cost; checked before each cloud call. Needs a price.
    pub max_milli_per_task: u64,
}

/// Highest accepted amount: 1 000 units of the currency.
pub const MAX_COST_MILLI: u64 = 1_000_000;

impl CloudCost {
    /// Estimated cost of `tokens`, rounded up, in thousandths of the currency.
    pub fn estimate(self, tokens: u64) -> u64 {
        let cost = (u128::from(tokens) * u128::from(self.price_milli_per_million)).div_ceil(1_000_000);
        u64::try_from(cost).unwrap_or(u64::MAX)
    }

    /// True once `tokens` reach the per-task cap; never true without both a price and a cap.
    pub fn exceeded(self, tokens: u64) -> bool {
        self.price_milli_per_million > 0
            && self.max_milli_per_task > 0
            && self.estimate(tokens) >= self.max_milli_per_task
    }
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
    pub fn local_runtime_preferences(&self) -> LocalRuntimePreferences {
        self.setting("model.local_runtime")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn set_local_runtime_preferences(&mut self, preferences: LocalRuntimePreferences) -> Result<(), &'static str> {
        if !preferences.lm_studio_model.is_empty() && !valid_model_name(&preferences.lm_studio_model) {
            return Err("SETTINGS_INVALID");
        }
        let value = serde_json::to_string(&preferences).map_err(|_| "SETTINGS_INVALID")?;
        let session = self.session_id().to_owned();
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES ('model.local_runtime', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [value],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.model.runtime.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, preferences.runtime.code()],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")
    }
    /// The desktop owns session creation and migrations. The task worker joins that exact session.
    pub fn attach_task_worker(path: &Path, session_id: &str, desktop_pid: u32) -> rusqlite::Result<Self> {
        let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.query_row("SELECT id FROM sessions WHERE id = ?1", [session_id], |_| Ok(()))?;
        Ok(Self {
            db,
            session_id: session_id.to_owned(),
            desktop_pid,
            #[cfg(windows)]
            broker_transport: None,
        })
    }

    /// Close tasks left by a failed worker without retrying their effects.
    pub fn recover_task_worker(&mut self) -> rusqlite::Result<()> {
        let transaction = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::recover_interrupted_tasks(&transaction, &self.session_id, Some(&self.session_id))?;
        transaction.commit()
    }

    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let db = Connection::open(path)?;
        // Restore point: an existing store about to be migrated is copied first. A failed copy
        // stops the start rather than migrating without a way back.
        let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 0 && (version as usize) < MIGRATIONS.len() {
            backup::create(&db, &backup::backup_dir(path), backup::Reason::BeforeMigration)
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
        }
        Self::start(db)
    }

    /// Makes a backup of the store now (journaled `data.backup.created`, code `manual`).
    pub fn backup(&self, dir: &Path) -> Result<backup::BackupInfo, &'static str> {
        let info = backup::create(&self.db, dir, backup::Reason::Manual)?;
        self.journal("data.backup.created", Some("manual")).map_err(|_| "BACKUP_FAILED")?;
        Ok(info)
    }

    fn start(mut db: Connection) -> rusqlite::Result<Self> {
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        // Each migration sets `user_version` to its own number; apply every one above the
        // current version, in order, inside this transaction.
        if version > MIGRATIONS.len() as u32 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        for migration in &MIGRATIONS[version as usize..] {
            transaction.execute_batch(migration)?;
        }
        let session_id = Uuid::new_v4().to_string();
        transaction.execute("INSERT INTO sessions (id) VALUES (?1)", [&session_id])?;
        transaction.execute(
            "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'session.started', 'core')",
            params![Uuid::new_v4().to_string(), session_id],
        )?;
        Self::recover_interrupted_tasks(&transaction, &session_id, None)?;
        Self::apply_retention(&transaction, &session_id)?;
        transaction.commit()?;
        Ok(Self {
            db,
            session_id,
            desktop_pid: std::process::id(),
            #[cfg(windows)]
            broker_transport: None,
        })
    }

    /// A task still in progress at startup was interrupted by a crash or a forced exit. It is
    /// closed as `interrupted` (its last state kept as outcome) and journaled in the new
    /// session. Nothing is re-run: a side effect that may have happened is never repeated.
    fn recover_interrupted_tasks(
        transaction: &rusqlite::Transaction<'_>,
        session_id: &str,
        only_session: Option<&str>,
    ) -> rusqlite::Result<()> {
        let open: Vec<(String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT id, state FROM tasks WHERE (?1 IS NULL OR session_id = ?1)
                 AND state NOT IN ('completed', 'failed', 'cancelled', 'interrupted')",
            )?;
            let rows = statement.query_map([only_session], |row| Ok((row.get(0)?, row.get(1)?)))?;
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
        match self.local_strategy() {
            LocalStrategy::LocalFirst => true,
            LocalStrategy::Manual => self.legacy_cloud_fallback(),
            _ => false,
        }
    }

    pub fn legacy_cloud_fallback(&self) -> bool {
        matches!(self.setting("routing.cloud_fallback"), Ok(Some(value)) if value == "true")
    }

    pub fn agent_models(&self) -> AgentModels {
        self.setting("routing.agent_models")
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn set_agent_models(&mut self, models: AgentModels) -> Result<(), &'static str> {
        if [
            &models.office,
            &models.windows_admin,
            &models.development,
            &models.research,
            &models.multimedia,
            &models.read_only,
        ]
        .into_iter()
        .any(|model| !model.is_empty() && !valid_model_name(model))
        {
            return Err("SETTINGS_INVALID");
        }
        let value = serde_json::to_string(&models).map_err(|_| "SETTINGS_INVALID")?;
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES ('routing.agent_models', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [value],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.routing.agents.changed', 'core', 'saved')",
                params![Uuid::new_v4().to_string(), self.session_id],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Records what a tool-call probe established about a local model ([S1 §4.2] manifest).
    /// Identifiers are checked; the event holds the status code only.
    pub fn record_model_probe(
        &self,
        name: &str,
        tool_calling: ToolCalling,
        facts: Option<&ModelFacts>,
        probe_ms: u64,
    ) -> Result<(), &'static str> {
        if !valid_model_name(name) {
            return Err("MODEL_REQUIRED");
        }
        self.db
            .execute(
                "INSERT INTO models (id, provider, name, tool_calling, vision, probe_ms, probed_at,
                                     context_length, parameter_size, quantization, size_bytes,
                                     tokens_per_second, vram_bytes)
                 VALUES (?1, 'ollama', ?2, ?3, COALESCE(?4, 'not_advertised'), ?5,
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?6, ?7, ?8, ?9, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET tool_calling = excluded.tool_calling,
                 vision = COALESCE(?4, models.vision),
                 context_length = CASE WHEN ?10 THEN excluded.context_length ELSE models.context_length END,
                 parameter_size = CASE WHEN ?10 THEN excluded.parameter_size ELSE models.parameter_size END,
                 quantization = CASE WHEN ?10 THEN excluded.quantization ELSE models.quantization END,
                 size_bytes = CASE WHEN ?10 THEN excluded.size_bytes ELSE models.size_bytes END,
                 tokens_per_second = CASE WHEN ?10 THEN excluded.tokens_per_second ELSE models.tokens_per_second END,
                 vram_bytes = CASE WHEN ?10 THEN excluded.vram_bytes ELSE models.vram_bytes END,
                 probe_ms = excluded.probe_ms, probed_at = excluded.probed_at,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![
                    format!("ollama:{name}"),
                    name,
                    tool_calling.code(),
                    facts.map(|facts| if facts.vision_advertised { "advertised" } else { "not_advertised" }),
                    i64::try_from(probe_ms).unwrap_or(i64::MAX),
                    facts.and_then(|facts| facts.context_length).and_then(|tokens| i64::try_from(tokens).ok()),
                    facts.and_then(|facts| facts.parameter_size.as_deref()),
                    facts.and_then(|facts| facts.quantization.as_deref()),
                    facts.and_then(|facts| facts.size_bytes).and_then(|bytes| i64::try_from(bytes).ok()),
                    facts.is_some(),
                    facts.and_then(|facts| facts.tokens_per_second).and_then(|speed| i64::try_from(speed).ok()),
                    facts.and_then(|facts| facts.vram_bytes).and_then(|bytes| i64::try_from(bytes).ok())
                ],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.db
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'model.manifest.updated', 'core', ?3)",
                params![Uuid::new_v4().to_string(), self.session_id, tool_calling.code()],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        Ok(())
    }

    /// Known local model manifests, most recently probed first.
    pub fn model_manifests(&self) -> rusqlite::Result<Vec<ModelManifest>> {
        let mut statement = self.db.prepare(
            "SELECT name, tool_calling, vision, probe_ms, probed_at, context_length, parameter_size,
                    quantization, size_bytes, tokens_per_second, vram_bytes FROM models
             WHERE provider = 'ollama' ORDER BY probed_at DESC, name LIMIT 64",
        )?;
        let rows = statement.query_map([], |row| {
            let tool_calling: String = row.get(1)?;
            let vision: String = row.get(2)?;
            Ok(ModelManifest {
                name: row.get(0)?,
                tool_calling: ToolCalling::from_code(&tool_calling).unwrap_or(ToolCalling::Unverified),
                vision_advertised: vision == "advertised",
                probe_ms: row.get::<_, Option<i64>>(3)?.and_then(|ms| u64::try_from(ms).ok()),
                probed_at: row.get(4)?,
                context_length: row.get::<_, Option<i64>>(5)?.and_then(|tokens| u64::try_from(tokens).ok()),
                parameter_size: row.get(6)?,
                quantization: row.get(7)?,
                size_bytes: row.get::<_, Option<i64>>(8)?.and_then(|bytes| u64::try_from(bytes).ok()),
                tokens_per_second: row.get::<_, Option<i64>>(9)?.and_then(|speed| u64::try_from(speed).ok()),
                vram_bytes: row.get::<_, Option<i64>>(10)?.and_then(|bytes| u64::try_from(bytes).ok()),
            })
        })?;
        rows.collect()
    }

    #[cfg(windows)]
    pub fn cloud_tool_capability_verified(&self, vendor: openai::Vendor, model: &str) -> bool {
        self.db
            .query_row(
                "SELECT tool_calling FROM models WHERE provider = ?1 AND name = ?2",
                params![vendor.code(), model],
                |row| row.get::<_, String>(0),
            )
            .is_ok_and(|value| value == ToolCalling::Verified.code())
    }

    #[cfg(windows)]
    pub fn record_cloud_tool_capability(
        &self,
        vendor: openai::Vendor,
        model: &str,
        verified: bool,
    ) -> rusqlite::Result<()> {
        if !valid_model_name(model) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.db.execute(
            "INSERT INTO models (id, provider, name, tool_calling, vision, probed_at)
             VALUES (?1, ?2, ?3, ?4, 'not_advertised', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(id) DO UPDATE SET tool_calling = excluded.tool_calling,
             probed_at = excluded.probed_at, updated_at = excluded.probed_at",
            params![
                format!("{}:{model}", vendor.code()),
                vendor.code(),
                model,
                if verified { "verified" } else { "failed" }
            ],
        )?;
        Ok(())
    }

    #[cfg(windows)]
    pub fn record_cloud_probe_usage(&self, tokens: u64) -> rusqlite::Result<()> {
        self.audit(None, "model.usage", &format!("tokens:{tokens}"))
    }

    /// Local models whose last probe failed or that do not advertise tool calling: routing
    /// strategies never switch to them. An unreadable store excludes nothing.
    pub fn unusable_local_models(&self) -> Vec<String> {
        self.model_manifests()
            .map(|manifests| {
                manifests
                    .into_iter()
                    .filter(|manifest| {
                        matches!(manifest.tool_calling, ToolCalling::Failed | ToolCalling::NotAdvertised)
                    })
                    .map(|manifest| manifest.name)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// How interactive Ollama tasks pick their local model. Falls back to the older
    /// `routing.adaptive_local` switch; an unknown or unreadable value means `Manual`.
    pub fn local_strategy(&self) -> LocalStrategy {
        match self.setting("routing.local_strategy") {
            Ok(Some(code)) => LocalStrategy::from_code(&code).unwrap_or_default(),
            Ok(None) if matches!(self.setting("routing.adaptive_local"), Ok(Some(value)) if value == "true") => {
                LocalStrategy::Adaptive
            }
            _ => LocalStrategy::Manual,
        }
    }

    /// Stores the local routing strategy and journals the change (strategy code only).
    pub fn set_local_strategy(&mut self, strategy: LocalStrategy) -> Result<(), &'static str> {
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES ('routing.local_strategy', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [strategy.code()],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.routing.strategy.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), self.session_id, strategy.code()],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")
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

    /// Applications of the closed list the user forbade the agent to open (per-resource rule).
    /// Codes `notepad`, `charmap`, `paint`; unknown codes are ignored.
    pub fn blocked_applications(&self) -> Vec<&'static str> {
        let stored = self.setting("policy.blocked_applications").ok().flatten().unwrap_or_default();
        APPLICATION_CODES.into_iter().filter(|code| stored.split(',').any(|item| item == *code)).collect()
    }

    pub fn set_blocked_applications(&mut self, codes: &[String]) -> Result<(), &'static str> {
        if codes.iter().any(|code| !APPLICATION_CODES.contains(&code.as_str())) {
            return Err("SETTINGS_INVALID");
        }
        let kept: Vec<&str> =
            APPLICATION_CODES.into_iter().filter(|code| codes.iter().any(|item| item == code)).collect();
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('policy.blocked_applications', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [kept.join(",")],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.policy.applications.changed', 'core', ?3)",
                params![
                    Uuid::new_v4().to_string(),
                    session,
                    if kept.is_empty() { "none".to_owned() } else { kept.join("+") }
                ],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// File name patterns the agent may never read, list, search, copy, move or trash
    /// (ROADMAP "règles par ressource"). Checked by the file service at execution time.
    pub fn protected_files(&self) -> Vec<String> {
        self.patterns("policy.protected_files")
    }

    /// Replaces the protected file patterns; blank entries are ignored, duplicates removed.
    pub fn set_protected_files(&mut self, patterns: &[String]) -> Result<(), &'static str> {
        self.set_patterns("policy.protected_files", "settings.policy.protected.changed", patterns)
    }

    /// Window patterns, matched against the window title and the application's process name:
    /// the agent never sees, reads, captures, arranges or clicks in these windows.
    pub fn protected_windows(&self) -> Vec<String> {
        self.patterns("policy.protected_windows")
    }

    pub fn set_protected_windows(&mut self, patterns: &[String]) -> Result<(), &'static str> {
        self.set_patterns("policy.protected_windows", "settings.policy.windows.changed", patterns)
    }

    fn patterns(&self, key: &str) -> Vec<String> {
        self.setting(key)
            .ok()
            .flatten()
            .map(|value| {
                value
                    .lines()
                    .filter(|pattern| valid_pattern(pattern))
                    .take(MAX_PROTECTED_PATTERNS)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn set_patterns(&mut self, key: &str, kind: &str, patterns: &[String]) -> Result<(), &'static str> {
        let mut kept: Vec<String> = Vec::new();
        for pattern in patterns.iter().map(|pattern| pattern.trim()).filter(|pattern| !pattern.is_empty()) {
            if !valid_pattern(pattern) {
                return Err("SETTINGS_INVALID");
            }
            if !kept.iter().any(|known| known.eq_ignore_ascii_case(pattern)) {
                kept.push(pattern.to_owned());
            }
        }
        if kept.len() > MAX_PROTECTED_PATTERNS {
            return Err("SETTINGS_INVALID");
        }
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![key, kept.join("\n")],
            )?;
            // The patterns themselves stay out of the journal: only how many there are.
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, ?3, 'core', ?4)",
                params![Uuid::new_v4().to_string(), session, kind, format!("count:{}", kept.len())],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Local model to use when the chosen local model fails on the first step (T-009). `None`
    /// when unset or invalid. Local to local only: never a cloud model.
    pub fn local_fallback_model(&self) -> Option<String> {
        self.setting("routing.local_fallback_model").ok().flatten().filter(|name| valid_model_name(name))
    }

    /// Sets or clears (empty name) the local fallback model.
    pub fn set_local_fallback_model(&mut self, name: &str) -> Result<(), &'static str> {
        if !name.is_empty() && !valid_model_name(name) {
            return Err("MODEL_REQUIRED");
        }
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES ('routing.local_fallback_model', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [name],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.routing.fallback.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, if name.is_empty() { "none" } else { "local" }],
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
        if !self.cloud_vendor_enabled(vendor) {
            return Err("CLOUD_PROVIDER_DISABLED");
        }
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

    #[cfg(windows)]
    pub fn cloud_vendor_enabled(&self, vendor: openai::Vendor) -> bool {
        match self.setting(&format!("cloud.disabled.{}", vendor.code())) {
            Ok(None) => true,
            Ok(Some(value)) => value == "false",
            Err(_) => false,
        }
    }

    #[cfg(windows)]
    pub fn set_cloud_vendor_enabled(&mut self, vendor: openai::Vendor, enabled: bool) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET
             value = excluded.value, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![format!("cloud.disabled.{}", vendor.code()), if enabled { "false" } else { "true" }],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.cloud.provider.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, vendor.code()],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// The folder the user chose for the agent's file tools, if any (applied at startup).
    pub fn workspace(&self) -> Option<std::path::PathBuf> {
        self.setting("files.workspace").ok().flatten().filter(|path| !path.is_empty()).map(Into::into)
    }

    /// Extra folders the agent may work in, besides the primary one (applied at the next start).
    pub fn extra_folders(&self) -> Vec<std::path::PathBuf> {
        self.setting("files.extra_folders")
            .ok()
            .flatten()
            .map(|value| {
                value.lines().filter(|line| !line.is_empty()).take(folders::MAX_EXTRA_FOLDERS).map(Into::into).collect()
            })
            .unwrap_or_default()
    }

    /// Stores the extra folders. Each path must already be validated by
    /// `LocalFiles::validate_workspace`; only their number is journaled.
    pub fn set_extra_folders(&mut self, paths: &[String]) -> Result<(), &'static str> {
        if paths.len() > folders::MAX_EXTRA_FOLDERS || paths.iter().any(|path| path.is_empty() || path.contains('\n')) {
            return Err("SETTINGS_INVALID");
        }
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('files.extra_folders', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [paths.join("\n")],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.db
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.folders.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), self.session_id, format!("count:{}", paths.len())],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        Ok(())
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

    /// Cloud price and per-task cost cap; unreadable or out-of-range values mean "not set".
    pub fn cloud_cost(&self) -> CloudCost {
        let read = |key: &str| {
            self.setting(key)
                .ok()
                .flatten()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|amount| *amount <= MAX_COST_MILLI)
                .unwrap_or(0)
        };
        CloudCost {
            price_milli_per_million: read("routing.cloud_price_milli"),
            max_milli_per_task: read("routing.cloud_max_cost_milli"),
        }
    }

    pub fn set_cloud_cost(&mut self, cost: CloudCost) -> Result<(), &'static str> {
        if cost.price_milli_per_million > MAX_COST_MILLI || cost.max_milli_per_task > MAX_COST_MILLI {
            return Err("SETTINGS_INVALID");
        }
        let session = self.session_id.clone();
        let write = |db: &mut Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            for (key, value) in [
                ("routing.cloud_price_milli", cost.price_milli_per_million),
                ("routing.cloud_max_cost_milli", cost.max_milli_per_task),
            ] {
                transaction.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                    params![key, value.to_string()],
                )?;
            }
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.cost.changed', 'core', ?3)",
                params![
                    Uuid::new_v4().to_string(),
                    session,
                    format!("price:{}+max:{}", cost.price_milli_per_million, cost.max_milli_per_task)
                ],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Retained cumulative cloud counters, counted once per task across sessions.
    pub fn cloud_usage(&self) -> rusqlite::Result<CloudUsage> {
        self.db.query_row(
            "SELECT COALESCE(SUM(tokens), 0), COUNT(*) FROM (
                SELECT MAX(CAST(substr(code, 8) AS INTEGER)) AS tokens
                FROM events
                WHERE kind = 'model.usage' AND task_id IS NOT NULL
                    AND timestamp >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days')
                    AND code GLOB 'tokens:[0-9]*'
                    AND substr(code, 8) NOT GLOB '*[^0-9]*'
                GROUP BY task_id
            )",
            [],
            |row| Ok(CloudUsage { tokens: row.get(0)?, tasks: row.get(1)? }),
        )
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
                let kind: String = row.get(4)?;
                let code: Option<String> = row.get(6)?;
                let payload = audit_payload(&kind, code.as_deref());
                Ok(AuditEntry {
                    sequence: row.get(0)?,
                    id: row.get(1)?,
                    session_id: row.get(2)?,
                    task_id: row.get(3)?,
                    kind,
                    source: row.get(5)?,
                    code,
                    timestamp: row.get(7)?,
                    payload,
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
    /// Journals the time from process start to the first rendered interface (T-001), once
    /// per launch. A duration only.
    pub fn record_ui_ready(&self, elapsed_ms: u64) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'ui.ready', 'desktop', ?3)",
            params![Uuid::new_v4().to_string(), self.session_id, format!("ms:{elapsed_ms}")],
        )?;
        Ok(())
    }

    /// Records render latency only. Page names and user content stay out of the journal.
    pub fn record_ui_navigation(&self, elapsed_ms: u32) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'ui.navigation', 'desktop', ?3)",
            params![Uuid::new_v4().to_string(), self.session_id, format!("ms:{elapsed_ms}")],
        )?;
        Ok(())
    }

    pub fn record_palette_ready(&self, elapsed_ms: u32) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'ui.palette.ready', 'desktop', ?3)",
            params![Uuid::new_v4().to_string(), self.session_id, format!("ms:{elapsed_ms}")],
        )?;
        Ok(())
    }

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
        read_activity(&self.db, &self.session_id, None)
    }
}

/// Reads committed audit records without starting a new core session or running migrations.
pub fn read_committed_activity(path: &Path, session: &str, after: Option<u64>) -> rusqlite::Result<Vec<Activity>> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_secs(2))?;
    read_activity(&db, session, after)
}

fn read_activity(db: &Connection, session: &str, after: Option<u64>) -> rusqlite::Result<Vec<Activity>> {
    let sql = if after.is_some() {
        "SELECT sequence, kind, timestamp, task_id, code FROM events
         WHERE session_id = ?1 AND sequence > ?2 ORDER BY sequence LIMIT 100"
    } else {
        "SELECT sequence, kind, timestamp, task_id, code FROM events
         WHERE session_id = ?1 AND ?2 IS NULL ORDER BY sequence DESC LIMIT 100"
    };
    let mut statement = db.prepare(sql)?;
    let events = statement.query_map(params![session, after], |row| {
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
    fn cloud_usage_counts_each_task_once_and_excludes_old_or_invalid_counters() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        for (task, code, days) in [
            ("a", "tokens:100", 0),
            ("a", "tokens:400", -1),
            ("b", "tokens:200", 0),
            ("c", "tokens:900", -31),
            ("d", "tokens:900garbage", 0),
        ] {
            core.db
                .execute(
                    "INSERT INTO events (id, session_id, task_id, kind, source, code, timestamp)
                 VALUES (?1, ?2, ?3, 'model.usage', 'core', ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?5))",
                    params![Uuid::new_v4().to_string(), core.session_id, task, code, format!("{days} days")],
                )
                .unwrap();
        }
        let usage = core.cloud_usage().unwrap();
        assert_eq!((usage.tokens, usage.tasks), (600, 2));
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
    fn a_failed_task_worker_can_be_replaced_within_the_same_session() {
        let path = std::env::temp_dir().join(format!("aiwin11-worker-recovery-{}.sqlite3", Uuid::new_v4()));
        let mut desktop = Core::open(&path).unwrap();
        let session = desktop.session_id().to_owned();
        let worker = Core::attach_task_worker(&path, &session, std::process::id()).unwrap();
        worker.task_created("first").unwrap();
        worker.task_state("first", "dispatching", None).unwrap();
        drop(worker);

        desktop.recover_task_worker().unwrap();
        desktop.recover_task_worker().unwrap();
        let replacement = Core::attach_task_worker(&path, &session, std::process::id()).unwrap();
        replacement.task_created("second").unwrap();
        let tasks = desktop.recent_tasks(10).unwrap();
        let first = tasks.iter().find(|task| task.id == "first").unwrap();
        assert_eq!(first.state, "interrupted");
        assert_eq!(first.outcome.as_deref(), Some("dispatching"));
        assert_eq!(tasks.iter().find(|task| task.id == "second").unwrap().state, "created");
        assert_eq!(desktop.activity().unwrap().iter().filter(|event| event.kind == "task.recovered").count(), 1);
        drop(replacement);
        drop(desktop);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn audit_export_adds_only_recognized_structured_payloads() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        core.journal("task.progress", Some("step:2/16")).unwrap();
        core.journal("model.provider.unavailable", Some("unrecognized")).unwrap();
        core.record_ui_navigation(32).unwrap();
        let events = serde_json::to_value(core.export_audit().unwrap()).unwrap();
        assert_eq!(events[1]["payload"], serde_json::json!({"type":"taskStep","current":2,"total":16}));
        assert!(events[2].get("payload").is_none());
        assert_eq!(events[3]["payload"], serde_json::json!({"type":"renderTime","elapsedMs":32}));
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
            "ask:launch_application:profile:medium",
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
            "memory_entries.last_run",
            "memory_entries.schedule",
            // Vector index: DPAPI ciphertext, the entry id and a local model name.
            "memory_vectors.entry_id",
            "memory_vectors.model",
            "memory_vectors.vector",
            // Model manifests: public model identifiers, status codes, a duration, timestamps.
            "models.context_length",
            "models.id",
            "models.name",
            "models.parameter_size",
            "models.probe_ms",
            "models.probed_at",
            "models.provider",
            "models.quantization",
            "models.size_bytes",
            "models.tokens_per_second",
            "models.tool_calling",
            "models.updated_at",
            "models.vision",
            "models.vram_bytes",
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
    fn local_strategy_defaults_to_manual_migrates_the_old_switch_and_persists() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.local_strategy(), LocalStrategy::Manual);
        core.db.execute("INSERT INTO settings (key, value) VALUES ('routing.adaptive_local', 'true')", []).unwrap();
        assert_eq!(core.local_strategy(), LocalStrategy::Adaptive, "the older switch is honoured");
        core.set_local_strategy(LocalStrategy::QualityFirst).unwrap();
        assert_eq!(core.local_strategy(), LocalStrategy::QualityFirst, "the new setting wins");
        let event = &core.activity().unwrap()[0];
        assert_eq!(
            (event.kind.as_str(), event.code.as_deref()),
            ("settings.routing.strategy.changed", Some("quality_first"))
        );
        core.db.execute("UPDATE settings SET value = 'invalid' WHERE key = 'routing.local_strategy'", []).unwrap();
        assert_eq!(core.local_strategy(), LocalStrategy::Manual, "unknown codes fall back to manual");
    }

    #[test]
    fn local_profiles_and_provider_capabilities_are_scoped() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        core.set_agent_models(AgentModels { office: "local:office".into(), ..Default::default() }).unwrap();
        assert_eq!(core.agent_models().for_profile(policy::AgentProfile::Office), Some("local:office"));
        assert_eq!(core.agent_models().for_profile(policy::AgentProfile::General), None);
        assert_eq!(
            core.set_agent_models(AgentModels { office: "not a model".into(), ..Default::default() }),
            Err("SETTINGS_INVALID")
        );
        core.record_cloud_tool_capability(openai::Vendor::DeepSeek, "fixture", true).unwrap();
        assert!(core.cloud_tool_capability_verified(openai::Vendor::DeepSeek, "fixture"));
        assert!(!core.cloud_tool_capability_verified(openai::Vendor::OpenAi, "fixture"));
        core.record_cloud_tool_capability(openai::Vendor::DeepSeek, "fixture", false).unwrap();
        assert!(!core.cloud_tool_capability_verified(openai::Vendor::DeepSeek, "fixture"));
        assert!(core.model_manifests().unwrap().is_empty());
        core.set_local_strategy(LocalStrategy::LocalFirst).unwrap();
        assert!(core.cloud_fallback_enabled());
        core.set_local_strategy(LocalStrategy::LocalOnly).unwrap();
        assert!(!core.cloud_fallback_enabled());
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
    fn protected_file_patterns_are_validated_and_matched_case_insensitively() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert!(core.protected_files().is_empty());
        let patterns: Vec<String> = ["contrat*.txt", " ", "CONTRAT*.TXT", "secret?.txt"].map(String::from).into();
        core.set_protected_files(&patterns).unwrap();
        assert_eq!(core.protected_files(), ["contrat*.txt", "secret?.txt"]);
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("count:2"));
        for bad in ["../x", "a\\b", "x\u{0}", &"m".repeat(65)] {
            assert_eq!(core.set_protected_files(&[bad.to_owned()]), Err("SETTINGS_INVALID"), "{bad:?}");
        }
        let too_many: Vec<String> = (0..=MAX_PROTECTED_PATTERNS).map(|index| format!("f{index}")).collect();
        assert_eq!(core.set_protected_files(&too_many), Err("SETTINGS_INVALID"));
        assert!(matches_pattern("contrat*.txt", "Contrat-2026.TXT"));
        assert!(matches_pattern("secret?.txt", "secret1.txt"));
        assert!(!matches_pattern("secret?.txt", "secret12.txt"));
        assert!(matches_pattern("*", "x.txt"));
        assert!(!matches_pattern("*.md", "notes.txt"));
        assert!(matches_pattern("a*b*c", "aXXbYYc"));
        assert!(!matches_pattern("a*b*c", "aXXbYY"));
    }

    #[test]
    fn protected_window_patterns_are_stored_apart_from_file_patterns() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        core.set_protected_windows(&["*Bitwarden*".to_owned(), "keepass*".to_owned()]).unwrap();
        assert_eq!(core.protected_windows(), ["*Bitwarden*", "keepass*"]);
        assert!(core.protected_files().is_empty());
        let event = &core.activity().unwrap()[0];
        assert_eq!((event.kind.as_str(), event.code.as_deref()), ("settings.policy.windows.changed", Some("count:2")));
    }

    #[test]
    fn blocked_applications_are_codes_of_the_closed_list() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert!(core.blocked_applications().is_empty());
        core.set_blocked_applications(&["paint".to_owned(), "notepad".to_owned()]).unwrap();
        assert_eq!(core.blocked_applications(), ["notepad", "paint"]);
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("notepad+paint"));
        assert_eq!(core.set_blocked_applications(&["cmd".to_owned()]), Err("SETTINGS_INVALID"));
        core.set_blocked_applications(&[]).unwrap();
        assert!(core.blocked_applications().is_empty());
    }

    #[test]
    fn ui_ready_records_a_duration_only() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        core.record_ui_ready(1_234).unwrap();
        let event = &core.activity().unwrap()[0];
        assert_eq!((event.kind.as_str(), event.code.as_deref()), ("ui.ready", Some("ms:1234")));
    }

    #[test]
    fn local_fallback_model_is_a_valid_identifier_or_nothing() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.local_fallback_model(), None);
        core.set_local_fallback_model("qwen3:4b").unwrap();
        assert_eq!(core.local_fallback_model().as_deref(), Some("qwen3:4b"));
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("local"));
        assert_eq!(core.set_local_fallback_model("a b"), Err("MODEL_REQUIRED"));
        assert_eq!(core.local_fallback_model().as_deref(), Some("qwen3:4b"), "unchanged after a refusal");
        core.set_local_fallback_model("").unwrap();
        assert_eq!(core.local_fallback_model(), None);
    }

    #[test]
    fn cloud_cost_is_user_entered_bounded_and_estimated_rounding_up() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(core.cloud_cost(), CloudCost::default(), "nothing set by default");
        let too_high = CloudCost { price_milli_per_million: MAX_COST_MILLI + 1, max_milli_per_task: 0 };
        assert_eq!(core.set_cloud_cost(too_high), Err("SETTINGS_INVALID"));
        // 0.150 per million tokens, at most 0.010 per task.
        let cost = CloudCost { price_milli_per_million: 150, max_milli_per_task: 10 };
        core.set_cloud_cost(cost).unwrap();
        assert_eq!(core.cloud_cost(), cost);
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("price:150+max:10"));
        assert_eq!(cost.estimate(0), 0);
        assert_eq!(cost.estimate(1), 1, "rounded up");
        assert_eq!(cost.estimate(60_000), 9);
        assert!(!cost.exceeded(60_000));
        assert!(cost.exceeded(66_667));
        assert!(!CloudCost { max_milli_per_task: 0, ..cost }.exceeded(u64::MAX), "no cap");
        assert!(!CloudCost { price_milli_per_million: 0, ..cost }.exceeded(u64::MAX), "no price");
        assert_eq!(
            CloudCost { price_milli_per_million: MAX_COST_MILLI, ..cost }.estimate(u64::MAX),
            u64::MAX,
            "no overflow"
        );
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
    fn model_manifests_record_probe_results_and_exclude_unusable_models() {
        let core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        assert!(core.model_manifests().unwrap().is_empty());
        let qwen_facts = ModelFacts {
            context_length: Some(40_960),
            parameter_size: Some("8.2B".into()),
            quantization: Some("Q4_K_M".into()),
            size_bytes: Some(5_225_387_923),
            tokens_per_second: Some(95),
            vram_bytes: Some(6_400_000_000),
            ..ModelFacts::default()
        };
        core.record_model_probe("qwen3:8b", ToolCalling::Verified, Some(&qwen_facts), 1_200).unwrap();
        core.record_model_probe("qwen3:0.6b", ToolCalling::Failed, Some(&ModelFacts::default()), 900).unwrap();
        core.record_model_probe(
            "gemma3:4b",
            ToolCalling::NotAdvertised,
            Some(&ModelFacts { vision_advertised: true, ..ModelFacts::default() }),
            0,
        )
        .unwrap();
        core.record_model_probe("gemma3:4b", ToolCalling::NotAdvertised, None, 0).unwrap();
        core.record_model_probe("qwen3:8b", ToolCalling::Verified, None, 800).unwrap();
        let manifests = core.model_manifests().unwrap();
        assert_eq!(manifests.len(), 3, "one row per model");
        let qwen = manifests.iter().find(|manifest| manifest.name == "qwen3:8b").unwrap();
        assert_eq!((qwen.tool_calling, qwen.probe_ms), (ToolCalling::Verified, Some(800)));
        // A routing update (`None`) keeps the limits and hardware of the last full probe.
        assert_eq!(
            (qwen.context_length, qwen.parameter_size.as_deref(), qwen.quantization.as_deref(), qwen.size_bytes),
            (Some(40_960), Some("8.2B"), Some("Q4_K_M"), Some(5_225_387_923))
        );
        assert_eq!((qwen.tokens_per_second, qwen.vram_bytes), (Some(95), Some(6_400_000_000)));
        // `None` keeps what the last full probe said about vision.
        assert!(manifests.iter().find(|manifest| manifest.name == "gemma3:4b").unwrap().vision_advertised);
        let mut unusable = core.unusable_local_models();
        unusable.sort();
        assert_eq!(unusable, ["gemma3:4b", "qwen3:0.6b"]);
        for invalid in ["", "a b", "x\u{0}", &"m".repeat(257)] {
            assert_eq!(core.record_model_probe(invalid, ToolCalling::Verified, None, 1), Err("MODEL_REQUIRED"));
        }
        let event = &core.activity().unwrap()[0];
        assert_eq!((event.kind.as_str(), event.code.as_deref()), ("model.manifest.updated", Some("verified")));
        for status in [ToolCalling::Unverified, ToolCalling::NotAdvertised, ToolCalling::Verified, ToolCalling::Failed]
        {
            assert_eq!(ToolCalling::from_code(status.code()), Some(status));
        }
    }

    #[test]
    fn a_newer_database_is_rejected() {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "user_version", 11).unwrap();
        assert!(Core::start(db).is_err());
    }

    #[test]
    fn a_version_1_database_is_migrated_in_place() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/001.sql")).unwrap();
        db.execute("INSERT INTO sessions (id) VALUES ('old')", []).unwrap();
        let core = Core::start(db).unwrap();
        let version: u32 = core.db.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as u32);
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
