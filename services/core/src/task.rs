use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{
    broker::{ExecutionError, Peer, Policy},
    model_gateway::{Ollama, Proposal, MAX_REPLY_CHARS},
    openai, Core,
};

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
    /// Scoped approval: lets the broker allow a Notepad launch proposed for this request.
    pub approved: bool,
    /// Scoped approval: lets this request's prompt be sent to the cloud provider.
    pub cloud_approved: bool,
}

/// What a finished request produced.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskOutcome {
    #[serde(rename_all = "camelCase")]
    Launched { application: &'static str, process_id: u32 },
    Reply { text: String },
}

/// A decision accepted by both the Python agent and the host.
#[derive(Debug, PartialEq, Eq)]
enum PlannedAction {
    Launch(Application),
    Reply(String),
}

const IDLE: u8 = 0;
const PLANNING: u8 = 1;
const CANCELLED: u8 = 2;
const DISPATCHING: u8 = 3;
pub const MAX_PROMPT_CHARS: usize = 2000;
const AGENT_PROTOCOL: u32 = 2;
const MAX_AGENT_INPUT: usize = 32 * 1024;
const MAX_AGENT_OUTPUT: usize = 32 * 1024;

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
    ) -> Result<TaskOutcome, &'static str> {
        let prompt = validate_prompt(&request.prompt)?;
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
        let outcome = self.plan_and_execute(core, &task, prompt, request, &progress);
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
        prompt: &str,
        request: &TaskRequest,
        progress: &impl Fn(&'static str),
    ) -> Result<TaskOutcome, &'static str> {
        record(core, task, "task.planning")?;
        progress("task.planning");
        let proposal = match request.provider {
            Provider::Ollama => Ollama::connect()
                .and_then(|provider| provider.propose(&request.model, prompt))
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
                openai::propose(&request.model, prompt)?
            }
        };
        if self.phase.load(Ordering::Acquire) == CANCELLED {
            return Err("TASK_CANCELLED");
        }
        let application = match run_agent(&proposal)? {
            PlannedAction::Reply(text) => {
                // The reply text stays out of the journal, like prompts.
                record(core, task, "agent.reply")?;
                progress("agent.reply");
                return Ok(TaskOutcome::Reply { text });
            }
            PlannedAction::Launch(application) => application,
        };
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
                application: application as i32,
            })),
        };
        self.phase.compare_exchange(PLANNING, DISPATCHING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "TASK_CANCELLED")?;
        core.execute_windows(Policy { allow_agent_to_launch_notepad: request.approved }, Peer::Agent, &command)
            .map(|process_id| TaskOutcome::Launched { application: "notepad", process_id })
            .map_err(|error| match error {
                ExecutionError::Denied(_) => "ACTION_DENIED",
                ExecutionError::WindowUnverified => "WINDOW_UNVERIFIED",
                ExecutionError::AuditUnavailable => "AUDIT_UNAVAILABLE",
                ExecutionError::OutcomeUnrecorded => "OUTCOME_UNRECORDED",
                ExecutionError::ExecutionFailed => "BRIDGE_FAILED",
            })
    }
}

fn validate_prompt(prompt: &str) -> Result<&str, &'static str> {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS || prompt.contains('\0') {
        return Err("PROMPT_INVALID");
    }
    Ok(prompt)
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
    action: String,
    tool: Option<String>,
    arguments: Option<Map<String, Value>>,
    text: Option<String>,
}

/// Validates the agent decision again on the trusted side.
fn accept_agent_reply(bytes: &[u8]) -> Result<PlannedAction, &'static str> {
    let reply: AgentReply = serde_json::from_slice(bytes).map_err(|_| "AGENT_REJECTED")?;
    if reply.version != AGENT_PROTOCOL {
        return Err("AGENT_REJECTED");
    }
    match (reply.action.as_str(), reply.tool.as_deref(), reply.arguments, reply.text) {
        ("tool", Some("launch_application"), Some(arguments), None)
            if arguments.len() == 1
                && arguments.get("application").and_then(Value::as_str) == Some("notepad") =>
        {
            Ok(PlannedAction::Launch(Application::Notepad))
        }
        ("reply", None, None, Some(text))
            if !text.trim().is_empty() && text.chars().count() <= MAX_REPLY_CHARS =>
        {
            Ok(PlannedAction::Reply(text))
        }
        _ => Err("AGENT_REJECTED"),
    }
}

