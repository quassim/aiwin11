use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command as Process, Stdio},
    sync::{atomic::{AtomicU8, Ordering}, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aiwin11_contracts::{
    v1::{command::Operation, launch_application::Application, Command, LaunchApplication},
    SCHEMA_VERSION,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{broker::{ExecutionError, Peer, Policy}, model_gateway::Ollama, openai, Core};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Ollama,
    OpenAi,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequest {
    pub provider: Provider,
    pub prompt: String,
    pub model: String,
    pub approved: bool,
    pub cloud_approved: bool,
}

const IDLE: u8 = 0;
const PLANNING: u8 = 1;
const CANCELLED: u8 = 2;
const DISPATCHING: u8 = 3;
const MAX_AGENT_INPUT: usize = 4096;

#[derive(Default)]
pub struct TaskSlot {
    phase: AtomicU8,
}

impl TaskSlot {
    pub fn cancel(&self) -> bool {
        self.phase.compare_exchange(PLANNING, CANCELLED, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn run(
        &self,
        core: &Mutex<Core>,
        request: &TaskRequest,
        progress: impl Fn(&'static str),
    ) -> Result<u32, &'static str> {
        let prompt = &request.prompt;
        let normalized = prompt.trim().trim_end_matches(['.', '!', '?']).trim();
        if prompt.len() > 256 || !normalized.eq_ignore_ascii_case("Ouvre le Bloc-notes") {
            return Err("UNSUPPORTED_REQUEST");
        }
        if request.model.is_empty() || request.model.len() > 256 {
            return Err("MODEL_REQUIRED");
        }
        if self.phase.compare_exchange(IDLE, PLANNING, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err("TASK_BUSY");
        }
        let _reset = ResetSlot(self);
        let task = Uuid::new_v4().to_string();
        record(core, &task, "task.created")?;
        progress("task.created");
        let outcome = self.plan_and_execute(core, &task, request, &progress);
        // Arbitrate cancellation even when the provider or agent returns an error.
        let outcome = match self.phase.compare_exchange(
            PLANNING, DISPATCHING, Ordering::AcqRel, Ordering::Acquire,
        ) {
            Err(CANCELLED) => Err("TASK_CANCELLED"),
            _ => outcome,
        };
        record(core, &task, match outcome {
            Ok(_) => "task.completed",
            Err("TASK_CANCELLED") => "task.cancelled",
            Err("WINDOW_UNVERIFIED") => "task.partial",
            Err(_) => "task.failed",
        })?;
        outcome
    }

    fn plan_and_execute(
        &self,
        core: &Mutex<Core>,
        task: &str,
        request: &TaskRequest,
        progress: &impl Fn(&'static str),
    ) -> Result<u32, &'static str> {
        record(core, task, "task.planning")?;
        progress("task.planning");
        let calls = match request.provider {
            Provider::Ollama => Ollama::connect()
                .and_then(|provider| provider.propose_notepad(&request.model))
                .map_err(|_| "MODEL_UNAVAILABLE_OR_INVALID")?,
            Provider::OpenAi => {
                if !request.cloud_approved {
                    record(core, task, "model.cloud.denied")?;
                    return Err("CLOUD_APPROVAL_REQUIRED");
                }
                record(core, task, "model.cloud.authorized")?;
                if self.phase.load(Ordering::Acquire) == CANCELLED {
                    return Err("TASK_CANCELLED");
                }
                openai::propose_notepad(&request.model)?
            }
        };
        if self.phase.load(Ordering::Acquire) == CANCELLED {
            return Err("TASK_CANCELLED");
        }
        run_agent(calls)?;
        record(core, task, "agent.tool.requested")?;
        progress("agent.tool.requested");
        let mut core = core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
        let deadline_ms = SystemTime::now().duration_since(UNIX_EPOCH)
            .map_err(|_| "CLOCK_UNAVAILABLE")?.as_millis() as i64 + 8_000;
        let command = Command {
            schema_version: SCHEMA_VERSION,
            request_id: Uuid::new_v4().to_string(),
            session_id: core.session_id().into(),
            task_id: task.into(),
            correlation_id: task.into(),
            authenticated_actor_id: "agent".into(),
            deadline_ms,
            idempotency_key: task.into(),
            operation: Some(Operation::LaunchApplication(LaunchApplication {
                application: Application::Notepad as i32,
            })),
        };
        self.phase.compare_exchange(PLANNING, DISPATCHING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "TASK_CANCELLED")?;
        core.execute_windows(Policy { allow_agent_to_launch_notepad: request.approved }, Peer::Agent, &command)
            .map_err(|error| match error {
                ExecutionError::Denied(_) => "ACTION_DENIED",
                ExecutionError::WindowUnverified => "WINDOW_UNVERIFIED",
                ExecutionError::AuditUnavailable => "AUDIT_UNAVAILABLE",
                ExecutionError::OutcomeUnrecorded => "OUTCOME_UNRECORDED",
                ExecutionError::ExecutionFailed => "BRIDGE_FAILED",
            })
    }
}

struct ResetSlot<'a>(&'a TaskSlot);

impl Drop for ResetSlot<'_> {
    fn drop(&mut self) {
        self.0.phase.store(IDLE, Ordering::Release);
    }
}

fn record(core: &Mutex<Core>, task: &str, kind: &str) -> Result<(), &'static str> {
    let core = core.lock().map_err(|_| "CORE_UNAVAILABLE")?;
    core.db.execute(
        "INSERT INTO events (id, session_id, task_id, kind, source) VALUES (?1, ?2, ?3, ?4, 'core')",
        params![Uuid::new_v4().to_string(), core.session_id(), task, kind],
    ).map_err(|_| "AUDIT_UNAVAILABLE")?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentReply {
    version: u32,
    operation: String,
}

fn run_agent(calls: Value) -> Result<(), &'static str> {
    let python = std::env::var_os("AIWIN11_PYTHON").map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .ok_or("AGENT_UNAVAILABLE")?;
    let executable = std::env::current_exe().map_err(|_| "AGENT_UNAVAILABLE")?;
    let directory = executable.parent().ok_or("AGENT_UNAVAILABLE")?;
    let script = directory.join("aiwin11_agent.py");
    if !script.is_file() {
        return Err("AGENT_UNAVAILABLE");
    }
    let bytes = json!({ "version": 1, "calls": calls }).to_string().into_bytes();
    if bytes.len() > MAX_AGENT_INPUT {
        return Err("AGENT_REJECTED");
    }
    let mut process = Process::new(python);
    process.env_clear();
    if let Some(root) = std::env::var_os("SystemRoot") {
        process.env("SystemRoot", root);
    }
    let mut child = process.args(["-I", "-S", "-B"])
        .arg(script)
        .current_dir(directory)
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn().map_err(|_| "AGENT_UNAVAILABLE")?;
    let wrote = child.stdin.take().is_some_and(|mut input| input.write_all(&bytes).is_ok());
    let deadline = Instant::now() + Duration::from_secs(4);
    let success = loop {
        if !wrote || Instant::now() >= deadline {
            break false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => break false,
        }
    };
    if !success {
        let _ = child.kill();
        let _ = child.wait();
        return Err("AGENT_REJECTED");
    }
    let mut output = Vec::new();
    child.stdout.take().ok_or("AGENT_REJECTED")?.take(257)
        .read_to_end(&mut output).map_err(|_| "AGENT_REJECTED")?;
    if output.len() > 256 {
        return Err("AGENT_REJECTED");
    }
    let reply: AgentReply = serde_json::from_slice(&output).map_err(|_| "AGENT_REJECTED")?;
    if reply.version != 1 || reply.operation != "launch_notepad" {
        return Err("AGENT_REJECTED");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    #[ignore = "requires Ollama, the installed companions and an interactive Windows desktop"]
    fn opens_notepad_only_after_broker_authorization() {
        let model = std::env::var("AIWIN11_TEST_MODEL").expect("set AIWIN11_TEST_MODEL");
        let core = Mutex::new(Core::start(Connection::open_in_memory().unwrap()).unwrap());
        let task = TaskSlot::default();
        let mut request = TaskRequest {
            provider: Provider::Ollama,
            prompt: "Ouvre le Bloc-notes".into(),
            model,
            approved: false,
            cloud_approved: false,
        };
        assert_eq!(task.run(&core, &request, |_| {}), Err("ACTION_DENIED"));
        let denied = core.lock().unwrap().activity().unwrap();
        assert!(denied.iter().any(|event| event.kind == "policy.denied"));
        assert!(!denied.iter().any(|event| event.kind.starts_with("tool.execution.")));

        request.approved = true;
        let stages = Mutex::new(Vec::new());
        let pid = task.run(&core, &request, |stage| stages.lock().unwrap().push(stage)).unwrap();
        assert!(pid > 0);
        assert_eq!(
            *stages.lock().unwrap(),
            ["task.created", "task.planning", "agent.tool.requested"]
        );
        let activity = core.lock().unwrap().activity().unwrap();
        assert_eq!(activity[0].kind, "task.completed");
        assert_eq!(activity[1].kind, "tool.execution.completed");
        assert_eq!(activity[2].kind, "policy.allowed");
    }
}
