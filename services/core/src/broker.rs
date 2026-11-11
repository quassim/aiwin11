use std::time::{SystemTime, UNIX_EPOCH};

use aiwin11_contracts::{
    v1::{command::Operation, launch_application::Application, Command},
    SCHEMA_VERSION,
};
use rusqlite::params;
use uuid::Uuid;

use crate::Core;

/// Constructed by the trusted host after authenticating the transport peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    Agent,
    DesktopInspector,
    Other,
}

#[derive(Clone, Copy, Default)]
pub struct Policy {
    pub allow_agent_to_launch_notepad: bool,
    /// Lets the agent list and read text files in the dedicated local folder.
    /// The host grants it only for tasks served by a local model.
    pub allow_agent_file_reads: bool,
    /// Lets the agent copy one text file inside the dedicated folder. The host grants it
    /// only after the user approved that exact copy.
    pub allow_agent_file_copy: bool,
    /// Lets the agent identify the window the user was using before AiWin11 (title and
    /// application only). Granted for local-model tasks.
    pub allow_agent_window_lookup: bool,
    /// Lets the agent read one window's control tree. Granted only after the user approved
    /// reading that exact window.
    pub allow_agent_window_inspection: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExecutionError {
    Denied(&'static str),
    AuditUnavailable,
    ExecutionFailed,
    WindowUnverified,
    OutcomeUnrecorded,
}

pub(crate) enum LaunchOutcome {
    Verified(u32),
    Unverified,
    Failed,
}

impl Policy {
    fn authorize(self, peer: Peer, command: &Command, session: &str) -> Result<&'static str, &'static str> {
        if command.schema_version != SCHEMA_VERSION {
            return Err("unsupported_schema");
        }
        if command.session_id != session || peer == Peer::Other {
            return Err("invalid_scope");
        }
        for id in [
            &command.request_id,
            &command.task_id,
            &command.correlation_id,
            &command.idempotency_key,
        ] {
            if !matches!(Uuid::parse_str(id), Ok(uuid) if uuid.to_string() == *id) {
                return Err("invalid_identifier");
            }
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "clock_unavailable")?
            .as_millis() as i64;
        if command.deadline_ms <= now {
            return Err("expired");
        }
        match &command.operation {
            Some(Operation::LaunchApplication(launch))
                if launch.application == Application::Notepad as i32
                    && self.allow_agent_to_launch_notepad && peer == Peer::Agent =>
            {
                Ok("launch_notepad")
            }
            Some(Operation::InspectForeground(_)) if peer == Peer::DesktopInspector => Ok("inspect_foreground"),
            Some(Operation::LocateWindow(_)) if peer == Peer::Agent && self.allow_agent_window_lookup => {
                Ok("locate_window")
            }
            Some(Operation::InspectWindow(_)) if peer == Peer::Agent && self.allow_agent_window_inspection => {
                Ok("inspect_window")
            }
            Some(Operation::CaptureForeground(_)) if peer == Peer::DesktopInspector => Ok("capture_foreground"),
            Some(Operation::AnalyzeCapture(_)) if peer == Peer::DesktopInspector => Ok("analyze_capture"),
            Some(Operation::PreviewLocalText(_))
                if peer == Peer::DesktopInspector || (peer == Peer::Agent && self.allow_agent_file_reads) =>
            {
                Ok("preview_local_text")
            }
            Some(Operation::ListLocalText(_))
                if peer == Peer::DesktopInspector || (peer == Peer::Agent && self.allow_agent_file_reads) =>
            {
                Ok("list_local_text")
            }
            Some(Operation::CopyLocalText(_))
                if peer == Peer::DesktopInspector || (peer == Peer::Agent && self.allow_agent_file_copy) =>
            {
                Ok("copy_local_text")
            }
            Some(Operation::MoveLocalText(_)) if peer == Peer::DesktopInspector => Ok("move_local_text"),
            Some(Operation::DeleteLocalText(input)) if peer == Peer::DesktopInspector => {
                if input.confirmed { Ok("delete_local_text") } else { Err("confirmation_required") }
            }
            Some(Operation::CharacterMapDigit(input))
                if peer == Peer::DesktopInspector && input.digit.is_some_and(|digit| digit <= 9) =>
            {
                Ok("character_map_digit")
            }
            _ => Err("default_deny"),
        }
    }
}

impl Core {
    /// The host supplies a fixed Notepad executor. The request supplies no path or arguments.
    pub(crate) fn execute(
        &mut self,
        policy: Policy,
        peer: Peer,
        command: &Command,
        launch_notepad: impl FnOnce() -> LaunchOutcome,
    ) -> Result<u32, ExecutionError> {
        let task_id = self.authorize_dispatch(policy, peer, command)?;
        let (kind, code, result) = match launch_notepad() {
            LaunchOutcome::Verified(pid) => ("tool.execution.completed", "success", Ok(pid)),
            LaunchOutcome::Unverified => (
                "tool.execution.partial",
                "window_unverified",
                Err(ExecutionError::WindowUnverified),
            ),
            LaunchOutcome::Failed => (
                "tool.execution.failed",
                "launch_failed",
                Err(ExecutionError::ExecutionFailed),
            ),
        };
        self.audit(task_id.as_deref(), kind, code)
            .map_err(|_| ExecutionError::OutcomeUnrecorded)?;
        result
    }

    pub(crate) fn authorize_dispatch(
        &mut self, policy: Policy, peer: Peer, command: &Command,
    ) -> Result<Option<String>, ExecutionError> {
        let task_id = Uuid::parse_str(&command.task_id).ok().map(|id| id.to_string());
        let operation = match policy.authorize(peer, command, self.session_id()) {
            Ok(operation) => operation,
            Err(code) => {
                self.audit(task_id.as_deref(), "policy.denied", code)
                    .map_err(|_| ExecutionError::AuditUnavailable)?;
                return Err(ExecutionError::Denied(code));
            }
        };

        let transaction = self.db.transaction().map_err(|_| ExecutionError::AuditUnavailable)?;
        let reserved = transaction
            .execute(
                "INSERT INTO tool_executions (session_id, idempotency_key, task_id)
                 VALUES (?1, ?2, ?3) ON CONFLICT(session_id, idempotency_key) DO NOTHING",
                params![self.session_id, command.idempotency_key, task_id],
            )
            .map_err(|_| ExecutionError::AuditUnavailable)?;
        let (kind, code) = if reserved == 0 {
            ("policy.denied", "duplicate_request")
        } else {
            ("policy.allowed", operation)
        };
        transaction
            .execute(
                "INSERT INTO events (id, session_id, task_id, kind, source, code)
                 VALUES (?1, ?2, ?3, ?4, 'broker', ?5)",
                params![Uuid::new_v4().to_string(), self.session_id, task_id, kind, code],
            )
            .map_err(|_| ExecutionError::AuditUnavailable)?;
        transaction.commit().map_err(|_| ExecutionError::AuditUnavailable)?;
        if reserved == 0 {
            return Err(ExecutionError::Denied(code));
        }

        if let Err(code) = policy.authorize(peer, command, self.session_id()) {
            self.audit(task_id.as_deref(), "policy.denied", code)
                .map_err(|_| ExecutionError::AuditUnavailable)?;
            return Err(ExecutionError::Denied(code));
        }
        Ok(task_id)
    }

    pub(crate) fn audit(&self, task_id: Option<&str>, kind: &str, code: &str) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, task_id, kind, source, code)
             VALUES (?1, ?2, ?3, ?4, 'broker', ?5)",
            params![Uuid::new_v4().to_string(), self.session_id, task_id, kind, code],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwin11_contracts::v1::LaunchApplication;
    use rusqlite::Connection;

    fn request(core: &Core) -> Command {
        Command {
            schema_version: SCHEMA_VERSION,
            request_id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
            session_id: core.session_id().into(),
            authenticated_actor_id: "agent".into(),
            deadline_ms: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
                + 60_000) as i64,
            operation: Some(Operation::LaunchApplication(LaunchApplication {
                application: Application::Notepad as i32,
            })),
        }
    }

