use std::{
    io::{Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command as Process, Stdio},
    ptr,
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aiwin11_contracts::{
    v1::{
        arrange_window::Arrangement, command::Operation, dictation_result::Status as DictationStatus, result::Status,
        run_diagnostic::Diagnostic, service_entry::StartMode, AnalyzeCapture, ArrangeWindow, CaptureForeground,
        CaptureWindow, CharacterMapDigit, Command, Dictate, GetSystemStatus, InspectForeground, InspectWindow,
        InvokeControl, ListProcesses, ListServices, ListWindows, LocateWindow, ReadClipboard, Result as BridgeReply,
        RunDiagnostic, TypeInNotepad, UiTree, WriteClipboard,
    },
    Message, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

use crate::{
    broker::{ExecutionError, LaunchOutcome, Peer, Policy},
    model_gateway::{GatewayError, Ollama},
    Core,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Time allowed after the command deadline for the bridge to report and exit.
const REPLY_GRACE: Duration = Duration::from_secs(2);
/// Upper bound on any wait, whatever deadline the command carries.
const MAX_BRIDGE_WAIT: Duration = Duration::from_secs(25);
const MAX_MESSAGE_BYTES: usize = 1024;

#[derive(Deserialize, Serialize)]
pub struct Display {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    primary: bool,
}
const MAX_JPEG_BYTES: usize = 256 * 1024;
/// Deadline of agent tool commands (see `Core::agent_command`).
const AGENT_DEADLINE_MS: i64 = 15_000;
/// Largest command sent to the bridge: room for 2000 characters of Notepad text.
const MAX_COMMAND_BYTES: usize = 16 * 1024;
const MAX_TREE_REPLY_BYTES: usize = 32 * 1024;
const MAX_PROCESS_REPLY_BYTES: usize = 8 * 1024;
/// 4000 characters of UTF-8 text plus the envelope.
const MAX_CLIPBOARD_REPLY_BYTES: usize = 20 * 1024;
/// 20 windows with a 120-character title and a 64-character application name.
const MAX_WINDOW_LIST_REPLY_BYTES: usize = 16 * 1024;
pub(crate) const MAX_LISTED_WINDOWS: usize = 20;
/// Services reported to the agent by `list_services`.
pub(crate) const AGENT_SERVICE_LIMIT: u32 = 20;
pub(crate) const MAX_CLIPBOARD_READ_CHARS: usize = 4000;
/// Processes reported to the agent by `list_processes`.
pub(crate) const AGENT_PROCESS_LIMIT: u32 = 15;
const MAX_AGENT_NODES: usize = 80;

/// A located window. Before approval, only the user sees its title and application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WindowTarget {
    pub window: u64,
    pub process_id: u32,
    pub title: String,
    pub process_name: String,
}

/// One control of an inspected window. The name is empty for password fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamedNode {
    pub parent: u32,
    pub control_type: u32,
    pub enabled: bool,
    pub focused: bool,
    pub name: String,
}

#[derive(Serialize)]
pub struct Inspection {
    nodes: Vec<InspectionNode>,
    truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectionNode {
    parent: u32,
    control_type: u32,
    enabled: bool,
    focused: bool,
}

#[derive(Serialize)]
pub struct Capture {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

impl Core {
    pub fn enter_character_map_digit(&mut self, digit: u32) -> Result<bool, &'static str> {
        let command = self
            .desktop_command(Operation::CharacterMapDigit(CharacterMapDigit { digit: Some(digit) }))
            .map_err(|_| "INPUT_UNAVAILABLE")?;
        let task =
            self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command).map_err(|_| "INPUT_DENIED")?;
        let outcome = dispatch(&command)
            .and_then(|reply| {
                if reply.ui_tree.is_some() || reply.window_capture.is_some() || reply.process_id.is_some() {
                    return None;
                }
                match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                    (Status::Success, None) => Some(Ok(true)),
                    (Status::Partial, Some("field_unverified")) => Some(Ok(false)),
                    (Status::Denied, Some("character_map_unavailable")) => Some(Err("CHARACTER_MAP_UNAVAILABLE")),
                    _ => None,
                }
            })
            .unwrap_or(Err("INPUT_UNAVAILABLE"));
        let (kind, code) = match outcome {
            Ok(true) => ("input.completed", "digit_verified"),
            Ok(false) => ("input.partial", "field_unverified"),
            Err(code) => ("input.failed", code),
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| "INPUT_AUDIT_UNAVAILABLE")?;
        outcome
    }

    pub fn analyze_capture(core: &Mutex<Self>, model: &str, jpeg: &[u8]) -> Result<String, &'static str> {
        Self::process_capture(core, model, jpeg, Operation::AnalyzeCapture(AnalyzeCapture {}))
    }

    pub fn transcribe_capture(core: &Mutex<Self>, model: &str, jpeg: &[u8]) -> Result<String, &'static str> {
        Self::process_capture(core, model, jpeg, Operation::TranscribeCapture(Default::default()))
    }

    fn process_capture(
        core: &Mutex<Self>,
        model: &str,
        jpeg: &[u8],
        operation: Operation,
    ) -> Result<String, &'static str> {
        if !valid_jpeg(jpeg) {
            return Err("INVALID_CAPTURE");
        }
        if model.is_empty() || model.len() > 256 {
            return Err("VISION_MODEL_REQUIRED");
        }
        let transcribe = matches!(&operation, Operation::TranscribeCapture(_));
        let task = {
            let mut core = core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
            let command = core.desktop_command(operation).map_err(|_| "VISION_UNAVAILABLE")?;
            core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
                .map_err(|_| "VISION_NOT_AUTHORIZED")?
        };
        let result = Ollama::connect().and_then(|provider| {
            if transcribe {
                provider.transcribe_capture(model, jpeg)
            } else {
                provider.describe_capture(model, jpeg)
            }
        });
        let kind = match (transcribe, result.is_ok()) {
            (true, true) => "ocr.completed",
            (true, false) => "ocr.failed",
            (false, true) => "vision.completed",
            (false, false) => "vision.failed",
        };
        let code = result.as_ref().err().map_or("success", GatewayError::vision_code);
        core.lock()
            .map_err(|_| "CORE_UNAVAILABLE")?
            .audit(task.as_deref(), kind, code)
            .map_err(|_| "VISION_AUDIT_UNAVAILABLE")?;
        result.map_err(|error: GatewayError| error.vision_code())
    }

    pub(crate) fn desktop_command(&self, operation: Operation) -> Result<Command, ExecutionError> {
        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| ExecutionError::ExecutionFailed)?;
        Ok(Command {
            schema_version: SCHEMA_VERSION,
            request_id: id.clone(),
            session_id: self.session_id().into(),
            task_id: id.clone(),
            correlation_id: id.clone(),
            authenticated_actor_id: "desktop_inspector".into(),
            deadline_ms: now.as_millis() as i64 + 6_000,
            idempotency_key: id,
            operation: Some(operation),
        })
    }

    /// A command issued for an agent tool call within a chat task. Each call gets its own
    /// request ID and idempotency key; the task ID ties it to the task's audit trail.
    /// The 15-second deadline leaves room for a bridge that is still starting (the operations
    /// themselves return as soon as they finish).
    pub(crate) fn agent_command(&self, task: &str, operation: Operation) -> Result<Command, ExecutionError> {
        let mut command = self.desktop_command(operation)?;
        command.task_id = task.into();
        command.correlation_id = task.into();
        command.authenticated_actor_id = "agent".into();
        command.deadline_ms = unix_millis().ok_or(ExecutionError::ExecutionFailed)? + AGENT_DEADLINE_MS;
        Ok(command)
    }

    /// Agent tool, step 1: identifies the window the user was using before AiWin11.
    pub(crate) fn agent_locate_window(&mut self, task: &str, policy: Policy) -> Result<WindowTarget, &'static str> {
        let operation = Operation::LocateWindow(LocateWindow { exclude_process_id: std::process::id() });
        self.agent_bridge_call(task, policy, operation, "window.locate", |reply| {
            if reply.ui_tree.is_some() || reply.window_capture.is_some() || reply.process_id.is_some() {
                return None;
            }
            let info = reply.window_info?;
            let bounded = |text: &str, max: usize| !text.is_empty() && text.chars().count() <= max;
            let valid = info.window != 0
                && info.process_id != 0
                && bounded(&info.title, 120)
                && bounded(&info.process_name, 64);
            valid.then_some(WindowTarget {
                window: info.window,
                process_id: info.process_id,
                title: info.title,
                process_name: info.process_name,
            })
        })
    }

    /// Agent tool: read-only snapshot of the processes using the most memory.
    pub(crate) fn agent_list_processes(&mut self, task: &str, policy: Policy) -> Result<ProcessSnapshot, &'static str> {
        let operation = Operation::ListProcesses(ListProcesses { limit: AGENT_PROCESS_LIMIT });
        self.agent_bridge_call(task, policy, operation, "process.list", |reply| {
            if reply.ui_tree.is_some()
                || reply.window_capture.is_some()
                || reply.window_info.is_some()
                || reply.process_id.is_some()
            {
                return None;
            }
            accept_process_list(reply.process_list?)
        })
    }

    /// Agent tool: clicks control `index` (1-based) of the inspected window, if it still has
    /// the same type and name.
    pub(crate) fn agent_invoke_control(
        &mut self,
        task: &str,
        policy: Policy,
        target: &WindowTarget,
        index: u32,
        node: &NamedNode,
        value: Option<&str>,
    ) -> Result<(), &'static str> {
        let operation = Operation::InvokeControl(InvokeControl {
            window: target.window,
            process_id: target.process_id,
            index,
            control_type: node.control_type,
            name: node.name.clone(),
            value: value.map(str::to_owned),
        });
        let command = self.agent_command(task, operation).map_err(|_| "BRIDGE_FAILED")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "WINDOW_ACCESS_DENIED",
            _ => "WINDOW_AUDIT_UNAVAILABLE",
        })?;
        let outcome = match dispatch(&command) {
            Some(reply) if reply.ui_tree.is_none() => {
                match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                    (Status::Success, None) => Ok(()),
                    (Status::Failed, Some("not_invokable")) => Err("CONTROL_NOT_INVOKABLE"),
                    (Status::Failed, Some("control_changed")) => Err("CONTROL_CHANGED"),
                    _ => Err("BRIDGE_FAILED"),
                }
            }
            _ => Err("BRIDGE_FAILED"),
        };
        let (kind, code) = match outcome {
            Ok(()) => ("control.invoke.completed", "invoked"),
            Err(code) => ("control.invoke.failed", code),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| "WINDOW_AUDIT_UNAVAILABLE")?;
        outcome
    }

    /// Agent tool: one read-only diagnostic of the closed list. Returns its output.
    pub(crate) fn agent_run_diagnostic(
        &mut self,
        task: &str,
        policy: Policy,
        diagnostic: Diagnostic,
    ) -> Result<(String, bool), &'static str> {
        let operation = Operation::RunDiagnostic(RunDiagnostic { diagnostic: diagnostic as i32 });
        self.agent_bridge_call(task, policy, operation, "diagnostic.run", |reply| {
            if reply.ui_tree.is_some() || reply.window_list.is_some() || reply.clipboard_content.is_some() {
                return None;
            }
            let output = reply.command_output?;
            (output.text.chars().count() <= 12_000).then_some((output.text, output.truncated))
        })
    }

    /// Agent tool: read-only snapshot of Windows services.
    pub(crate) fn agent_list_services(&mut self, task: &str, policy: Policy) -> Result<ServiceSnapshot, &'static str> {
        let operation = Operation::ListServices(ListServices { limit: AGENT_SERVICE_LIMIT });
        self.agent_bridge_call(task, policy, operation, "service.list", |reply| {
            if reply.ui_tree.is_some()
                || reply.window_capture.is_some()
                || reply.window_info.is_some()
                || reply.process_id.is_some()
                || reply.window_list.is_some()
            {
                return None;
            }
            accept_service_list(reply.service_list?)
        })
    }

    /// Agent tool: lists application windows (titles, applications), after the user's approval.
    pub(crate) fn agent_list_windows(&mut self, task: &str, policy: Policy) -> Result<Vec<ListedWindow>, &'static str> {
        let operation = Operation::ListWindows(ListWindows { exclude_process_id: std::process::id() });
        self.agent_bridge_call(task, policy, operation, "window.list", |reply| {
            if reply.ui_tree.is_some()
                || reply.window_capture.is_some()
                || reply.window_info.is_some()
                || reply.process_id.is_some()
                || reply.clipboard_content.is_some()
            {
                return None;
            }
            accept_window_list(reply.window_list?)
        })
    }

    /// Agent tool: applies one arrangement to a window the same task listed. Returns whether
    /// the resulting state was verified.
    pub(crate) fn agent_arrange_window(
        &mut self,
        task: &str,
        policy: Policy,
        target: &WindowTarget,
        arrangement: Arrangement,
    ) -> Result<bool, &'static str> {
        let operation = Operation::ArrangeWindow(ArrangeWindow {
            window: target.window,
            process_id: target.process_id,
            arrangement: arrangement as i32,
        });
        let command = self.agent_command(task, operation).map_err(|_| "BRIDGE_FAILED")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "WINDOW_ACCESS_DENIED",
            _ => "WINDOW_AUDIT_UNAVAILABLE",
        })?;
        let outcome = match dispatch(&command) {
            Some(reply) if reply.window_list.is_none() && reply.ui_tree.is_none() => {
                match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                    (Status::Success, None) => Ok(true),
                    (Status::Partial, Some("state_unverified")) => Ok(false),
                    (Status::Failed, Some("window_unavailable")) => Err("WINDOW_UNAVAILABLE"),
                    _ => Err("BRIDGE_FAILED"),
                }
            }
            _ => Err("BRIDGE_FAILED"),
        };
        let (kind, code) = match outcome {
            Ok(true) => ("window.arrange.completed", "state_verified"),
            Ok(false) => ("window.arrange.partial", "state_unverified"),
            Err(code) => ("window.arrange.failed", code),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| "WINDOW_AUDIT_UNAVAILABLE")?;
        outcome
    }

    /// Agent tool: reads the clipboard's text once, after the user's approval.
    pub(crate) fn agent_read_clipboard(&mut self, task: &str, policy: Policy) -> Result<ClipboardRead, &'static str> {
        let operation = Operation::ReadClipboard(ReadClipboard {});
        self.agent_bridge_call(task, policy, operation, "clipboard.read", |reply| {
            if reply.ui_tree.is_some()
                || reply.window_capture.is_some()
                || reply.window_info.is_some()
                || reply.process_id.is_some()
                || reply.process_list.is_some()
                || reply.system_status.is_some()
            {
                return None;
            }
            let content = reply.clipboard_content?;
            let count = content.text.chars().count();
            (count <= MAX_CLIPBOARD_READ_CHARS && content.has_text == (count > 0) && (!content.truncated || count > 0))
                .then_some(ClipboardRead { text: content.text, truncated: content.truncated })
        })
    }

    /// Agent tool: replaces the clipboard with `text`, after the user's approval. Returns
    /// whether the text was read back identical.
    pub(crate) fn agent_write_clipboard(
        &mut self,
        task: &str,
        policy: Policy,
        text: &str,
    ) -> Result<bool, &'static str> {
        let operation = Operation::WriteClipboard(WriteClipboard { text: text.to_owned() });
        let command = self.agent_command(task, operation).map_err(|_| "BRIDGE_FAILED")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "CLIPBOARD_ACCESS_DENIED",
            _ => "CLIPBOARD_AUDIT_UNAVAILABLE",
        })?;
        let outcome = match dispatch(&command) {
            Some(reply) if reply.clipboard_content.is_none() && reply.ui_tree.is_none() => {
                match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                    (Status::Success, None) => Ok(true),
                    (Status::Partial, Some("text_unverified")) => Ok(false),
                    (Status::Failed, Some("clipboard_unavailable")) => Err("CLIPBOARD_UNAVAILABLE"),
                    _ => Err("BRIDGE_FAILED"),
                }
            }
            _ => Err("BRIDGE_FAILED"),
        };
        let (kind, code) = match outcome {
            Ok(true) => ("clipboard.write.completed", "text_verified"),
            Ok(false) => ("clipboard.write.partial", "text_unverified"),
            Err(code) => ("clipboard.write.failed", code),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| "CLIPBOARD_AUDIT_UNAVAILABLE")?;
        outcome
    }

    /// Agent tool: read-only system metrics.
    pub(crate) fn agent_system_status(&mut self, task: &str, policy: Policy) -> Result<SystemSnapshot, &'static str> {
        let operation = Operation::GetSystemStatus(GetSystemStatus {});
        self.agent_bridge_call(task, policy, operation, "system.status", |reply| {
            if reply.ui_tree.is_some()
                || reply.window_capture.is_some()
                || reply.window_info.is_some()
                || reply.process_id.is_some()
                || reply.process_list.is_some()
            {
                return None;
            }
            accept_system_status(reply.system_status?)
        })
    }

    /// Agent tool, step 2: reads the named control tree of exactly that window.
    /// The host grants `Capability::WindowInspect` only after the user approved it.
    pub(crate) fn agent_inspect_window(
        &mut self,
        task: &str,
        policy: Policy,
        target: &WindowTarget,
    ) -> Result<(Vec<NamedNode>, bool), &'static str> {
        let operation =
            Operation::InspectWindow(InspectWindow { window: target.window, process_id: target.process_id });
        self.agent_bridge_call(task, policy, operation, "window.inspect", |reply| {
            if reply.window_info.is_some() || reply.window_capture.is_some() || reply.process_id.is_some() {
                return None;
            }
            named_tree(reply.ui_tree?)
        })
    }

    /// Agent tool: captures exactly that window as a JPEG (at most 1024 px per side, 256 KiB),
    /// for a local vision model. The host grants `Capability::WindowCapture` only after approval.
    pub(crate) fn agent_capture_window(
        &mut self,
        task: &str,
        policy: Policy,
        target: &WindowTarget,
    ) -> Result<Vec<u8>, &'static str> {
        let operation =
            Operation::CaptureWindow(CaptureWindow { window: target.window, process_id: target.process_id });
        self.agent_bridge_call(task, policy, operation, "window.capture", |reply| {
            if reply.ui_tree.is_some() || reply.window_info.is_some() || reply.process_id.is_some() {
                return None;
            }
            let frame = reply.window_capture?;
            ((1..=1024).contains(&frame.width) && (1..=1024).contains(&frame.height) && valid_jpeg(&frame.jpeg))
                .then_some(frame.jpeg)
        })
    }

    /// Agent tool: writes `text` into the empty editor of the Notepad process `process_id`,
    /// which the same task launched. Returns whether the text was read back identical.
    pub(crate) fn agent_type_in_notepad(
        &mut self,
        task: &str,
        policy: Policy,
        process_id: u32,
        text: &str,
    ) -> Result<bool, &'static str> {
        let operation = Operation::TypeInNotepad(TypeInNotepad { process_id, text: text.to_owned() });
        let command = self.agent_command(task, operation).map_err(|_| "BRIDGE_FAILED")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "INPUT_ACCESS_DENIED",
            _ => "INPUT_AUDIT_UNAVAILABLE",
        })?;
        let outcome = match dispatch(&command) {
            Some(reply)
                if reply.ui_tree.is_none()
                    && reply.window_capture.is_none()
                    && reply.window_info.is_none()
                    && reply.process_id.is_none() =>
            {
                match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                    (Status::Success, None) => Ok(true),
                    (Status::Partial, Some("text_unverified")) => Ok(false),
                    (Status::Denied, Some("document_not_empty")) => Err("NOTEPAD_NOT_EMPTY"),
                    (Status::Denied, Some("notepad_unavailable")) => Err("NOTEPAD_UNAVAILABLE"),
                    _ => Err("BRIDGE_FAILED"),
                }
            }
            _ => Err("BRIDGE_FAILED"),
        };
        let (kind, code) = match outcome {
            Ok(true) => ("input.notepad.completed", "text_verified"),
            Ok(false) => ("input.notepad.partial", "text_unverified"),
            Err(code) => ("input.notepad.failed", code),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| "INPUT_AUDIT_UNAVAILABLE")?;
        outcome
    }

    fn agent_bridge_call<T>(
        &mut self,
        task: &str,
        policy: Policy,
        operation: Operation,
        event: &str,
        accept: impl FnOnce(BridgeReply) -> Option<T>,
    ) -> Result<T, &'static str> {
        let command = self.agent_command(task, operation).map_err(|_| "WINDOW_UNAVAILABLE")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "WINDOW_ACCESS_DENIED",
            _ => "WINDOW_AUDIT_UNAVAILABLE",
        })?;
        let result = dispatch(&command)
            .filter(|reply| reply.status() == Status::Success && reply.error.is_none())
            .and_then(accept);
        let (suffix, code) = if result.is_some() { ("completed", "success") } else { ("failed", "window_unavailable") };
        self.audit(task_id.as_deref(), &format!("{event}.{suffix}"), code).map_err(|_| "WINDOW_AUDIT_UNAVAILABLE")?;
        result.ok_or("WINDOW_UNAVAILABLE")
    }

    /// Desktop only, after the user's click: one phrase from the microphone, recognized
    /// offline by Windows. Neither audio nor text is stored or journaled.
    pub fn dictate(&mut self) -> Result<String, &'static str> {
        let mut command =
            self.desktop_command(Operation::Dictate(Dictate { max_seconds: 10 })).map_err(|_| "DICTATION_FAILED")?;
        command.deadline_ms = unix_millis().ok_or("DICTATION_FAILED")? + 14_000;
        let task = self
            .authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|_| "DICTATION_FAILED")?;
        let outcome =
            match dispatch(&command).filter(|reply| reply.status() == Status::Success).and_then(|r| r.dictation) {
                Some(result) => match result.status() {
                    DictationStatus::Recognized if result.text.chars().count() <= 2000 => Ok(result.text),
                    DictationStatus::Silence => Err("DICTATION_SILENCE"),
                    DictationStatus::NoRecognizer => Err("DICTATION_NO_RECOGNIZER"),
                    DictationStatus::NoMicrophone => Err("DICTATION_NO_MICROPHONE"),
                    _ => Err("DICTATION_FAILED"),
                },
                None => Err("DICTATION_FAILED"),
            };
        let (kind, code) = match &outcome {
            Ok(_) => ("dictation.completed", "recognized"),
            Err(code) => ("dictation.failed", *code),
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| "DICTATION_FAILED")?;
        outcome
    }

    pub fn capture_foreground(&mut self) -> Result<Capture, ExecutionError> {
        self.capture_desktop(Operation::CaptureForeground(CaptureForeground {}))
    }

    pub fn capture_display(&mut self, display: Display) -> Result<Capture, ExecutionError> {
        self.capture_desktop(Operation::CaptureDisplay(aiwin11_contracts::v1::CaptureDisplay {
            display: Some(aiwin11_contracts::v1::DisplayInfo {
                x: display.x,
                y: display.y,
                width: display.width,
                height: display.height,
                primary: display.primary,
            }),
        }))
    }

    fn capture_desktop(&mut self, operation: Operation) -> Result<Capture, ExecutionError> {
        let display = matches!(&operation, Operation::CaptureDisplay(_));
        let command = self.desktop_command(operation)?;
        let task = self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)?;
        let capture = dispatch(&command).and_then(|reply| {
            if reply.status() != Status::Success
                || reply.error.is_some()
                || reply.process_id.is_some()
                || reply.ui_tree.is_some()
            {
                return None;
            }
            let frame = reply.window_capture?;
            if !(1..=1024).contains(&frame.width) || !(1..=1024).contains(&frame.height) || !valid_jpeg(&frame.jpeg) {
                return None;
            }
            Some(Capture { jpeg: frame.jpeg, width: frame.width, height: frame.height })
        });
        let kind = match (display, capture.is_some()) {
            (true, true) => "display.capture.completed",
            (true, false) => "display.capture.failed",
            (false, true) => "capture.completed",
            (false, false) => "capture.failed",
        };
        let code = if capture.is_some() { "success" } else { "capture_unavailable" };
        self.audit(task.as_deref(), kind, code).map_err(|_| ExecutionError::OutcomeUnrecorded)?;
        capture.ok_or(ExecutionError::ExecutionFailed)
    }

    pub fn list_displays(&mut self) -> Result<Vec<Display>, &'static str> {
        let command =
            self.desktop_command(Operation::ListDisplays(Default::default())).map_err(|_| "DISPLAYS_UNAVAILABLE")?;
        let task = self
            .authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|_| "DISPLAYS_NOT_AUTHORIZED")?;
        let displays = dispatch(&command).and_then(|reply| {
            if reply.status() != Status::Success || reply.error.is_some() {
                return None;
            }
            let list = reply.display_list?.displays;
            if !(1..=32).contains(&list.len())
                || list.iter().filter(|display| display.primary).count() != 1
                || list.iter().any(|display| {
                    display.width == 0
                        || display.height == 0
                        || i32::try_from(display.width).ok().and_then(|width| display.x.checked_add(width)).is_none()
                        || i32::try_from(display.height).ok().and_then(|height| display.y.checked_add(height)).is_none()
                })
            {
                return None;
            }
            Some(
                list.into_iter()
                    .map(|display| Display {
                        x: display.x,
                        y: display.y,
                        width: display.width,
                        height: display.height,
                        primary: display.primary,
                    })
                    .collect(),
            )
        });
        let (kind, code) = if displays.is_some() {
            ("displays.completed", "success")
        } else {
            ("displays.failed", "displays_unavailable")
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| "DISPLAYS_AUDIT_UNAVAILABLE")?;
        displays.ok_or("DISPLAYS_UNAVAILABLE")
    }

    pub fn inspect_foreground(&mut self) -> Result<Inspection, ExecutionError> {
        let command = self.desktop_command(Operation::InspectForeground(InspectForeground {}))?;
        let task = self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)?;
        let tree =
            dispatch(&command).and_then(|reply| {
                if reply.status() != Status::Success
                    || reply.error.is_some()
                    || reply.process_id.is_some()
                    || reply.window_capture.is_some()
                {
                    return None;
                }
                let tree = reply.ui_tree?;
                if tree.nodes.is_empty()
                    || tree.nodes.len() > 40
                    || tree.nodes.iter().enumerate().any(|(i, node)| {
                        (i == 0 && node.parent != 0) || (i > 0 && !(1..=i as u32).contains(&node.parent))
                    })
                {
                    return None;
                }
                Some(Inspection {
                    truncated: tree.truncated,
                    nodes: tree
                        .nodes
                        .into_iter()
                        .map(|node| InspectionNode {
                            parent: node.parent,
                            control_type: node.control_type,
                            enabled: node.enabled,
                            focused: node.focused,
                        })
                        .collect(),
                })
            });
        let (kind, code) = if tree.is_some() {
            ("inspection.completed", "success")
        } else {
            ("inspection.failed", "window_unavailable")
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| ExecutionError::OutcomeUnrecorded)?;
        tree.ok_or(ExecutionError::ExecutionFailed)
    }

    pub fn execute_windows(&mut self, policy: Policy, peer: Peer, command: &Command) -> Result<u32, ExecutionError> {
        self.execute(policy, peer, command, || {
            let mut trusted_command = command.clone();
            trusted_command.authenticated_actor_id = "agent".into();
            match dispatch(&trusted_command) {
                Some(reply) if reply.ui_tree.is_none() && reply.window_capture.is_none() => {
                    match (reply.status(), reply.process_id) {
                        (Status::Success, Some(pid)) if pid != 0 && reply.error.is_none() => {
                            LaunchOutcome::Verified(pid)
                        }
                        (Status::Partial, None) => LaunchOutcome::Unverified,
                        _ => LaunchOutcome::Failed,
                    }
                }
                _ => LaunchOutcome::Failed,
            }
        })
    }
}

