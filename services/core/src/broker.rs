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

/// Agent capabilities the broker can grant for one command. The host requests a capability
/// only when the policy engine allowed the action, or the user approved it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
    /// Start Windows Notepad (fixed executable, no arguments).
    LaunchNotepad = 1 << 0,
    /// List and read text files in the dedicated folder.
    FileRead = 1 << 1,
    /// Copy one text file inside the dedicated folder.
    FileCopy = 1 << 2,
    /// Rename one text file inside the dedicated folder.
    FileMove = 1 << 3,
    /// Move one text file to AiWin11's own trash folder.
    FileTrash = 1 << 4,
    /// Identify the window used before AiWin11 (title and application only).
    WindowLookup = 1 << 5,
    /// Read one window's control tree.
    WindowInspect = 1 << 6,
    /// Capture one window's pixels for a local vision model.
    WindowCapture = 1 << 7,
    /// Type text into a Notepad window the same task launched.
    TypeText = 1 << 8,
    /// Read a snapshot of running processes (names, IDs, memory, CPU time). Read-only.
    ProcessList = 1 << 9,
    /// Read memory, processor load, uptime and drive space. Read-only.
    SystemStatus = 1 << 10,
    /// Read the clipboard's text once.
    ClipboardRead = 1 << 11,
    /// Replace the clipboard with one approved text.
    ClipboardWrite = 1 << 12,
    /// List application windows (titles and applications).
    WindowList = 1 << 13,
    /// Focus, minimize, restore or maximize one listed window.
    WindowArrange = 1 << 14,
    /// Read a snapshot of Windows services (names, state, start mode). Read-only.
    ServiceList = 1 << 15,
    /// Run one approved operation of the elevated helper's closed list.
    AdminOperation = 1 << 16,
    /// Start Character Map or Paint (fixed executables, no arguments).
    LaunchAllowedApp = 1 << 17,
    /// Click one control of the window the same task inspected (UI Automation Invoke).
    ControlInvoke = 1 << 18,
    /// Save one note in the encrypted memory.
    MemoryWrite = 1 << 19,
    /// Read the encrypted memory's notes.
    MemoryRead = 1 << 20,
    /// Run one read-only diagnostic of the closed list (ShellService).
    Diagnostic = 1 << 21,
    /// Start a user-declared MCP server for one request.
    McpServer = 1 << 22,
    /// Generate one image with the user's local engine into AiWin11's images folder.
    ImageGenerate = 1 << 23,
}

/// The capabilities granted for one command. Empty by default: deny by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    grants: u32,
}

impl Policy {
    pub fn granting(capabilities: &[Capability]) -> Self {
        Self { grants: capabilities.iter().fold(0, |grants, capability| grants | *capability as u32) }
    }

    /// Grants `capability` only if `condition` holds, e.g. the user approved.
    pub fn granting_if(condition: bool, capability: Capability) -> Self {
        if condition {
            Self::granting(&[capability])
        } else {
            Self::default()
        }
    }

