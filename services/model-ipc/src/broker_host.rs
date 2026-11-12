use std::{
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};

use aiwin11_contracts::{v1::Command, Message};
use aiwin11_core::{
    broker::{BrokerDecision, ExecutionError, Peer, Policy},
    Core,
};
use tonic::{transport::Server, Request, Response, Status};
use uuid::Uuid;

use crate::{
    pipe,
    worker::{
        connect_channel, pipe_name,
        protocol::{
            broker_reply, broker_request,
            execution_broker_client::ExecutionBrokerClient,
            execution_broker_server::{ExecutionBroker, ExecutionBrokerServer},
            BrokerReply, BrokerRequest,
        },
        PipeConnection,
    },
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const CALL_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_COMMAND_BYTES: usize = 128 * 1024;

struct Broker(Mutex<Core>);

#[tonic::async_trait]
impl ExecutionBroker for Broker {
    async fn dispatch(&self, request: Request<BrokerRequest>) -> Result<Response<BrokerReply>, Status> {
        let request = request.into_inner();
        let peer = match broker_request::Peer::try_from(request.peer) {
            Ok(broker_request::Peer::Agent) => Peer::Agent,
            Ok(broker_request::Peer::DesktopInspector) => Peer::DesktopInspector,
            _ => return Err(Status::permission_denied("unrecognized broker caller")),
        };
        let policy = Policy::from_capability_bits(request.capability_bits)
            .ok_or_else(|| Status::invalid_argument("unrecognized broker capability"))?;
        let command = Command::decode(request.command.as_slice())
            .map_err(|_| Status::invalid_argument("invalid broker command"))?;
        let mut core = self.0.lock().map_err(|_| Status::unavailable("broker unavailable"))?;
        let mut reply = BrokerReply::default();
        match core.broker_dispatch(policy, peer, &command, request.execute_windows) {
            Ok((task_id, result)) => {
                reply.status = broker_reply::Status::Authorized as i32;
                reply.task_id = task_id;
                reply.windows_result = result.map(|result| result.encode_to_vec()).unwrap_or_default();
            }
            Err(ExecutionError::Denied(code)) => {
                reply.status = broker_reply::Status::Denied as i32;
                reply.denial_code = code.into();
            }
            Err(_) => reply.status = broker_reply::Status::AuditUnavailable as i32,
        }
        Ok(Response::new(reply))
    }
}

async fn serve(
    id: Uuid,
    parent: u32,
    path: &Path,
    session: &str,
    desktop_pid: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = pipe::listen(&pipe_name(id))?;
    tokio::time::timeout(CONNECT_TIMEOUT, pipe::accept(&listener, parent)).await??;
    let core = Core::attach_task_worker(path, session, desktop_pid)?;
    use tokio_stream::StreamExt;
    let incoming = tokio_stream::once(Ok::<_, io::Error>(PipeConnection(listener))).chain(tokio_stream::pending());
    tokio::time::timeout(
        CALL_TIMEOUT,
        Server::builder()
            .add_service(
                ExecutionBrokerServer::new(Broker(Mutex::new(core))).max_decoding_message_size(MAX_COMMAND_BYTES),
            )
            .serve_with_incoming(incoming),
    )
    .await??;
    Ok(())
}

pub(crate) fn worker_entry(mut args: impl Iterator<Item = OsString>) -> i32 {
    let id = args.next().and_then(|value| Uuid::parse_str(value.to_str()?).ok());
    let parent = args.next().and_then(|value| value.to_str()?.parse::<u32>().ok()).filter(|&pid| pid != 0);
    let path = args.next().map(PathBuf::from).filter(|path| path.is_absolute());
    let session = args.next().and_then(|value| value.into_string().ok());
    let desktop_pid = args.next().and_then(|value| value.to_str()?.parse::<u32>().ok()).filter(|&pid| pid != 0);
    let (Some(id), Some(parent), Some(path), Some(session), Some(desktop_pid), None) =
        (id, parent, path, session, desktop_pid, args.next())
    else {
        return 2;
    };
    std::thread::spawn(|| {
        let _ = io::stdin().read_exact(&mut [0]);
        std::process::exit(0);
    });
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return 1;
    };
    i32::from(runtime.block_on(serve(id, parent, &path, &session, desktop_pid)).is_err())
}

pub fn dispatch(
    path: &Path,
    session: &str,
    desktop_pid: u32,
    policy: Policy,
    peer: Peer,
    command: &Command,
    execute_windows: bool,
) -> BrokerDecision {
    let peer = match peer {
        Peer::Agent => broker_request::Peer::Agent,
        Peer::DesktopInspector => broker_request::Peer::DesktopInspector,
        Peer::Other => return Err(ExecutionError::Denied("invalid_scope")),
    };
    let request = BrokerRequest {
        peer: peer as i32,
        capability_bits: policy.capability_bits(),
        command: command.encode_to_vec(),
        execute_windows,
    };
    if request.encoded_len() > MAX_COMMAND_BYTES {
        return Err(ExecutionError::Denied("command_too_large"));
    }
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| ExecutionError::AuditUnavailable)?;
                runtime.block_on(call(path, session, desktop_pid, request))
            })
            .join()
            .unwrap_or(Err(ExecutionError::AuditUnavailable))
    })
}