/// Checks the shape of an agent inspection: 1 to 80 nodes, parents pointing backwards
/// (one-based, zero for the root), names of at most 80 characters.
fn named_tree(tree: UiTree) -> Option<(Vec<NamedNode>, bool)> {
    let valid = !tree.nodes.is_empty()
        && tree.nodes.len() <= MAX_AGENT_NODES
        && tree.nodes.iter().enumerate().all(|(index, node)| {
            (if index == 0 { node.parent == 0 } else { (1..=index as u32).contains(&node.parent) })
                && node.name.chars().count() <= 80
        });
    valid.then(|| {
        (
            tree.nodes
                .into_iter()
                .map(|node| NamedNode {
                    parent: node.parent,
                    control_type: node.control_type,
                    enabled: node.enabled,
                    focused: node.focused,
                    name: node.name,
                })
                .collect(),
            tree.truncated,
        )
    })
}

fn valid_jpeg(jpeg: &[u8]) -> bool {
    jpeg.len() <= MAX_JPEG_BYTES && jpeg.starts_with(&[0xff, 0xd8]) && jpeg.ends_with(&[0xff, 0xd9])
}

/// One bridge process started ahead of time. It blocks on its private stdin and does
/// nothing until it receives a broker-authorized command, so its startup cost (.NET load,
/// antivirus scan of a newly installed binary: 17.9 s measured once) is not spent within a
/// command's deadline. It has no listener; when the host exits, its stdin closes and it exits.
static SPARE_BRIDGE: Mutex<Option<std::process::Child>> = Mutex::new(None);

