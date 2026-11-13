use std::{
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use aiwin11_core::{folders::FolderSet, local_files::LocalFiles, task::TaskSlot, Core};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
use uuid::Uuid;

use crate::worker::{
    connect_channel, pipe_name, propose_cloud, propose_local,
    protocol::{
        task_control::Action,
        task_host_client::TaskHostClient,
        task_host_server::{TaskHost, TaskHostServer},
        TaskAck, TaskControl, TaskCursor, TaskPage, TaskStart,
    },
    route_local_model, PipeConnection,
};

pub use crate::worker::protocol::task_control::Action as TaskCommand;

const TIMEOUT: Duration = Duration::from_secs(4);
const UNAVAILABLE: &str = "TASK_WORKER_UNAVAILABLE";

pub(crate) fn worker_entry(mut args: impl Iterator<Item = OsString>) -> i32 {
    let id = args.next().and_then(|value| Uuid::parse_str(value.to_str()?).ok());
    let parent = args.next().and_then(|value| value.to_str()?.parse::<u32>().ok());
    let store = args.next().map(PathBuf::from).filter(|path| path.is_absolute());
    let session = args.next().and_then(|value| value.into_string().ok());
    let (Some(id), Some(parent), Some(store), Some(session), None) = (id, parent, store, session, args.next()) else {
        return 2;
    };
    std::thread::spawn(|| {
        let _ = io::stdin().read_exact(&mut [0]);
        std::process::exit(0);
    });
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return 1;
    };
    i32::from(runtime.block_on(serve(id, parent, &store, &session)).is_err())
}

async fn serve(id: Uuid, parent: u32, store: &Path, session: &str) -> Result<(), Box<dyn std::error::Error>> {
    let host = Host::open(store, session, parent)?;
    host.core.lock().map_err(|_| "CORE_UNAVAILABLE")?.set_broker_transport(crate::broker_dispatch);
    serve_host(id, parent, host).await
}