    #[test]
    fn character_map_input_requires_the_desktop_role_and_a_decimal_digit() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        let operation = |digit| Operation::CharacterMapDigit(aiwin11_contracts::v1::CharacterMapDigit { digit });
        command.operation = Some(operation(Some(7)));
        assert_eq!(
            core.authorize_dispatch(Policy::default(), Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny"))
        );
        for invalid in [None, Some(10)] {
            command.operation = Some(operation(invalid));
            assert_eq!(
                core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command),
                Err(ExecutionError::Denied("default_deny"))
            );
        }
        command.operation = Some(operation(Some(0)));
        assert!(core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn default_denies_and_audits_without_dispatch() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let command = request(&core);
        assert_eq!(
            core.execute(Policy::default(), Peer::Agent, &command, || panic!("dispatched")),
            Err(ExecutionError::Denied("default_deny"))
        );
        assert_eq!(core.activity().unwrap()[0].kind, "policy.denied");
    }

    #[test]
    fn only_the_desktop_inspector_can_authorize_inspection() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        command.operation = Some(Operation::InspectForeground(Default::default()));
        assert_eq!(
            core.authorize_dispatch(Policy::default(), Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny"))
        );
        assert!(core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn capture_is_denied_to_the_agent_even_with_launch_permission() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        command.operation = Some(Operation::CaptureForeground(Default::default()));
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        assert_eq!(
            core.authorize_dispatch(policy, Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny"))
        );
        assert!(core.authorize_dispatch(policy, Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn file_copy_is_denied_to_the_agent_even_with_launch_permission() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        command.operation = Some(Operation::CopyLocalText(Default::default()));
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        assert_eq!(
            core.authorize_dispatch(policy, Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny"))
        );
        assert!(core.authorize_dispatch(policy, Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn agent_file_reads_require_their_own_grant_and_never_extend_to_writes() {
        let reads = [
            Operation::PreviewLocalText(Default::default()),
            Operation::ListLocalText(Default::default()),
        ];
        let launch_only = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        let reads_only = Policy { allow_agent_file_reads: true, ..Default::default() };
        for operation in reads {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(operation);
            assert_eq!(
                core.authorize_dispatch(launch_only, Peer::Agent, &command),
                Err(ExecutionError::Denied("default_deny"))
            );
            command.idempotency_key = Uuid::new_v4().to_string();
            assert!(core.authorize_dispatch(reads_only, Peer::Agent, &command).is_ok());
        }
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        for write in [
            Operation::CopyLocalText(Default::default()),
            Operation::MoveLocalText(Default::default()),
            Operation::DeleteLocalText(aiwin11_contracts::v1::DeleteLocalText { confirmed: true }),
        ] {
            let mut command = request(&core);
            command.operation = Some(write);
            assert_eq!(
                core.authorize_dispatch(reads_only, Peer::Agent, &command),
                Err(ExecutionError::Denied("default_deny"))
            );
        }
        let mut command = request(&core);
        assert_eq!(
            core.authorize_dispatch(reads_only, Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny")),
            "file reads do not grant the Notepad launch"
        );
        command.operation = Some(Operation::ListLocalText(Default::default()));
        assert_eq!(
            core.authorize_dispatch(reads_only, Peer::Other, &command),
            Err(ExecutionError::Denied("invalid_scope"))
        );
    }

    #[test]
    fn window_lookup_and_inspection_are_separate_agent_grants() {
        let lookup = Policy { allow_agent_window_lookup: true, ..Default::default() };
        let inspection = Policy { allow_agent_window_inspection: true, ..Default::default() };
        let everything_else = Policy {
            allow_agent_to_launch_notepad: true, allow_agent_file_reads: true, allow_agent_file_copy: true,
            ..Default::default()
        };
        let locate = Operation::LocateWindow(Default::default());
        let inspect = Operation::InspectWindow(Default::default());
        for (operation, allowed, denied) in [(locate, lookup, inspection), (inspect, inspection, lookup)] {
            for (policy, peer, expected) in [
                (allowed, Peer::Agent, true),
                (denied, Peer::Agent, false),
                (everything_else, Peer::Agent, false),
                (allowed, Peer::DesktopInspector, false),
            ] {
                let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
                let mut command = request(&core);
                command.operation = Some(operation);
                assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{operation:?} {peer:?}");
            }
        }
    }

    #[test]
    fn agent_copy_grant_covers_copy_only() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let copy_only = Policy { allow_agent_file_copy: true, ..Default::default() };
        let mut command = request(&core);
        command.operation = Some(Operation::CopyLocalText(Default::default()));
        assert_eq!(
            core.authorize_dispatch(Policy { allow_agent_file_reads: true, ..Default::default() }, Peer::Agent, &command),
            Err(ExecutionError::Denied("default_deny")),
            "reading does not imply copying"
        );
        command.idempotency_key = Uuid::new_v4().to_string();
        assert!(core.authorize_dispatch(copy_only, Peer::Agent, &command).is_ok());
        for other in [
            Operation::MoveLocalText(Default::default()),
            Operation::DeleteLocalText(aiwin11_contracts::v1::DeleteLocalText { confirmed: true }),
            Operation::PreviewLocalText(Default::default()),
        ] {
            let mut command = request(&core);
            command.operation = Some(other);
            assert_eq!(
                core.authorize_dispatch(copy_only, Peer::Agent, &command),
                Err(ExecutionError::Denied("default_deny"))
            );
        }
    }

    #[test]
    fn trusts_transport_identity_and_deduplicates_dispatch() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let command = request(&core);
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        assert_eq!(
            core.execute(policy, Peer::Other, &command, || panic!("dispatched")),
            Err(ExecutionError::Denied("invalid_scope"))
        );
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || LaunchOutcome::Verified(42)),
            Ok(42)
        );
        assert_eq!(core.activity().unwrap()[0].kind, "tool.execution.completed");
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || panic!("dispatched twice")),
            Err(ExecutionError::Denied("duplicate_request"))
        );
    }

    #[test]
    fn enabled_rule_still_denies_unknown_applications_and_expired_requests() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        command.operation = Some(Operation::LaunchApplication(LaunchApplication {
            application: i32::MAX,
        }));
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || panic!("unknown application")),
            Err(ExecutionError::Denied("default_deny"))
        );
        command.deadline_ms = 0;
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || panic!("expired request")),
            Err(ExecutionError::Denied("expired"))
        );
    }

    #[test]
    fn missing_audit_storage_prevents_dispatch() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let command = request(&core);
        core.db.execute("DROP TABLE events", []).unwrap();
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || panic!("without audit")),
            Err(ExecutionError::AuditUnavailable)
        );
        let reserved: u32 = core
            .db
            .query_row("SELECT COUNT(*) FROM tool_executions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(reserved, 0);
    }

    #[test]
    fn reports_and_audits_unverified_launch() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let command = request(&core);
        let policy = Policy { allow_agent_to_launch_notepad: true, ..Default::default() };
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || LaunchOutcome::Unverified),
            Err(ExecutionError::WindowUnverified)
        );
        assert_eq!(core.activity().unwrap()[0].kind, "tool.execution.partial");
    }
}