fn run_agent(proposal: &Proposal) -> Result<PlannedAction, &'static str> {
    let python = std::env::var_os("AIWIN11_PYTHON").map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .ok_or("AGENT_UNAVAILABLE")?;
    let executable = std::env::current_exe().map_err(|_| "AGENT_UNAVAILABLE")?;
    let directory = executable.parent().ok_or("AGENT_UNAVAILABLE")?;
    let script = directory.join("aiwin11_agent.py");
    if !script.is_file() {
        return Err("AGENT_UNAVAILABLE");
    }
    invoke_agent(python.as_path(), &script, directory, proposal)
}

fn invoke_agent(
    python: &Path, script: &Path, directory: &Path, proposal: &Proposal,
) -> Result<PlannedAction, &'static str> {
    let bytes = json!({ "version": AGENT_PROTOCOL, "proposal": proposal.to_agent_message() })
        .to_string().into_bytes();
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
    // Drain stdout concurrently so a reply larger than the pipe buffer cannot stall the child.
    let reader = child.stdout.take().map(|stdout| thread::spawn(move || {
        let mut output = Vec::new();
        stdout.take(MAX_AGENT_OUTPUT as u64 + 1).read_to_end(&mut output).map(|_| output)
    }));
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
    }
    let _ = child.wait();
    let output = reader.ok_or("AGENT_REJECTED")?.join()
        .map_err(|_| "AGENT_REJECTED")?.map_err(|_| "AGENT_REJECTED")?;
    if !success || output.len() > MAX_AGENT_OUTPUT {
        return Err("AGENT_REJECTED");
    }
    accept_agent_reply(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn prompts_are_trimmed_and_bounded() {
        assert_eq!(validate_prompt("  Ouvre le Bloc-notes \n"), Ok("Ouvre le Bloc-notes"));
        assert_eq!(validate_prompt(&"é".repeat(MAX_PROMPT_CHARS)).map(str::len), Ok(MAX_PROMPT_CHARS * 2));
        for invalid in ["", "   ", "a\0b"] {
            assert_eq!(validate_prompt(invalid), Err("PROMPT_INVALID"));
        }
        assert_eq!(validate_prompt(&"a".repeat(MAX_PROMPT_CHARS + 1)), Err("PROMPT_INVALID"));
    }

    #[test]
    fn host_accepts_only_registered_agent_decisions() {
        let accept = |value: Value| accept_agent_reply(value.to_string().as_bytes());
        assert_eq!(
            accept(json!({"version":2,"action":"tool","tool":"launch_application","arguments":{"application":"notepad"}})),
            Ok(PlannedAction::Launch(Application::Notepad))
        );
        assert_eq!(
            accept(json!({"version":2,"action":"reply","text":"Bonjour"})),
            Ok(PlannedAction::Reply("Bonjour".into()))
        );
        for invalid in [
            json!({"version":1,"action":"reply","text":"Bonjour"}),
            json!({"version":2,"action":"reply","text":"  "}),
            json!({"version":2,"action":"reply","text":"a".repeat(MAX_REPLY_CHARS + 1)}),
            json!({"version":2,"action":"reply","text":"x","tool":"launch_application"}),
            json!({"version":2,"action":"tool","tool":"launch_application","arguments":{"application":"cmd"}}),
            json!({"version":2,"action":"tool","tool":"launch_application","arguments":{"application":"notepad","path":"x"}}),
            json!({"version":2,"action":"tool","tool":"shell","arguments":{"application":"notepad"}}),
            json!({"version":2,"action":"tool","tool":"launch_application","arguments":{"application":"notepad"},"extra":1}),
            json!({"version":2,"action":"execute"}),
        ] {
            assert_eq!(accept(invalid.clone()), Err("AGENT_REJECTED"), "{invalid}");
        }
    }

    #[test]
    fn outcomes_serialize_for_the_desktop() {
        assert_eq!(
            serde_json::to_value(TaskOutcome::Launched { application: "notepad", process_id: 7 }).unwrap(),
            json!({"kind":"launched","application":"notepad","processId":7})
        );
        assert_eq!(
            serde_json::to_value(TaskOutcome::Reply { text: "ok".into() }).unwrap(),
            json!({"kind":"reply","text":"ok"})
        );
    }

    #[test]
    fn real_agent_process_returns_large_replies_and_rejects_unknown_tools() {
        // Requires `python` on PATH, as in the Windows CI job.
        let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../agent");
        let script = agent.join("agent.py");
        let invoke = |proposal| invoke_agent(Path::new("python"), &script, &agent, &proposal);
        let long = "é".repeat(MAX_REPLY_CHARS);
        assert_eq!(invoke(Proposal::Text(long.clone())), Ok(PlannedAction::Reply(long)));
        assert_eq!(
            invoke(Proposal::Calls(json!([{"function":{"name":"launch_application","arguments":{"application":"notepad"}}}]))),
            Ok(PlannedAction::Launch(Application::Notepad))
        );
        assert_eq!(
            invoke(Proposal::Calls(json!([{"function":{"name":"run_shell","arguments":{"command":"dir"}}}]))),
            Err("AGENT_REJECTED")
        );
    }

    fn live_request(prompt: &str, approved: bool) -> TaskRequest {
        TaskRequest {
            provider: Provider::Ollama,
            prompt: prompt.into(),
            model: std::env::var("AIWIN11_TEST_MODEL").expect("set AIWIN11_TEST_MODEL"),
            approved,
            cloud_approved: false,
        }
    }

    #[test]
    #[ignore = "requires Ollama, the installed companions and an interactive Windows desktop"]
    fn opens_notepad_only_after_broker_authorization() {
        let core = Mutex::new(Core::start(Connection::open_in_memory().unwrap()).unwrap());
        let task = TaskSlot::default();
        assert_eq!(task.run(&core, &live_request("Ouvre le Bloc-notes", false), |_| {}), Err("ACTION_DENIED"));
        let denied = core.lock().unwrap().activity().unwrap();
        assert!(denied.iter().any(|event| event.kind == "policy.denied"));
        assert!(!denied.iter().any(|event| event.kind.starts_with("tool.execution.")));

        let stages = Mutex::new(Vec::new());
        let outcome = task.run(&core, &live_request("Peux-tu ouvrir le Bloc-notes ?", true), |stage| {
            stages.lock().unwrap().push(stage)
        }).unwrap();
        assert!(matches!(outcome, TaskOutcome::Launched { application: "notepad", process_id } if process_id > 0));
        assert_eq!(
            *stages.lock().unwrap(),
            ["task.created", "task.planning", "agent.tool.requested"]
        );
        let activity = core.lock().unwrap().activity().unwrap();
        assert_eq!(activity[0].kind, "task.completed");
        assert_eq!(activity[1].kind, "tool.execution.completed");
        assert_eq!(activity[2].kind, "policy.allowed");
    }

    #[test]
    #[ignore = "requires Ollama and the installed agent companion"]
    fn answers_without_side_effects_when_no_tool_applies() {
        let core = Mutex::new(Core::start(Connection::open_in_memory().unwrap()).unwrap());
        let task = TaskSlot::default();
        let outcome = task.run(&core, &live_request("Combien font deux plus trois ?", true), |_| {}).unwrap();
        assert!(matches!(outcome, TaskOutcome::Reply { ref text } if !text.is_empty()));
        let activity = core.lock().unwrap().activity().unwrap();
        assert_eq!(activity[0].kind, "task.completed");
        assert_eq!(activity[1].kind, "agent.reply");
        assert!(!activity.iter().any(|event| event.kind.starts_with("policy.") || event.kind.starts_with("tool.")));
    }
}