async fn serve_host(id: Uuid, parent: u32, host: Host) -> Result<(), Box<dyn std::error::Error>> {
    let name = pipe_name(id);
    let mut listener = crate::pipe::listen_instance(&name, true, 8)?;
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            if crate::pipe::accept(&listener, parent).await.is_err() {
                break;
            }
            let next = crate::pipe::listen_instance(&name, false, 8);
            if tx.send(Ok::<_, io::Error>(PipeConnection(listener))).await.is_err() {
                break;
            }
            let Ok(next) = next else {
                break;
            };
            listener = next;
        }
    });
    Server::builder().add_service(TaskHostServer::new(host)).serve_with_incoming(ReceiverStream::new(rx)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwin11_core::{
        model_gateway::Proposal,
        policy::AgentProfile,
        task::{Provider, TaskRequest},
    };
    use serde_json::json;

    #[test]
    fn test_task_peer_entry() {
        let Ok(id) = std::env::var("AIWIN11_TASK_TEST_PIPE") else {
            return;
        };
        let store = PathBuf::from(std::env::var_os("AIWIN11_TASK_TEST_STORE").unwrap());
        let parent = std::env::var("AIWIN11_TASK_TEST_PARENT").unwrap().parse().unwrap();
        let session = std::env::var("AIWIN11_TASK_TEST_SESSION").unwrap();
        let mut host = Host::open(&store, &session, parent).unwrap();
        host.core.lock().unwrap().set_broker_transport(crate::broker_dispatch);
        host.task = Arc::new(TaskSlot::with_model_worker(
            |_, _, _, _| Ok(None),
            |_, _, messages, _, _| {
                if messages.last().is_some_and(|message| message["role"] == "tool") {
                    Ok(Proposal::Text("Copie terminée.".into()))
                } else {
                    Ok(Proposal::Calls(json!([{"function":{"name":"copy_text_file",
                        "arguments":{"source":"source.txt","destination":"destination.txt"}}}])))
                }
            },
            |_, _, _, _| Err("CLOUD_REQUEST_FAILED"),
        ));
        std::thread::spawn(|| {
            let _ = io::stdin().read_exact(&mut [0]);
            std::process::exit(0);
        });
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(serve_host(id.parse().unwrap(), parent, host)).unwrap();
    }

    #[tokio::test]
    async fn approved_task_survives_reconnect_and_cannot_dispatch_twice() {
        exercise_pending_approval(false).await;
    }

    #[tokio::test]
    async fn cancelling_a_live_task_prevents_dispatch_and_invalidates_approval() {
        exercise_pending_approval(true).await;
    }

    async fn exercise_pending_approval(cancel: bool) {
        if !aiwin11_core::task::python_available() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let exe = directory.path().join("task-test.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
        std::fs::write(directory.path().join("aiwin11_agent.py"), include_str!("../../agent/agent.py")).unwrap();
        let store = directory.path().join("test.sqlite3");
        let core = Core::open(&store).unwrap();
        let files = LocalFiles::open(directory.path()).unwrap();
        let root = files.root();
        std::fs::write(root.join("source.txt"), b"fixture").unwrap();
        let id = Uuid::new_v4();
        let run_id = id.to_string();
        let mut child = tokio::process::Command::new(exe)
            .args(["--exact", "task_host::tests::test_task_peer_entry"])
            .env("AIWIN11_TASK_TEST_PIPE", &run_id)
            .env("AIWIN11_TASK_TEST_STORE", &store)
            .env("AIWIN11_TASK_TEST_PARENT", std::process::id().to_string())
            .env("AIWIN11_TASK_TEST_SESSION", core.session_id())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            let mut client = TaskHostClient::new(connect_channel(id, pid).await.unwrap());
            let request = TaskRequest {
                provider: Provider::Ollama,
                prompt: "Copie source.txt vers destination.txt".into(),
                model: "fixture:latest".into(),
                cloud_approved: false,
                cloud_vendor: None,
                local_runtime: aiwin11_core::model_gateway::LocalRuntime::Ollama,
                vision_model: None,
                cloud_fallback_model: None,
                scheduled: false,
                agent: AgentProfile::General,
                context: Vec::new(),
                custom_agent: None,
            };
            let start = TaskStart { run_id: run_id.clone(), request_json: serde_json::to_vec(&request).unwrap() };
            assert!(client.start(start.clone()).await.unwrap().into_inner().accepted);
            let cursor = TaskCursor { run_id: run_id.clone(), after_sequence: 0 };
            let approval = loop {
                let page = client.page(cursor.clone()).await.unwrap().into_inner();
                assert!(page.error.is_none(), "{:?}", page.error);
                if let Some(approval) = page.updates_json.iter().find_map(|bytes| {
                    let update: Value = serde_json::from_slice(bytes).unwrap();
                    update["approvalId"].as_str().map(str::to_owned)
                }) {
                    break approval;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            };
            drop(client);
            let mut client = TaskHostClient::new(connect_channel(id, pid).await.unwrap());
            let replay = client.page(cursor.clone()).await.unwrap().into_inner();
            assert_eq!(
                replay
                    .updates_json
                    .iter()
                    .filter(|bytes| { serde_json::from_slice::<Value>(bytes).unwrap()["approvalId"] == approval })
                    .count(),
                1
            );
            assert!(client.start(start).await.unwrap().into_inner().accepted);
            let approve = TaskControl { run_id: run_id.clone(), action: Action::Approve as i32, approval_id: approval };
            if cancel {
                let stop =
                    TaskControl { run_id: run_id.clone(), action: Action::Cancel as i32, approval_id: String::new() };
                assert!(client.control(stop).await.unwrap().into_inner().accepted);
                assert!(!client.control(approve).await.unwrap().into_inner().accepted);
                loop {
                    let page = client.page(cursor.clone()).await.unwrap().into_inner();
                    assert!(page.outcome_json.is_none());
                    if let Some(code) = page.error {
                        assert_eq!(code, "TASK_CANCELLED");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                assert!(!root.join("destination.txt").exists());
                assert!(!core.activity().unwrap().iter().any(|event| event.kind == "policy.approval.granted"));
                return;
            }
            assert!(client.control(approve.clone()).await.unwrap().into_inner().accepted);
            assert!(!client.control(approve).await.unwrap().into_inner().accepted);
            loop {
                let page = client.page(cursor.clone()).await.unwrap().into_inner();
                assert!(page.error.is_none(), "{:?}", page.error);
                if let Some(bytes) = page.outcome_json {
                    let outcome: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(outcome["actions"].as_array().unwrap().len(), 1);
                    assert_eq!(outcome["actions"][0]["status"], "done");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            assert_eq!(std::fs::read(root.join("destination.txt")).unwrap(), b"fixture");
            assert_eq!(
                core.activity().unwrap().iter().filter(|event| event.kind == "policy.approval.granted").count(),
                1
            );
        })
        .await
        .expect("approval flow completes after reconnection");
        drop(child.stdin.take());
        assert!(tokio::time::timeout(TIMEOUT, child.wait()).await.unwrap().unwrap().success());
    }
}

#[derive(Clone)]
struct Active {
    id: String,
    channel: Channel,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskReplay {
    pub run_id: String,
    pub updates: Vec<Value>,
    pub outcome: Option<Value>,
    pub error: Option<String>,
    pub scheduled: bool,
    pub after_sequence: usize,
}

#[derive(Default)]
pub struct TaskSupervisor {
    active: Mutex<Option<Active>>,
    replay: Mutex<TaskReplay>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskPresence {
    Idle,
    Running,
    Approval,
}

impl TaskSupervisor {
    pub fn presence(&self) -> TaskPresence {
        let Ok(state) = self.replay.lock() else { return TaskPresence::Idle };
        if state.run_id.is_empty() || state.outcome.is_some() || state.error.is_some() {
            return TaskPresence::Idle;
        }
        for update in state.updates.iter().rev() {
            if update["type"] == "approvalRequired" {
                return TaskPresence::Approval;
            }
            if matches!(
                update["stage"].as_str(),
                Some("policy.approval.granted" | "policy.approval.declined" | "policy.approval.expired")
            ) {
                break;
            }
        }
        TaskPresence::Running
    }

    pub fn replay(&self, run_id: Option<&str>, after: usize) -> TaskReplay {
        let Ok(state) = self.replay.lock() else {
            return TaskReplay::default();
        };
        if state.scheduled {
            return TaskReplay::default();
        }
        let after_sequence = if run_id == Some(state.run_id.as_str()) { after.min(state.updates.len()) } else { 0 };
        TaskReplay {
            run_id: state.run_id.clone(),
            updates: state.updates[after_sequence..].to_vec(),
            outcome: state.outcome.clone(),
            error: state.error.clone(),
            scheduled: false,
            after_sequence,
        }
    }

    pub async fn control(&self, action: TaskCommand, approval: String) -> Result<(bool, String), &'static str> {
        let active = self.active.lock().map_err(|_| UNAVAILABLE)?.clone();
        let Some(active) = active else {
            return Ok((false, "NO_ACTIVE_TASK".into()));
        };
        let reply = tokio::time::timeout(
            TIMEOUT,
            TaskHostClient::new(active.channel).control(TaskControl {
                run_id: active.id,
                action: action as i32,
                approval_id: approval,
            }),
        )
        .await
        .map_err(|_| UNAVAILABLE)?
        .map_err(|_| UNAVAILABLE)?
        .into_inner();
        Ok((reply.accepted, reply.code))
    }

    pub async fn run(
        &self,
        store: &Path,
        session: &str,
        request: &aiwin11_core::task::TaskRequest,
        progress: impl Fn(Value),
    ) -> Result<Value, String> {
        let id = Uuid::new_v4();
        *self.replay.lock().map_err(|_| UNAVAILABLE)? =
            TaskReplay { run_id: id.to_string(), scheduled: request.scheduled, ..TaskReplay::default() };
        let result = self.run_peer(store, session, request, id, progress).await;
        *self.active.lock().map_err(|_| UNAVAILABLE)? = None;
        let mut replay = self.replay.lock().map_err(|_| UNAVAILABLE)?;
        match &result {
            Ok(outcome) => replay.outcome = Some(outcome.clone()),
            Err(code) => replay.error = Some(code.clone()),
        }
        result
    }

    async fn run_peer(
        &self,
        store: &Path,
        session: &str,
        request: &aiwin11_core::task::TaskRequest,
        id: Uuid,
        progress: impl Fn(Value),
    ) -> Result<Value, String> {
        let mut child = tokio::process::Command::new(std::env::current_exe().map_err(|_| UNAVAILABLE)?)
            .arg("--task-worker")
            .arg(id.to_string())
            .arg(std::process::id().to_string())
            .arg(store)
            .arg(session)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| UNAVAILABLE)?;
        let _lifetime = child.stdin.take().ok_or(UNAVAILABLE)?;
        let pid = child.id().ok_or(UNAVAILABLE)?;
        let call = async {
            let channel = tokio::time::timeout(TIMEOUT, connect_channel(id, pid))
                .await
                .map_err(|_| UNAVAILABLE)?
                .map_err(|_| UNAVAILABLE)?;
            let mut client = TaskHostClient::new(channel.clone());
            let start = TaskStart {
                run_id: id.to_string(),
                request_json: serde_json::to_vec(request).map_err(|_| UNAVAILABLE)?,
            };
            let ack = tokio::time::timeout(TIMEOUT, client.start(start))
                .await
                .map_err(|_| UNAVAILABLE)?
                .map_err(|_| UNAVAILABLE)?
                .into_inner();
            if !ack.accepted {
                return Err(ack.code);
            }
            *self.active.lock().map_err(|_| UNAVAILABLE)? = Some(Active { id: id.to_string(), channel });
            let mut after = 0;
            loop {
                let cursor = TaskCursor { run_id: id.to_string(), after_sequence: after };
                let page = match tokio::time::timeout(TIMEOUT, client.page(cursor.clone())).await {
                    Ok(Ok(reply)) => reply.into_inner(),
                    _ => {
                        // Retry only reading the same cursor. A submitted action is never repeated.
                        tokio::time::timeout(TIMEOUT, client.page(cursor))
                            .await
                            .map_err(|_| UNAVAILABLE)?
                            .map_err(|_| UNAVAILABLE)?
                            .into_inner()
                    }
                };
                after = page.sequence;
                for bytes in page.updates_json {
                    let update: Value = serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE)?;
                    let sequence = {
                        let mut replay = self.replay.lock().map_err(|_| UNAVAILABLE)?;
                        replay.updates.push(update.clone());
                        replay.updates.len()
                    };
                    progress(serde_json::json!({ "runId": id.to_string(), "sequence": sequence, "update": update }));
                }
                if let Some(code) = page.error {
                    return Err(code);
                }
                if let Some(outcome) = page.outcome_json {
                    return serde_json::from_slice(&outcome).map_err(|_| UNAVAILABLE.into());
                }
                tokio::time::sleep(Duration::from_millis(70)).await;
            }
        };
        let result = tokio::select! {
            result = call => result,
            _ = child.wait() => Err(UNAVAILABLE.into()),
        };
        let _ = child.kill().await;
        result
    }
}

#[derive(Clone)]
pub(crate) struct Host {
    core: Arc<Mutex<Core>>,
    task: Arc<TaskSlot>,
    run: Arc<Mutex<Option<Run>>>,
    files: Arc<Option<FolderSet>>,
}

struct Run {
    id: String,
    updates: Vec<Vec<u8>>,
    result: Option<Result<Vec<u8>, &'static str>>,
}

impl Host {
    pub(crate) fn open(store: &Path, session: &str, parent: u32) -> Result<Self, &'static str> {
        let core = Core::attach_task_worker(store, session, parent).map_err(|_| "CORE_UNAVAILABLE")?;
        let directory = store.parent().ok_or("CORE_UNAVAILABLE")?;
        let files = core
            .workspace()
            .and_then(|path| LocalFiles::open_with_workspace(directory, &path).ok())
            .or_else(|| LocalFiles::open(directory).ok())
            .map(|primary| FolderSet::with_extra(primary, directory, &core.extra_folders()));
        let task = TaskSlot::with_model_worker(
            |strategy, model, excluded, cancelled| {
                tokio::runtime::Handle::current().block_on(route_local_model(strategy, model, excluded, cancelled))
            },
            |runtime, model, messages, tools, sink| {
                tokio::runtime::Handle::current().block_on(propose_local(runtime, model, messages, tools, sink))
            },
            |vendor, model, messages, sink| {
                tokio::runtime::Handle::current().block_on(propose_cloud(vendor, model, messages, sink))
            },
        );
        Ok(Self {
            core: Arc::new(Mutex::new(core)),
            task: Arc::new(task),
            run: Arc::new(Mutex::new(None)),
            files: Arc::new(files),
        })
    }
}

fn ack(accepted: bool, code: &str) -> Response<TaskAck> {
    Response::new(TaskAck { accepted, code: code.into() })
}

#[tonic::async_trait]
impl TaskHost for Host {
    async fn start(&self, request: Request<TaskStart>) -> Result<Response<TaskAck>, Status> {
        let request = request.into_inner();
        if uuid::Uuid::parse_str(&request.run_id).is_err() {
            return Err(Status::invalid_argument("invalid task identity"));
        }
        let input = serde_json::from_slice(&request.request_json)
            .map_err(|_| Status::invalid_argument("invalid task request"))?;
        {
            let mut run = self.run.lock().map_err(|_| Status::internal("task state unavailable"))?;
            if let Some(existing) = run.as_ref() {
                return Ok(ack(existing.id == request.run_id, "TASK_BUSY"));
            }
            *run = Some(Run { id: request.run_id, updates: Vec::new(), result: None });
        }
        let host = self.clone();
        tokio::spawn(async move {
            let worker = host.clone();
            let result = tokio::task::spawn_blocking(move || {
                worker
                    .task
                    .run(&worker.core, worker.files.as_ref().as_ref(), &input, |update| {
                        if let Ok(mut event) = serde_json::to_value(&update) {
                            if matches!(update, aiwin11_core::task::TaskUpdate::ApprovalRequired { .. }) {
                                event["issuedAtMs"] = serde_json::json!(std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    as u64);
                            }
                            let Ok(bytes) = serde_json::to_vec(&event) else {
                                return;
                            };
                            if let Ok(mut run) = worker.run.lock() {
                                if let Some(run) = run.as_mut() {
                                    run.updates.push(bytes);
                                }
                            }
                        }
                    })
                    .and_then(|outcome| serde_json::to_vec(&outcome).map_err(|_| "CORE_UNAVAILABLE"))
            })
            .await
            .unwrap_or(Err("CORE_UNAVAILABLE"));
            if let Ok(mut run) = host.run.lock() {
                if let Some(run) = run.as_mut() {
                    run.result = Some(result);
                }
            }
        });
        Ok(ack(true, ""))
    }

    async fn page(&self, request: Request<TaskCursor>) -> Result<Response<TaskPage>, Status> {
        let request = request.into_inner();
        let state = self.run.lock().map_err(|_| Status::internal("task state unavailable"))?;
        let run =
            state.as_ref().filter(|run| run.id == request.run_id).ok_or_else(|| Status::not_found("task not found"))?;
        let after = request.after_sequence as usize;
        if after > run.updates.len() {
            return Err(Status::invalid_argument("invalid task cursor"));
        }
        let end = (after + 16).min(run.updates.len());
        let (outcome_json, error) = match (&run.result, end == run.updates.len()) {
            (Some(Ok(bytes)), true) => (Some(bytes.clone()), None),
            (Some(Err(code)), true) => (None, Some((*code).into())),
            _ => (None, None),
        };
        Ok(Response::new(TaskPage {
            sequence: end as u32,
            updates_json: run.updates[after..end].to_vec(),
            outcome_json,
            error,
        }))
    }

    async fn control(&self, request: Request<TaskControl>) -> Result<Response<TaskAck>, Status> {
        let request = request.into_inner();
        if !self
            .run
            .lock()
            .map_err(|_| Status::internal("task state unavailable"))?
            .as_ref()
            .is_some_and(|run| run.id == request.run_id && run.result.is_none())
        {
            return Ok(ack(false, "NO_ACTIVE_TASK"));
        }
        let action = Action::try_from(request.action).map_err(|_| Status::invalid_argument("unknown task control"))?;
        Ok(match action {
            Action::Cancel => ack(self.task.cancel(), ""),
            Action::Pause => ack(self.task.pause(), ""),
            Action::Resume => ack(self.task.resume(), ""),
            Action::Stop => ack(true, self.task.stop_all().code()),
            Action::Approve | Action::Decline => {
                match self.task.resolve_approval(&request.approval_id, action == Action::Approve) {
                    Ok(()) => ack(true, ""),
                    Err(code) => ack(false, code),
                }
            }
            Action::Unspecified => return Err(Status::invalid_argument("missing task control")),
        })
    }
}