fn spawn_bridge() -> Option<std::process::Child> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    Process::new(directory.join("AiWin11.Windows.exe"))
        .current_dir(directory)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Starts the spare bridge if none is waiting. Called at application start and after each
/// dispatch. Failures are ignored: `dispatch` then starts a bridge on demand.
/// One process as reported to the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessInfo {
    pub process_id: u32,
    pub name: String,
    pub working_set_bytes: u64,
    pub cpu_time_ms: u64,
    pub has_window: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessSnapshot {
    pub processes: Vec<ProcessInfo>,
    pub total: u32,
}

/// Validates the bridge's process list: bounded size, real IDs, short printable names,
/// sorted by memory as requested.
fn accept_process_list(list: aiwin11_contracts::v1::ProcessList) -> Option<ProcessSnapshot> {
    if list.processes.len() > AGENT_PROCESS_LIMIT as usize || (list.total as usize) < list.processes.len() {
        return None;
    }
    let sorted = list.processes.windows(2).all(|pair| pair[0].working_set_bytes >= pair[1].working_set_bytes);
    let valid = list.processes.iter().all(|entry| {
        entry.process_id != 0
            && !entry.name.is_empty()
            && entry.name.chars().count() <= 64
            && entry.name.chars().all(|c| !c.is_control())
    });
    (sorted && valid).then(|| ProcessSnapshot {
        total: list.total,
        processes: list
            .processes
            .into_iter()
            .map(|entry| ProcessInfo {
                process_id: entry.process_id,
                name: entry.name,
                working_set_bytes: entry.working_set_bytes,
                cpu_time_ms: entry.cpu_time_ms,
                has_window: entry.has_window,
            })
            .collect(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub running: bool,
    pub start_mode: StartMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceSnapshot {
    pub services: Vec<ServiceInfo>,
    pub total: u32,
    pub running: u32,
    pub stopped_automatic: u32,
}

fn accept_service_list(list: aiwin11_contracts::v1::ServiceList) -> Option<ServiceSnapshot> {
    let counts_ok = list.running <= list.total
        && list.stopped_automatic <= list.total
        && list.services.len() <= AGENT_SERVICE_LIMIT as usize
        && list.services.len() <= list.total as usize;
    let bounded = |text: &str, max: usize| text.chars().count() <= max && text.chars().all(|c| !c.is_control());
    let services: Option<Vec<_>> = list
        .services
        .into_iter()
        .map(|entry| {
            let start_mode =
                StartMode::try_from(entry.start_mode).ok().filter(|mode| *mode != StartMode::Unspecified)?;
            (!entry.name.is_empty() && bounded(&entry.name, 64) && bounded(&entry.display_name, 120)).then_some(
                ServiceInfo { name: entry.name, display_name: entry.display_name, running: entry.running, start_mode },
            )
        })
        .collect();
    let services = services.filter(|_| counts_ok)?;
    Some(ServiceSnapshot {
        services,
        total: list.total,
        running: list.running,
        stopped_automatic: list.stopped_automatic,
    })
}

/// One window of a `list_windows` result, kept by the task for `arrange_window`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedWindow {
    pub target: WindowTarget,
    pub minimized: bool,
}

fn accept_window_list(list: aiwin11_contracts::v1::WindowList) -> Option<Vec<ListedWindow>> {
    if list.windows.len() > MAX_LISTED_WINDOWS {
        return None;
    }
    let bounded = |text: &str, max: usize| {
        !text.is_empty() && text.chars().count() <= max && text.chars().all(|c| !c.is_control())
    };
    list.windows
        .into_iter()
        .map(|info| {
            (info.window != 0 && info.process_id != 0 && bounded(&info.title, 120) && bounded(&info.process_name, 64))
                .then_some(ListedWindow {
                    minimized: info.minimized,
                    target: WindowTarget {
                        window: info.window,
                        process_id: info.process_id,
                        title: info.title,
                        process_name: info.process_name,
                    },
                })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClipboardRead {
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriveSpace {
    pub name: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemSnapshot {
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub cpu_load_percent: u32,
    pub logical_processors: u32,
    pub uptime_seconds: u64,
    pub drives: Vec<DriveSpace>,
    /// Adapter name, driver version, memory (WMI).
    pub gpus: Vec<(String, String, u64)>,
}

/// Validates the bridge's metrics: coherent memory and drive figures, bounded lists.
fn accept_system_status(status: aiwin11_contracts::v1::SystemStatus) -> Option<SystemSnapshot> {
    let drives_ok = status.drives.len() <= 8
        && status.drives.iter().all(|drive| {
            !drive.name.is_empty()
                && drive.name.chars().count() <= 8
                && drive.name.chars().all(|c| !c.is_control())
                && drive.free_bytes <= drive.total_bytes
        });
    let valid = status.memory_total_bytes > 0
        && status.memory_available_bytes <= status.memory_total_bytes
        && status.cpu_load_percent <= 100
        && (1..=1024).contains(&status.logical_processors)
        && drives_ok
        && status.gpus.len() <= 4
        && status.gpus.iter().all(|gpu| {
            gpu.name.chars().count() <= 120
                && gpu.driver_version.chars().count() <= 40
                && !gpu.name.chars().chain(gpu.driver_version.chars()).any(char::is_control)
        });
    valid.then(|| SystemSnapshot {
        memory_total_bytes: status.memory_total_bytes,
        memory_available_bytes: status.memory_available_bytes,
        cpu_load_percent: status.cpu_load_percent,
        logical_processors: status.logical_processors,
        uptime_seconds: status.uptime_seconds,
        drives: status
            .drives
            .into_iter()
            .map(|drive| DriveSpace { name: drive.name, total_bytes: drive.total_bytes, free_bytes: drive.free_bytes })
            .collect(),
        gpus: status.gpus.into_iter().map(|gpu| (gpu.name, gpu.driver_version, gpu.memory_bytes)).collect(),
    })
}

pub fn prewarm_bridge() {
    if let Ok(mut spare) = SPARE_BRIDGE.lock() {
        if spare.is_none() {
            *spare = spawn_bridge();
        }
    }
}

/// Takes the waiting spare if it is still alive, otherwise starts a bridge now.
/// Either way, a new spare is started for the next command.
fn take_bridge() -> Option<std::process::Child> {
    let spare = SPARE_BRIDGE.lock().ok().and_then(|mut spare| spare.take());
    let child = match spare {
        Some(mut child) => {
            if matches!(child.try_wait(), Ok(None)) {
                Some(child)
            } else {
                // The spare exited or cannot be queried: reap it and start a fresh bridge.
                let _ = child.kill();
                let _ = child.wait();
                spawn_bridge()
            }
        }
        None => spawn_bridge(),
    };
    prewarm_bridge();
    child
}

fn dispatch(command: &Command) -> Option<BridgeReply> {
    let bytes = command.encode_to_vec();
    if bytes.len() > MAX_COMMAND_BYTES {
        return None;
    }
    // The bridge stops working at the command deadline, then reports. Wait for that report
    // instead of a fixed delay, so a slow .NET cold start yields a result rather than a kill.
    let deadline = Instant::now() + bridge_wait(command.deadline_ms, unix_millis()?);
    let mut child = take_bridge()?;

    // The bridge reads its whole stdin before writing anything, so this write cannot deadlock.
    // Close the writer to delimit the frame.
    let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(&bytes).is_ok());
    let limit = match command.operation {
        Some(Operation::CaptureForeground(_) | Operation::CaptureWindow(_) | Operation::CaptureDisplay(_)) => {
            MAX_JPEG_BYTES + MAX_MESSAGE_BYTES
        }
        Some(Operation::InspectWindow(_)) => MAX_TREE_REPLY_BYTES,
        Some(Operation::ListProcesses(_) | Operation::GetSystemStatus(_)) => MAX_PROCESS_REPLY_BYTES,
        Some(Operation::ReadClipboard(_) | Operation::RunDiagnostic(_) | Operation::Dictate(_)) => {
            MAX_CLIPBOARD_REPLY_BYTES * 3
        }
        Some(Operation::ListWindows(_) | Operation::ListServices(_)) => MAX_WINDOW_LIST_REPLY_BYTES,
        _ => MAX_MESSAGE_BYTES,
    };
    let mut stdout = child.stdout.take()?;
    let mut reply_bytes = Vec::new();
    loop {
        if !wrote || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        };
        // Drain available data even before exit, because image replies can fill the pipe.
        // Never wait for EOF: a launched application can inherit another copy of the writer.
        let available = pipe_available(&stdout) as usize;
        if available > limit - reply_bytes.len() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        if available > 0 {
            let start = reply_bytes.len();
            reply_bytes.resize(start + available, 0);
            stdout.read_exact(&mut reply_bytes[start..]).ok()?;
        }
        if let Some(status) = status {
            if !status.success() {
                return None;
            }
            break;
        }
        if available == 0 {
            thread::sleep(Duration::from_millis(25));
        }
    }
    let reply = BridgeReply::decode(reply_bytes.as_slice()).ok()?;
    if reply.schema_version != SCHEMA_VERSION || reply.request_id != command.request_id {
        return None;
    }
    Some(reply)
}

fn unix_millis() -> Option<i64> {
    Some(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn bridge_wait(deadline_ms: i64, now_ms: i64) -> Duration {
    let remaining = Duration::from_millis(u64::try_from(deadline_ms.saturating_sub(now_ms)).unwrap_or(0));
    (remaining + REPLY_GRACE).min(MAX_BRIDGE_WAIT)
}

fn pipe_available(stdout: &std::process::ChildStdout) -> u32 {
    let mut available = 0;
    unsafe {
        PeekNamedPipe(stdout.as_raw_handle(), ptr::null_mut(), 0, ptr::null_mut(), &mut available, ptr::null_mut());
    }
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn lists_displays_through_the_broker_and_bridge() {
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let command = core.desktop_command(Operation::ListDisplays(Default::default())).unwrap();
        assert!(matches!(
            core.authorize_dispatch(Policy::default(), Peer::Agent, &command),
            Err(ExecutionError::Denied(_))
        ));
        let displays = core.list_displays().unwrap();
        assert_eq!(displays.iter().filter(|display| display.primary).count(), 1);
        assert!(displays.iter().all(|display| display.width > 0 && display.height > 0));
        assert!(core.activity().unwrap().iter().any(|event| event.kind == "displays.completed"));
        let stale = Display { x: i32::MAX, y: i32::MAX, width: 1, height: 1, primary: false };
        assert!(matches!(core.capture_display(stale), Err(ExecutionError::ExecutionFailed)));
        assert!(core.activity().unwrap().iter().any(|event| event.kind == "display.capture.failed"));
    }

    /// Reads the clipboard; with AIWIN11_TEST_CLIPBOARD=1, also writes and reads back a test text,
    /// then restores the previous plain text.
    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn writes_then_reads_the_clipboard_through_the_broker_and_the_bridge() {
        use crate::broker::Capability;
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let read = Policy::granting(&[Capability::ClipboardRead]);
        let write = Policy::granting(&[Capability::ClipboardWrite]);
        assert_eq!(core.agent_read_clipboard(&task, Policy::default()), Err("WINDOW_ACCESS_DENIED"));
        assert_eq!(core.agent_write_clipboard(&task, read, "x"), Err("CLIPBOARD_ACCESS_DENIED"));
        let saved = core.agent_read_clipboard(&task, read).unwrap();
        // Writing replaces the user's clipboard (rich formats and images are lost): opt-in only.
        if std::env::var("AIWIN11_TEST_CLIPBOARD").as_deref() != Ok("1") {
            eprintln!("clipboard write skipped: set AIWIN11_TEST_CLIPBOARD=1 to test it");
            return;
        }
        let marker = format!("AiWin11 test {}", Uuid::new_v4());
        assert_eq!(core.agent_write_clipboard(&task, write, &marker), Ok(true));
        assert_eq!(core.agent_read_clipboard(&task, read).unwrap().text, marker);
        if !saved.text.is_empty() && !saved.truncated {
            assert_eq!(core.agent_write_clipboard(&task, write, &saved.text), Ok(true), "restore");
        }
    }

    #[test]
    fn service_lists_from_the_bridge_are_validated() {
        use aiwin11_contracts::v1::{ServiceEntry, ServiceList};
        let entry = |name: &str, mode: StartMode| ServiceEntry {
            name: name.into(),
            display_name: "Service".into(),
            running: false,
            start_mode: mode as i32,
        };
        let good = ServiceList {
            services: vec![entry("Spooler", StartMode::Automatic)],
            total: 200,
            running: 90,
            stopped_automatic: 1,
        };
        assert_eq!(accept_service_list(good.clone()).unwrap().services[0].name, "Spooler");
        for bad in [
            ServiceList { running: 201, ..good.clone() },
            ServiceList { services: vec![entry("", StartMode::Manual)], ..good.clone() },
            ServiceList { services: vec![entry("a\nb", StartMode::Manual)], ..good.clone() },
            ServiceList { services: vec![entry("x", StartMode::Unspecified)], ..good.clone() },
            ServiceList { services: (0..21).map(|_| entry("x", StartMode::Manual)).collect(), ..good.clone() },
        ] {
            assert_eq!(accept_service_list(bad.clone()), None, "{bad:?}");
        }
    }

    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn lists_services_through_the_broker_and_the_bridge() {
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        assert_eq!(core.agent_list_services(&task, Policy::default()), Err("WINDOW_ACCESS_DENIED"));
        let snapshot =
            core.agent_list_services(&task, Policy::granting(&[crate::broker::Capability::ServiceList])).unwrap();
        assert!(snapshot.total > 0 && snapshot.running > 0 && !snapshot.services.is_empty(), "{snapshot:?}");
    }

    #[test]
    fn window_lists_from_the_bridge_are_validated() {
        use aiwin11_contracts::v1::{WindowInfo, WindowList};
        let info = |window, title: &str| WindowInfo {
            window,
            process_id: 7,
            title: title.into(),
            process_name: "notepad".into(),
            minimized: false,
        };
        let listed = accept_window_list(WindowList { windows: vec![info(1, "Sans titre"), info(2, "Notes")] }).unwrap();
        assert_eq!(listed[1].target.title, "Notes");
        for bad in [
            WindowList { windows: vec![info(0, "x")] },
            WindowList { windows: vec![info(1, "")] },
            WindowList { windows: vec![info(1, "a\nb")] },
            WindowList { windows: vec![info(1, &"t".repeat(121))] },
            WindowList { windows: (1..=21).map(|window| info(window, "x")).collect() },
        ] {
            assert_eq!(accept_window_list(bad.clone()), None, "{bad:?}");
        }
    }

    /// Lists windows, then minimizes and restores a Character Map window owned by the test.
    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn lists_then_arranges_a_window_through_the_broker_and_the_bridge() {
        use crate::broker::Capability;
        let system = std::env::var_os("SystemRoot").expect("SystemRoot");
        let mut charmap = std::process::Command::new(std::path::Path::new(&system).join(r"System32\charmap.exe"))
            .spawn()
            .expect("start Character Map");
        thread::sleep(Duration::from_secs(2));
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let result = (|| {
            assert_eq!(core.agent_list_windows(&task, Policy::default()), Err("WINDOW_ACCESS_DENIED"));
            let listed = core.agent_list_windows(&task, Policy::granting(&[Capability::WindowList]))?;
            let target = listed
                .iter()
                .find(|window| window.target.process_id == charmap.id())
                .map(|window| window.target.clone())
                .ok_or("CHARMAP_NOT_LISTED")?;
            let arrange = Policy::granting(&[Capability::WindowArrange]);
            let minimized = core.agent_arrange_window(&task, arrange, &target, Arrangement::Minimize)?;
            let restored = core.agent_arrange_window(&task, arrange, &target, Arrangement::Restore)?;
            Ok::<_, &'static str>((minimized, restored))
        })();
        let _ = charmap.kill();
        let _ = charmap.wait();
        assert_eq!(result, Ok((true, true)));
    }

    #[test]
    fn system_status_from_the_bridge_is_validated() {
        use aiwin11_contracts::v1::{DriveStatus, SystemStatus};
        let good = SystemStatus {
            memory_total_bytes: 32 << 30,
            memory_available_bytes: 20 << 30,
            cpu_load_percent: 12,
            logical_processors: 24,
            uptime_seconds: 3600,
            drives: vec![DriveStatus { name: "C:\\".into(), total_bytes: 1 << 40, free_bytes: 1 << 39 }],
            gpus: vec![aiwin11_contracts::v1::GpuInfo {
                name: "NVIDIA GeForce RTX 3090".into(),
                driver_version: "32.0".into(),
                memory_bytes: 4 << 30,
            }],
        };
        assert_eq!(accept_system_status(good.clone()).unwrap().drives[0].name, "C:\\");
        for bad in [
            SystemStatus { memory_available_bytes: 64 << 30, ..good.clone() },
            SystemStatus { cpu_load_percent: 101, ..good.clone() },
            SystemStatus { logical_processors: 0, ..good.clone() },
            SystemStatus { memory_total_bytes: 0, memory_available_bytes: 0, ..good.clone() },
            SystemStatus {
                drives: vec![DriveStatus { name: "C:\\".into(), total_bytes: 1, free_bytes: 2 }],
                ..good.clone()
            },
            SystemStatus {
                drives: vec![DriveStatus { name: "".into(), total_bytes: 1, free_bytes: 1 }],
                ..good.clone()
            },
        ] {
            assert_eq!(accept_system_status(bad.clone()), None, "{bad:?}");
        }
    }

    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn reads_the_system_status_through_the_broker_and_the_bridge() {
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        assert_eq!(core.agent_system_status(&task, Policy::default()), Err("WINDOW_ACCESS_DENIED"));
        let status =
            core.agent_system_status(&task, Policy::granting(&[crate::broker::Capability::SystemStatus])).unwrap();
        assert!(status.memory_available_bytes > 0 && !status.drives.is_empty(), "{status:?}");
    }

    #[test]
    fn process_lists_from_the_bridge_are_validated() {
        use aiwin11_contracts::v1::{ProcessEntry, ProcessList};
        let entry = |pid, name: &str, memory| ProcessEntry {
            process_id: pid,
            name: name.into(),
            working_set_bytes: memory,
            cpu_time_ms: 1,
            has_window: false,
        };
        let good = ProcessList { processes: vec![entry(10, "chrome", 900), entry(11, "code", 500)], total: 120 };
        let snapshot = accept_process_list(good).unwrap();
        assert_eq!(snapshot.total, 120);
        assert_eq!(snapshot.processes[0].name, "chrome");
        for bad in [
            ProcessList { processes: vec![entry(10, "a", 1), entry(11, "b", 2)], total: 2 },
            ProcessList { processes: vec![entry(0, "Idle", 1)], total: 1 },
            ProcessList { processes: vec![entry(4, "", 1)], total: 1 },
            ProcessList { processes: vec![entry(4, "a\nb", 1)], total: 1 },
            ProcessList { processes: vec![entry(4, &"x".repeat(65), 1)], total: 1 },
            ProcessList { processes: vec![entry(4, "a", 1)], total: 0 },
            ProcessList { processes: (1..=16).map(|pid| entry(pid, "p", 1)).collect(), total: 16 },
        ] {
            assert_eq!(accept_process_list(bad.clone()), None, "{bad:?}");
        }
    }

    #[test]
    #[ignore = "requires the published bridge and an interactive Windows session"]
    fn lists_processes_through_the_broker_and_the_bridge() {
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let denied = core.agent_list_processes(&task, Policy::default());
        assert_eq!(denied, Err("WINDOW_ACCESS_DENIED"));
        let snapshot =
            core.agent_list_processes(&task, Policy::granting(&[crate::broker::Capability::ProcessList])).unwrap();
        assert!(!snapshot.processes.is_empty() && snapshot.total as usize >= snapshot.processes.len());
        assert!(
            snapshot.processes.iter().any(|process| process.process_id == std::process::id())
                || snapshot.processes.len() == AGENT_PROCESS_LIMIT as usize
        );
        let kinds: Vec<_> = core.activity().unwrap().into_iter().map(|event| event.kind).collect();
        assert!(kinds.contains(&"process.list.completed".to_owned()), "{kinds:?}");
    }

    fn node(parent: u32, name: &str) -> aiwin11_contracts::v1::UiNode {
        aiwin11_contracts::v1::UiNode { parent, control_type: 50032, enabled: true, focused: false, name: name.into() }
    }

    #[test]
    fn named_trees_are_shape_checked() {
        let tree = |nodes, truncated| UiTree { nodes, truncated };
        let (nodes, truncated) =
            named_tree(tree(vec![node(0, "Fenêtre"), node(1, "Fichier"), node(2, "")], true)).unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[1].name, "Fichier");
        assert!(truncated);
        for invalid in [
            vec![],
            vec![node(1, "root points to itself")],
            vec![node(0, "a"), node(2, "forward parent")],
            vec![node(0, &"x".repeat(81))],
            vec![node(0, "a"); MAX_AGENT_NODES + 1],
        ] {
            assert!(named_tree(tree(invalid, false)).is_none());
        }
    }

    /// An expired command makes the bridge answer TIMEOUT without touching the desktop.
    fn expired_command() -> Command {
        let id = Uuid::new_v4().to_string();
        Command {
            schema_version: SCHEMA_VERSION,
            request_id: id.clone(),
            session_id: id.clone(),
            task_id: id.clone(),
            correlation_id: id.clone(),
            authenticated_actor_id: "test".into(),
            idempotency_key: id,
            deadline_ms: unix_millis().unwrap() - 1_000,
            operation: Some(Operation::LaunchApplication(aiwin11_contracts::v1::LaunchApplication {
                application: aiwin11_contracts::v1::launch_application::Application::Notepad as i32,
            })),
        }
    }

    #[test]
    #[ignore = "requires the published bridge beside the test binary"]
    fn a_prewarmed_bridge_answers_without_startup_cost() {
        prewarm_bridge();
        // Let the spare finish starting, however slow its first start is.
        thread::sleep(Duration::from_secs(25));
        let started = Instant::now();
        let reply = dispatch(&expired_command()).expect("bridge reply");
        let elapsed = started.elapsed();
        assert_eq!(reply.status(), Status::Timeout);
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
        assert!(SPARE_BRIDGE.lock().unwrap().is_some(), "a new spare is waiting for the next command");
    }

    #[test]
    fn bridge_wait_follows_the_command_deadline_with_bounds() {
        assert_eq!(bridge_wait(10_000, 2_000), Duration::from_secs(10));
        assert_eq!(bridge_wait(1_000, 2_000), REPLY_GRACE, "an expired command still gets time to report");
        assert_eq!(bridge_wait(i64::MAX, 0), MAX_BRIDGE_WAIT);
        assert_eq!(bridge_wait(i64::MIN, i64::MAX), REPLY_GRACE);
    }
}
