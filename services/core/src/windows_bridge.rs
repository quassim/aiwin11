use std::{
    io::{Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command as Process, Stdio},
    ptr, sync::Mutex, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aiwin11_contracts::{
    v1::{command::Operation, result::Status, AnalyzeCapture, CharacterMapDigit, CaptureForeground, Command, InspectForeground, Result as BridgeReply},
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
const MAX_MESSAGE_BYTES: usize = 1024;
const MAX_JPEG_BYTES: usize = 256 * 1024;

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

fn valid_jpeg(jpeg: &[u8]) -> bool {
    jpeg.len() <= MAX_JPEG_BYTES
        && jpeg.starts_with(&[0xff, 0xd8])
        && jpeg.ends_with(&[0xff, 0xd9])
}

fn dispatch(command: &Command) -> Option<BridgeReply> {
    let bytes = command.encode_to_vec();
    if bytes.len() > MAX_MESSAGE_BYTES {
        return None;
    }
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let deadline = Instant::now() + Duration::from_secs(7);
    let mut child = Process::new(directory.join("AiWin11.Windows.exe"))
        .current_dir(directory)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // The command fits in the anonymous pipe buffer. Close the writer to delimit the frame.
    let wrote = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(&bytes).is_ok());
    let limit = match command.operation {
        Some(Operation::CaptureForeground(_)) => MAX_JPEG_BYTES + MAX_MESSAGE_BYTES,
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
