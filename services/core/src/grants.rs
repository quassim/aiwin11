//! Temporary, revocable permissions ("autorisations temporaires", ROADMAP R2).
//!
//! The user can let one action run without asking for a limited time. A grant only turns an
//! `Ask` from the profile into `Allow`: it never relaxes a denial, a critical action, screen
//! reading, the local-only rule or simulation mode (see `policy::evaluate`). Grants are
//! checked at each evaluation, just before execution, so a revocation or an expiry takes
//! effect for the very next action.

use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    policy::{grantable, AgentAction},
    Core,
};

/// Accepted grant durations, in minutes.
pub const GRANT_MINUTES: std::ops::RangeInclusive<u32> = 1..=480;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub id: String,
    pub action: AgentAction,
    pub created_at: String,
    pub expires_at: String,
}

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

impl Core {
    /// Lets `action` run without asking for `minutes`. Refused for actions a grant cannot
    /// relax (reads, critical actions, screen reading) and for out-of-range durations.
    pub fn grant_temporarily(&mut self, action: AgentAction, minutes: u32) -> Result<String, &'static str> {
        if !grantable(action) {
            return Err("GRANT_NOT_ALLOWED");
        }
        if !GRANT_MINUTES.contains(&minutes) {
            return Err("GRANT_INVALID");
        }
        let id = Uuid::new_v4().to_string();
        let session_id = self.session_id.clone();
        let create = |db: &mut rusqlite::Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO grants (id, action, expires_at)
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?3))",
                params![id, action.code(), format!("+{minutes} minutes")],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'grant.created', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session_id, format!("{}:{minutes}min", action.code())],
            )?;
            transaction.commit()
        };
        create(&mut self.db).map_err(|_| "GRANT_UNAVAILABLE")?;
        Ok(id)
    }

    /// Revokes a grant at once. Unknown, expired or already revoked grants are reported.
    pub fn revoke_grant(&mut self, id: &str) -> Result<(), &'static str> {
        let session_id = self.session_id.clone();
        let revoke = |db: &mut rusqlite::Connection| -> rusqlite::Result<Option<String>> {
            let transaction = db.transaction()?;
            let action: Option<String> = transaction
                .query_row(
                    &format!("SELECT action FROM grants WHERE id = ?1 AND revoked_at IS NULL AND expires_at > {NOW}"),
                    [id],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    error => Err(error),
                })?;
            let Some(action) = action else { return Ok(None) };
            transaction.execute(&format!("UPDATE grants SET revoked_at = {NOW} WHERE id = ?1"), [id])?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code) VALUES (?1, ?2, 'grant.revoked', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session_id, crate::journal_code(&action)],
            )?;
            transaction.commit()?;
            Ok(Some(action))
        };
        match revoke(&mut self.db) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err("GRANT_NOT_ACTIVE"),
            Err(_) => Err("GRANT_UNAVAILABLE"),
        }
    }

    /// Grants still in force, soonest expiry first.
    pub fn active_grants(&self) -> Result<Vec<Grant>, &'static str> {
        let read = || -> rusqlite::Result<Vec<(String, String, String, String)>> {
            let mut statement = self.db.prepare(&format!(
                "SELECT id, action, created_at, expires_at FROM grants
                 WHERE revoked_at IS NULL AND expires_at > {NOW} ORDER BY expires_at"
            ))?;
            let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
            rows.collect()
        };
        let rows = read().map_err(|_| "GRANT_UNAVAILABLE")?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, action, created_at, expires_at)| {
                AgentAction::from_code(&action).map(|action| Grant { id, action, created_at, expires_at })
            })
            .collect())
    }

    /// True if a grant for `action` is in force now. A storage error means no grant.
    pub fn has_active_grant(&self, action: AgentAction) -> bool {
        grantable(action)
            && self
                .db
                .query_row(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM grants WHERE action = ?1
                         AND revoked_at IS NULL AND expires_at > {NOW})"
                    ),
                    [action.code()],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn core() -> Core {
        Core::start(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn grants_are_limited_to_relaxable_actions_and_durations() {
        let mut core = core();
        for action in
            [AgentAction::TrashFile, AgentAction::InspectWindow, AgentAction::DescribeWindow, AgentAction::ReadFile]
        {
            assert_eq!(core.grant_temporarily(action, 10), Err("GRANT_NOT_ALLOWED"), "{action:?}");
        }
        assert_eq!(core.grant_temporarily(AgentAction::CopyFile, 0), Err("GRANT_INVALID"));
        assert_eq!(core.grant_temporarily(AgentAction::CopyFile, 481), Err("GRANT_INVALID"));
        assert!(core.active_grants().unwrap().is_empty());
    }

    #[test]
    fn a_grant_is_active_until_revoked_or_expired() {
        let mut core = core();
        assert!(!core.has_active_grant(AgentAction::CopyFile));
        let id = core.grant_temporarily(AgentAction::CopyFile, 30).unwrap();
        assert!(core.has_active_grant(AgentAction::CopyFile));
        assert!(!core.has_active_grant(AgentAction::MoveFile), "a grant covers one action only");
        assert_eq!(core.active_grants().unwrap().len(), 1);
        assert_eq!(core.activity().unwrap()[0].code.as_deref(), Some("copy_file:30min"));

        core.revoke_grant(&id).unwrap();
        assert!(!core.has_active_grant(AgentAction::CopyFile), "revocation is effective at once");
        assert_eq!(core.revoke_grant(&id), Err("GRANT_NOT_ACTIVE"));
        assert_eq!(core.activity().unwrap()[0].kind, "grant.revoked");

        let id = core.grant_temporarily(AgentAction::LaunchApplication, 5).unwrap();
        core.db.execute("UPDATE grants SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1", [&id]).unwrap();
        assert!(!core.has_active_grant(AgentAction::LaunchApplication), "an expired grant is ignored");
        assert!(core.active_grants().unwrap().is_empty());
    }
}