async fn call(path: &Path, session: &str, desktop_pid: u32, request: BrokerRequest) -> BrokerDecision {
    let unavailable = || ExecutionError::AuditUnavailable;
    let id = Uuid::new_v4();
    let mut process = tokio::process::Command::new(std::env::current_exe().map_err(|_| unavailable())?);
    #[cfg(not(test))]
    process
        .arg("--broker-worker")
        .arg(id.to_string())
        .arg(std::process::id().to_string())
        .arg(path)
        .arg(session)
        .arg(desktop_pid.to_string());
    #[cfg(test)]
    process
        .args(["--exact", "broker_host::tests::broker_peer"])
        .env("AIWIN11_BROKER_TEST_PIPE", id.to_string())
        .env("AIWIN11_BROKER_TEST_PARENT", std::process::id().to_string())
        .env("AIWIN11_BROKER_TEST_STORE", path)
        .env("AIWIN11_BROKER_TEST_SESSION", session)
        .env("AIWIN11_BROKER_TEST_DESKTOP", desktop_pid.to_string());
    let mut worker = process
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let lifetime = worker.stdin.take().ok_or_else(unavailable)?;
    let pid = worker.id().ok_or_else(unavailable)?;
    let result = tokio::time::timeout(CALL_TIMEOUT, async {
        let channel = tokio::time::timeout(CONNECT_TIMEOUT, connect_channel(id, pid))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
        let reply =
            ExecutionBrokerClient::new(channel).dispatch(request).await.map_err(|_| unavailable())?.into_inner();
        match reply.status() {
            broker_reply::Status::Authorized => {
                let result = if reply.windows_result.is_empty() {
                    None
                } else {
                    Some(
                        aiwin11_contracts::v1::Result::decode(reply.windows_result.as_slice())
                            .map_err(|_| unavailable())?,
                    )
                };
                Ok((reply.task_id, result))
            }
            broker_reply::Status::Denied => Err(ExecutionError::Denied(denial_code(&reply.denial_code))),
            _ => Err(unavailable()),
        }
    })
    .await
    .unwrap_or_else(|_| Err(unavailable()));
    drop(lifetime);
    if tokio::time::timeout(Duration::from_secs(1), worker.wait()).await.is_err() {
        let _ = worker.kill().await;
    }
    result
}

fn denial_code(code: &str) -> &'static str {
    match code {
        "unsupported_schema" => "unsupported_schema",
        "invalid_scope" => "invalid_scope",
        "invalid_identifier" => "invalid_identifier",
        "clock_unavailable" => "clock_unavailable",
        "expired" => "expired",
        "confirmation_required" => "confirmation_required",
        "resource_blocked" => "resource_blocked",
        "duplicate_request" => "duplicate_request",
        "unsupported_bridge_operation" => "unsupported_bridge_operation",
        _ => "default_deny",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwin11_contracts::v1::{command::Operation, launch_application::Application, LaunchApplication};

    #[test]
    fn broker_peer() {
        if std::env::var_os("AIWIN11_BROKER_TEST_PIPE").is_none() {
            return;
        }
        let args = ["PIPE", "PARENT", "STORE", "SESSION", "DESKTOP"]
            .into_iter()
            .map(|key| std::env::var_os(format!("AIWIN11_BROKER_TEST_{key}")).unwrap());
        std::process::exit(worker_entry(args));
    }

    #[test]
    fn remote_broker_denies_ungranted_launch_and_deduplicates() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("broker.sqlite3");
        let core = Core::open(&store).unwrap();
        let session = core.session_id();
        let deadline_ms =
            (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() + 30_000) as i64;
        let command = Command {
            schema_version: aiwin11_contracts::SCHEMA_VERSION,
            request_id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            correlation_id: Uuid::new_v4().to_string(),
            idempotency_key: Uuid::new_v4().to_string(),
            session_id: session.to_owned(),
            deadline_ms,
            operation: Some(Operation::LaunchApplication(LaunchApplication {
                application: Application::Notepad as i32,
            })),
            ..Command::default()
        };
        let invoke =
            |policy, execute| dispatch(&store, session, std::process::id(), policy, Peer::Agent, &command, execute);
        assert_eq!(invoke(Policy::default(), true), Err(ExecutionError::Denied("default_deny")));
        let grant = Policy::granting(&[aiwin11_core::broker::Capability::LaunchNotepad]);
        assert_eq!(invoke(grant, false).unwrap().0.as_deref(), Some(command.task_id.as_str()));
        assert_eq!(invoke(grant, true), Err(ExecutionError::Denied("duplicate_request")));
        let events = core.activity().unwrap();
        assert_eq!(events.iter().filter(|event| event.kind == "policy.allowed").count(), 1);
        assert!(!events.iter().any(|event| event.kind.starts_with("broker.windows.")));
    }
}