    pub fn allows(self, capability: Capability) -> bool {
        self.grants & capability as u32 != 0
    }
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
        for id in [&command.request_id, &command.task_id, &command.correlation_id, &command.idempotency_key] {
            if !matches!(Uuid::parse_str(id), Ok(uuid) if uuid.to_string() == *id) {
                return Err("invalid_identifier");
            }
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "clock_unavailable")?.as_millis() as i64;
        if command.deadline_ms <= now {
            return Err("expired");
        }
        let desktop = peer == Peer::DesktopInspector;
        let agent = |capability| peer == Peer::Agent && self.allows(capability);
        match &command.operation {
            Some(Operation::LaunchApplication(launch))
                if launch.application == Application::Notepad as i32 && agent(Capability::LaunchNotepad) =>
            {
                Ok("launch_notepad")
            }
            Some(Operation::LaunchApplication(launch))
                if [Application::CharacterMap as i32, Application::Paint as i32].contains(&launch.application)
                    && agent(Capability::LaunchAllowedApp) =>
            {
                Ok("launch_application")
            }
            Some(Operation::InspectForeground(_)) if peer == Peer::DesktopInspector => Ok("inspect_foreground"),
            Some(Operation::LocateWindow(_)) if agent(Capability::WindowLookup) => Ok("locate_window"),
            Some(Operation::InspectWindow(_)) if agent(Capability::WindowInspect) => Ok("inspect_window"),
            Some(Operation::CaptureWindow(_)) if agent(Capability::WindowCapture) => Ok("capture_window"),
            Some(Operation::GetSystemStatus(_)) if agent(Capability::SystemStatus) => Ok("get_system_status"),
            Some(Operation::ReadClipboard(_)) if agent(Capability::ClipboardRead) => Ok("read_clipboard"),
            Some(Operation::ListWindows(_)) if agent(Capability::WindowList) => Ok("list_windows"),
            Some(Operation::RememberNote(_)) if agent(Capability::MemoryWrite) => Ok("remember_note"),
            Some(Operation::McpRequest(_)) if agent(Capability::McpServer) => Ok("mcp_request"),
            Some(Operation::GenerateImage(_)) if agent(Capability::ImageGenerate) => Ok("generate_image"),
            Some(Operation::RunDiagnostic(run))
                if agent(Capability::Diagnostic)
                    && aiwin11_contracts::v1::run_diagnostic::Diagnostic::try_from(run.diagnostic)
                        .is_ok_and(|d| d != aiwin11_contracts::v1::run_diagnostic::Diagnostic::Unspecified) =>
            {
                Ok("run_diagnostic")
            }
            Some(Operation::RecallNotes(_)) if agent(Capability::MemoryRead) => Ok("recall_notes"),
            Some(Operation::InvokeControl(control))
                if agent(Capability::ControlInvoke)
                    && control.window != 0
                    && control.process_id != 0
                    && (1..=80).contains(&control.index)
                    && control
                        .value
                        .as_ref()
                        .is_none_or(|value| value.chars().count() <= 2000 && !value.contains('\0')) =>
            {
                Ok("invoke_control")
            }
            Some(Operation::AdminService(_) | Operation::AdminCleanTemp(_)) if agent(Capability::AdminOperation) => {
                Ok("admin_operation")
            }
            Some(Operation::ListServices(listing))
                if agent(Capability::ServiceList) && (1..=25).contains(&listing.limit) =>
            {
                Ok("list_services")
            }
            Some(Operation::ArrangeWindow(arrange))
                if agent(Capability::WindowArrange)
                    && arrange.window != 0
                    && arrange.process_id != 0
                    && arrange.arrangement
                        != aiwin11_contracts::v1::arrange_window::Arrangement::Unspecified as i32
                    && aiwin11_contracts::v1::arrange_window::Arrangement::try_from(arrange.arrangement).is_ok() =>
            {
                Ok("arrange_window")
            }
            Some(Operation::WriteClipboard(input))
                if agent(Capability::ClipboardWrite)
                    && !input.text.is_empty()
                    && input.text.chars().count() <= 2000
                    && !input.text.contains('\0') =>
            {
                Ok("write_clipboard")
            }
            Some(Operation::ListProcesses(listing))
                if agent(Capability::ProcessList) && (1..=25).contains(&listing.limit) =>
            {
                Ok("list_processes")
            }
            Some(Operation::TypeInNotepad(input))
                if agent(Capability::TypeText) && !input.text.is_empty() && input.text.chars().count() <= 2000 =>
            {
                Ok("type_in_notepad")
            }
            Some(Operation::CaptureForeground(_)) if peer == Peer::DesktopInspector => Ok("capture_foreground"),
            Some(Operation::Dictate(dictate)) if desktop && (1..=10).contains(&dictate.max_seconds) => Ok("dictate"),
            Some(Operation::AnalyzeCapture(_)) if peer == Peer::DesktopInspector => Ok("analyze_capture"),
            Some(Operation::PreviewLocalText(_)) if desktop || agent(Capability::FileRead) => Ok("preview_local_text"),
            Some(Operation::ListLocalText(_)) if desktop || agent(Capability::FileRead) => Ok("list_local_text"),
            Some(Operation::CopyLocalText(_)) if desktop || agent(Capability::FileCopy) => Ok("copy_local_text"),
            Some(Operation::MoveLocalText(_)) if desktop || agent(Capability::FileMove) => Ok("move_local_text"),
            Some(Operation::TrashLocalText(_)) if agent(Capability::FileTrash) => Ok("trash_local_text"),
            Some(Operation::DeleteLocalText(input)) if peer == Peer::DesktopInspector => {
                if input.confirmed {
                    Ok("delete_local_text")
                } else {
                    Err("confirmation_required")
                }
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
            LaunchOutcome::Unverified => {
                ("tool.execution.partial", "window_unverified", Err(ExecutionError::WindowUnverified))
            }
            LaunchOutcome::Failed => ("tool.execution.failed", "launch_failed", Err(ExecutionError::ExecutionFailed)),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| ExecutionError::OutcomeUnrecorded)?;
        result
    }

