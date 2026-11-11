use std::{
    io::{Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command as Process, Stdio},
    ptr, sync::Mutex, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aiwin11_contracts::{
    v1::{
        command::Operation, result::Status, AnalyzeCapture, CharacterMapDigit, CaptureForeground, Command,
        InspectForeground, InspectWindow, LocateWindow, Result as BridgeReply, UiTree,
    },
    Message, SCHEMA_VERSION,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use serde::Serialize;
use uuid::Uuid;

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
const MAX_JPEG_BYTES: usize = 256 * 1024;
const MAX_TREE_REPLY_BYTES: usize = 32 * 1024;
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
        let command = self.desktop_command(Operation::CharacterMapDigit(CharacterMapDigit { digit: Some(digit) }))
            .map_err(|_| "INPUT_UNAVAILABLE")?;
        let task = self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|_| "INPUT_DENIED")?;
        let outcome = dispatch(&command).and_then(|reply| {
            if reply.ui_tree.is_some() || reply.window_capture.is_some() || reply.process_id.is_some() {
                return None;
            }
            match (reply.status(), reply.error.as_ref().map(|error| error.code.as_str())) {
                (Status::Success, None) => Some(Ok(true)),
                (Status::Partial, Some("field_unverified")) => Some(Ok(false)),
                (Status::Denied, Some("character_map_unavailable")) => Some(Err("CHARACTER_MAP_UNAVAILABLE")),
                _ => None,
            }
        }).unwrap_or(Err("INPUT_UNAVAILABLE"));
        let (kind, code) = match outcome {
            Ok(true) => ("input.completed", "digit_verified"),
            Ok(false) => ("input.partial", "field_unverified"),
            Err(code) => ("input.failed", code),
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| "INPUT_AUDIT_UNAVAILABLE")?;
        outcome
    }

    pub fn analyze_capture(
        core: &Mutex<Self>, model: &str, jpeg: &[u8],
    ) -> Result<String, &'static str> {
        if !valid_jpeg(jpeg) {
            return Err("INVALID_CAPTURE");
        }
        if model.is_empty() || model.len() > 256 {
            return Err("VISION_MODEL_REQUIRED");
        }
        let task = {
            let mut core = core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
            let command = core.desktop_command(Operation::AnalyzeCapture(AnalyzeCapture {}))
                .map_err(|_| "VISION_UNAVAILABLE")?;
            core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
                .map_err(|_| "VISION_NOT_AUTHORIZED")?
        };
        let result = Ollama::connect().and_then(|provider| provider.describe_capture(model, jpeg));
        let (kind, code) = match &result {
            Ok(_) => ("vision.completed", "success"),
            Err(error) => ("vision.failed", error.vision_code()),
        };
        core.lock().map_err(|_| "CORE_UNAVAILABLE")?
            .audit(task.as_deref(), kind, code).map_err(|_| "VISION_AUDIT_UNAVAILABLE")?;
        result.map_err(|error: GatewayError| error.vision_code())
    }

    pub(crate) fn desktop_command(&self, operation: Operation) -> Result<Command, ExecutionError> {
        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(|_| ExecutionError::ExecutionFailed)?;
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
    pub(crate) fn agent_command(&self, task: &str, operation: Operation) -> Result<Command, ExecutionError> {
        let mut command = self.desktop_command(operation)?;
        command.task_id = task.into();
        command.correlation_id = task.into();
        command.authenticated_actor_id = "agent".into();
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
            let valid = info.window != 0 && info.process_id != 0
                && bounded(&info.title, 120) && bounded(&info.process_name, 64);
            valid.then_some(WindowTarget {
                window: info.window, process_id: info.process_id,
                title: info.title, process_name: info.process_name,
            })
        })
    }

    /// Agent tool, step 2: reads the named control tree of exactly that window.
    /// The host grants `Capability::WindowInspect` only after the user approved it.
    pub(crate) fn agent_inspect_window(
        &mut self, task: &str, policy: Policy, target: &WindowTarget,
    ) -> Result<(Vec<NamedNode>, bool), &'static str> {
        let operation = Operation::InspectWindow(InspectWindow { window: target.window, process_id: target.process_id });
        self.agent_bridge_call(task, policy, operation, "window.inspect", |reply| {
            if reply.window_info.is_some() || reply.window_capture.is_some() || reply.process_id.is_some() {
                return None;
            }
            named_tree(reply.ui_tree?)
        })
    }

    fn agent_bridge_call<T>(
        &mut self, task: &str, policy: Policy, operation: Operation, event: &str,
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
        self.audit(task_id.as_deref(), &format!("{event}.{suffix}"), code)
            .map_err(|_| "WINDOW_AUDIT_UNAVAILABLE")?;
        result.ok_or("WINDOW_UNAVAILABLE")
    }

    pub fn capture_foreground(&mut self) -> Result<Capture, ExecutionError> {
        let command = self.desktop_command(Operation::CaptureForeground(CaptureForeground {}))?;
        let task = self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)?;
        let capture = dispatch(&command).and_then(|reply| {
            if reply.status() != Status::Success || reply.error.is_some()
                || reply.process_id.is_some() || reply.ui_tree.is_some()
            {
                return None;
            }
            let frame = reply.window_capture?;
            if !(1..=1024).contains(&frame.width) || !(1..=1024).contains(&frame.height)
                || !valid_jpeg(&frame.jpeg)
            {
                return None;
            }
            Some(Capture { jpeg: frame.jpeg, width: frame.width, height: frame.height })
        });
        let (kind, code) = if capture.is_some() {
            ("capture.completed", "success")
        } else {
            ("capture.failed", "capture_unavailable")
        };
        self.audit(task.as_deref(), kind, code).map_err(|_| ExecutionError::OutcomeUnrecorded)?;
        capture.ok_or(ExecutionError::ExecutionFailed)
    }

    pub fn inspect_foreground(&mut self) -> Result<Inspection, ExecutionError> {
        let command = self.desktop_command(Operation::InspectForeground(InspectForeground {}))?;
        let task = self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)?;
        let tree = dispatch(&command).and_then(|reply| {
            if reply.status() != Status::Success || reply.error.is_some()
                || reply.process_id.is_some() || reply.window_capture.is_some() {
                return None;
            }
            let tree = reply.ui_tree?;
            if tree.nodes.is_empty() || tree.nodes.len() > 40
                || tree.nodes.iter().enumerate().any(|(i, node)| {
                    (i == 0 && node.parent != 0) || (i > 0 && !(1..=i as u32).contains(&node.parent))
                })
            {
                return None;
            }
            Some(Inspection {
                truncated: tree.truncated,
                nodes: tree.nodes.into_iter().map(|node| InspectionNode {
                    parent: node.parent, control_type: node.control_type,
                    enabled: node.enabled, focused: node.focused,
                }).collect(),
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

    pub fn execute_windows(
        &mut self,
        policy: Policy,
        peer: Peer,
        command: &Command,
    ) -> Result<u32, ExecutionError> {
        self.execute(policy, peer, command, || {
            let mut trusted_command = command.clone();
            trusted_command.authenticated_actor_id = "agent".into();
            match dispatch(&trusted_command) {
                Some(reply) if reply.ui_tree.is_none() && reply.window_capture.is_none() => match (reply.status(), reply.process_id) {
                    (Status::Success, Some(pid)) if pid != 0 && reply.error.is_none() => LaunchOutcome::Verified(pid),
                    (Status::Partial, None) => LaunchOutcome::Unverified,
                    _ => LaunchOutcome::Failed,
                },
                _ => LaunchOutcome::Failed,
            }
        })
    }
}

/// Checks the shape of an agent inspection: 1 to 80 nodes, parents pointing backwards
/// (one-based, zero for the root), names of at most 80 characters.
fn named_tree(tree: UiTree) -> Option<(Vec<NamedNode>, bool)> {
    let valid = !tree.nodes.is_empty() && tree.nodes.len() <= MAX_AGENT_NODES
        && tree.nodes.iter().enumerate().all(|(index, node)| {
            (if index == 0 { node.parent == 0 } else { (1..=index as u32).contains(&node.parent) })
                && node.name.chars().count() <= 80
        });
    valid.then(|| (
        tree.nodes.into_iter().map(|node| NamedNode {
            parent: node.parent, control_type: node.control_type,
            enabled: node.enabled, focused: node.focused, name: node.name,
        }).collect(),
        tree.truncated,
    ))
}

fn valid_jpeg(jpeg: &[u8]) -> bool {
    jpeg.len() <= MAX_JPEG_BYTES
        && jpeg.starts_with(&[0xff, 0xd8])
        && jpeg.ends_with(&[0xff, 0xd9])
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
    if bytes.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    // The bridge stops working at the command deadline, then reports. Wait for that report
    // instead of a fixed delay, so a slow .NET cold start yields a result rather than a kill.
    let deadline = Instant::now() + bridge_wait(command.deadline_ms, unix_millis()?);
    let mut child = take_bridge()?;

    // The command fits in the anonymous pipe buffer. Close the writer to delimit the frame.
    let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(&bytes).is_ok());
    let limit = match command.operation {
        Some(Operation::CaptureForeground(_)) => MAX_JPEG_BYTES + MAX_MESSAGE_BYTES,
        Some(Operation::InspectWindow(_)) => MAX_TREE_REPLY_BYTES,
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
        PeekNamedPipe(
            stdout.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
        );
    }
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(parent: u32, name: &str) -> aiwin11_contracts::v1::UiNode {
        aiwin11_contracts::v1::UiNode { parent, control_type: 50032, enabled: true, focused: false, name: name.into() }
    }

    #[test]
    fn named_trees_are_shape_checked() {
        let tree = |nodes, truncated| UiTree { nodes, truncated };
        let (nodes, truncated) = named_tree(tree(vec![node(0, "Fenêtre"), node(1, "Fichier"), node(2, "")], true)).unwrap();
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
            schema_version: SCHEMA_VERSION, request_id: id.clone(), session_id: id.clone(), task_id: id.clone(),
            correlation_id: id.clone(), authenticated_actor_id: "test".into(), idempotency_key: id,
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