    pub(crate) fn authorize_dispatch(
        &mut self,
        policy: Policy,
        peer: Peer,
        command: &Command,
    ) -> Result<Option<String>, ExecutionError> {
        let task_id = Uuid::parse_str(&command.task_id).ok().map(|id| id.to_string());
        let operation = match policy.authorize(peer, command, self.session_id()) {
            Ok(operation) => operation,
            Err(code) => {
                self.audit(task_id.as_deref(), "policy.denied", code).map_err(|_| ExecutionError::AuditUnavailable)?;
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
        let (kind, code) =
            if reserved == 0 { ("policy.denied", "duplicate_request") } else { ("policy.allowed", operation) };
        transaction
            .execute(
                "INSERT INTO events (id, session_id, task_id, kind, source, code)
                 VALUES (?1, ?2, ?3, ?4, 'broker', ?5)",
                params![
                    Uuid::new_v4().to_string(),
                    self.session_id,
                    task_id.as_deref().map(crate::journal_code),
                    kind,
                    crate::journal_code(code)
                ],
            )
            .map_err(|_| ExecutionError::AuditUnavailable)?;
        transaction.commit().map_err(|_| ExecutionError::AuditUnavailable)?;
        if reserved == 0 {
            return Err(ExecutionError::Denied(code));
        }

        if let Err(code) = policy.authorize(peer, command, self.session_id()) {
            self.audit(task_id.as_deref(), "policy.denied", code).map_err(|_| ExecutionError::AuditUnavailable)?;
            return Err(ExecutionError::Denied(code));
        }
        Ok(task_id)
    }

    pub(crate) fn audit(&self, task_id: Option<&str>, kind: &str, code: &str) -> rusqlite::Result<()> {
        self.db.execute(
            "INSERT INTO events (id, session_id, task_id, kind, source, code)
             VALUES (?1, ?2, ?3, ?4, 'broker', ?5)",
            params![
                Uuid::new_v4().to_string(),
                self.session_id,
                task_id.map(crate::journal_code),
                crate::journal_code(kind),
                crate::journal_code(code)
            ],
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
            deadline_ms: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() + 60_000) as i64,
            operation: Some(Operation::LaunchApplication(LaunchApplication {
                application: Application::Notepad as i32,
            })),
        }
    }

    /// Measurement for the NFR "permission evaluation p95 < 50 ms" (ROADMAP §4): the policy
    /// engine decision plus the broker's persisted authorization, on a database file.
    /// Run with `cargo test --locked -p aiwin11-core --release measure_ -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement on the reference machine"]
    fn measure_permission_decision_latency() {
        use crate::policy::{self, ActionContext, AgentAction, AgentProfile, PolicySettings};
        use std::time::Instant;
        let path = std::env::temp_dir().join(format!("aiwin11-measure-{}.sqlite3", Uuid::new_v4()));
        let mut core = Core::open(&path).unwrap();
        let mut samples = Vec::new();
        for _ in 0..500 {
            let command = request(&core);
            let start = Instant::now();
            let decision = policy::evaluate(
                PolicySettings::default(),
                AgentAction::LaunchApplication,
                ActionContext { local_model: true, temporary_grant: false, profile: AgentProfile::General },
            );
            assert!(!decision.rule.is_empty());
            core.authorize_dispatch(Policy::granting(&[Capability::LaunchNotepad]), Peer::Agent, &command).unwrap();
            samples.push(start.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        let p95 = samples[samples.len() * 95 / 100];
        println!("permission decision: p50 {p50:?}, p95 {p95:?}, max {:?}", samples.last().unwrap());
        drop(core);
        let _ = std::fs::remove_file(&path);
        assert!(p95.as_millis() < 50, "p95 {p95:?} over the 50 ms budget");
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
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        assert_eq!(core.authorize_dispatch(policy, Peer::Agent, &command), Err(ExecutionError::Denied("default_deny")));
        assert!(core.authorize_dispatch(policy, Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn file_copy_is_denied_to_the_agent_even_with_launch_permission() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let mut command = request(&core);
        command.operation = Some(Operation::CopyLocalText(Default::default()));
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        assert_eq!(core.authorize_dispatch(policy, Peer::Agent, &command), Err(ExecutionError::Denied("default_deny")));
        assert!(core.authorize_dispatch(policy, Peer::DesktopInspector, &command).is_ok());
    }

    #[test]
    fn agent_file_reads_require_their_own_grant_and_never_extend_to_writes() {
        let reads = [Operation::PreviewLocalText(Default::default()), Operation::ListLocalText(Default::default())];
        let launch_only = Policy::granting(&[Capability::LaunchNotepad]);
        let reads_only = Policy::granting(&[Capability::FileRead]);
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
        let lookup = Policy::granting(&[Capability::WindowLookup]);
        let inspection = Policy::granting(&[Capability::WindowInspect]);
        let everything_else = Policy::granting(&[
            Capability::LaunchNotepad,
            Capability::FileRead,
            Capability::FileCopy,
            Capability::FileMove,
            Capability::FileTrash,
            Capability::WindowCapture,
            Capability::TypeText,
        ]);
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
                command.operation = Some(operation.clone());
                assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{operation:?} {peer:?}");
            }
        }
    }

    #[test]
    fn clipboard_read_and_write_are_separate_bounded_agent_grants() {
        use aiwin11_contracts::v1::{ReadClipboard, WriteClipboard};
        let read = Policy::granting(&[Capability::ClipboardRead]);
        let write = Policy::granting(&[Capability::ClipboardWrite]);
        let read_op = Operation::ReadClipboard(ReadClipboard {});
        let write_op = |text: &str| Operation::WriteClipboard(WriteClipboard { text: text.into() });
        for (policy, peer, operation, expected) in [
            (read, Peer::Agent, read_op.clone(), true),
            (write, Peer::Agent, read_op.clone(), false),
            (read, Peer::DesktopInspector, read_op, false),
            (write, Peer::Agent, write_op("Bonjour"), true),
            (read, Peer::Agent, write_op("Bonjour"), false),
            (write, Peer::Agent, write_op(""), false),
            (write, Peer::Agent, write_op(&"a".repeat(2001)), false),
            (write, Peer::Agent, write_op("a\0b"), false),
        ] {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(operation.clone());
            assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{operation:?} {peer:?}");
        }
    }

    #[test]
    fn window_listing_and_arrangement_are_separate_bounded_grants() {
        use aiwin11_contracts::v1::{arrange_window::Arrangement, ArrangeWindow, ListWindows};
        let list = Policy::granting(&[Capability::WindowList]);
        let arrange = Policy::granting(&[Capability::WindowArrange]);
        let arrange_op =
            |window, arrangement: i32| Operation::ArrangeWindow(ArrangeWindow { window, process_id: 42, arrangement });
        for (policy, operation, expected) in [
            (list, Operation::ListWindows(ListWindows { exclude_process_id: 1 }), true),
            (arrange, Operation::ListWindows(ListWindows { exclude_process_id: 1 }), false),
            (arrange, arrange_op(7, Arrangement::Minimize as i32), true),
            (list, arrange_op(7, Arrangement::Minimize as i32), false),
            (arrange, arrange_op(0, Arrangement::Minimize as i32), false),
            (arrange, arrange_op(7, Arrangement::Unspecified as i32), false),
            (arrange, arrange_op(7, 99), false),
        ] {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(operation.clone());
            assert_eq!(core.authorize_dispatch(policy, Peer::Agent, &command).is_ok(), expected, "{operation:?}");
        }
    }

    #[test]
    fn service_listing_needs_its_own_grant_and_a_bounded_limit() {
        use aiwin11_contracts::v1::ListServices;
        let listing = Policy::granting(&[Capability::ServiceList]);
        let others = Policy::granting(&[Capability::ProcessList, Capability::SystemStatus]);
        for (policy, peer, limit, expected) in [
            (listing, Peer::Agent, 20, true),
            (listing, Peer::Agent, 0, false),
            (listing, Peer::Agent, 26, false),
            (others, Peer::Agent, 20, false),
            (listing, Peer::DesktopInspector, 20, false),
        ] {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(Operation::ListServices(ListServices { limit }));
            assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{peer:?} {limit}");
        }
    }

    #[test]
    fn system_status_needs_its_own_agent_grant() {
        let status = Policy::granting(&[Capability::SystemStatus]);
        let others = Policy::granting(&[Capability::ProcessList, Capability::FileRead]);
        for (policy, peer, expected) in
            [(status, Peer::Agent, true), (others, Peer::Agent, false), (status, Peer::DesktopInspector, false)]
        {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(Operation::GetSystemStatus(Default::default()));
            assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{peer:?}");
        }
    }

    #[test]
    fn process_listing_needs_its_own_grant_and_a_bounded_limit() {
        use aiwin11_contracts::v1::ListProcesses;
        let listing = Policy::granting(&[Capability::ProcessList]);
        let others = Policy::granting(&[Capability::WindowInspect, Capability::FileRead, Capability::LaunchNotepad]);
        for (policy, peer, limit, expected) in [
            (listing, Peer::Agent, 15, true),
            (listing, Peer::Agent, 25, true),
            (listing, Peer::Agent, 0, false),
            (listing, Peer::Agent, 26, false),
            (others, Peer::Agent, 15, false),
            (listing, Peer::DesktopInspector, 15, false),
            (listing, Peer::Other, 15, false),
        ] {
            let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
            let mut command = request(&core);
            command.operation = Some(Operation::ListProcesses(ListProcesses { limit }));
            assert_eq!(core.authorize_dispatch(policy, peer, &command).is_ok(), expected, "{peer:?} {limit}");
        }
    }

    #[test]
    fn agent_copy_grant_covers_copy_only() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let copy_only = Policy::granting(&[Capability::FileCopy]);
        let mut command = request(&core);
        command.operation = Some(Operation::CopyLocalText(Default::default()));
        assert_eq!(
            core.authorize_dispatch(Policy::granting(&[Capability::FileRead]), Peer::Agent, &command),
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
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        assert_eq!(
            core.execute(policy, Peer::Other, &command, || panic!("dispatched")),
            Err(ExecutionError::Denied("invalid_scope"))
        );
        assert_eq!(core.execute(policy, Peer::Agent, &command, || LaunchOutcome::Verified(42)), Ok(42));
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
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        command.operation = Some(Operation::LaunchApplication(LaunchApplication { application: i32::MAX }));
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
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || panic!("without audit")),
            Err(ExecutionError::AuditUnavailable)
        );
        let reserved: u32 = core.db.query_row("SELECT COUNT(*) FROM tool_executions", [], |row| row.get(0)).unwrap();
        assert_eq!(reserved, 0);
    }

    #[test]
    fn reports_and_audits_unverified_launch() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let command = request(&core);
        let policy = Policy::granting(&[Capability::LaunchNotepad]);
        assert_eq!(
            core.execute(policy, Peer::Agent, &command, || LaunchOutcome::Unverified),
            Err(ExecutionError::WindowUnverified)
        );
        assert_eq!(core.activity().unwrap()[0].kind, "tool.execution.partial");
    }
}
