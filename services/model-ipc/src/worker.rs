use std::{
    future::Future,
    io::{self, Read},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Mutex,
    task::{Context, Poll},
    time::Duration,
};

use aiwin11_core::{
    model_gateway::{
        GatewayError, LoadedModel, LocalModelProvider, LocalRuntime, LocalStrategy, Ollama, Proposal, StreamSink,
    },
    openai::{self, Vendor},
    read_committed_activity, valid_model_name, Activity,
};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::windows::named_pipe::NamedPipeServer,
    sync::{mpsc, Semaphore},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    transport::{server::Connected, Channel, Endpoint, Server},
    Request, Response, Status,
};

use crate::pipe;

pub(crate) mod protocol {
    tonic::include_proto!("aiwin11.model.v1");
}

use protocol::{
    core_session_client::CoreSessionClient,
    core_session_server::{CoreSession, CoreSessionServer},
    journal_client::JournalClient,
    journal_server::{Journal, JournalServer},
    local_proposal_update::Update,
    model_catalog_client::ModelCatalogClient,
    model_catalog_server::{ModelCatalog, ModelCatalogServer},
    CloudProposalRequest, JournalEvent, JournalReply, JournalRequest, ListLoadedModelsReply, ListLocalModelsReply,
    ListLocalModelsRequest, LocalProposalRequest, LocalProposalUpdate, RouteLocalModelReply, RouteLocalModelRequest,
};

const CALL_TIMEOUT: Duration = Duration::from_secs(8);
const ROUTE_TIMEOUT: Duration = Duration::from_secs(180);
const INFERENCE_TIMEOUT: Duration = Duration::from_secs(110);
const WORKER_TIMEOUT: Duration = Duration::from_secs(185);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SCHEMA_VERSION: u32 = 1;
const UNAVAILABLE: &str = "MODEL_UNAVAILABLE_OR_INVALID";
static WORKER_SLOTS: Semaphore = Semaphore::const_new(4);

struct Catalog;

struct SessionBootstrap {
    store: PathBuf,
    session: Mutex<Option<String>>,
}

#[tonic::async_trait]
impl CoreSession for SessionBootstrap {
    async fn initialize(
        &self,
        request: Request<protocol::InitializeSession>,
    ) -> Result<Response<protocol::InitializedSession>, Status> {
        if request.into_inner().schema_version != SCHEMA_VERSION {
            return Err(Status::invalid_argument("unsupported schema version"));
        }
        let mut session = self.session.lock().map_err(|_| Status::unavailable("session state unavailable"))?;
        if let Some(id) = session.as_ref() {
            return Ok(Response::new(protocol::InitializedSession { session_id: id.clone() }));
        }
        let restored = aiwin11_core::backup::apply_pending_restore(&self.store);
        let core = aiwin11_core::Core::open(&self.store).map_err(|_| Status::unavailable("cannot open core store"))?;
        if let Some(outcome) = restored {
            let _ = core.journal(
                if outcome.is_ok() { "data.restore.applied" } else { "data.restore.failed" },
                Some(outcome.err().unwrap_or("success")),
            );
        }
        let id = core.session_id().to_owned();
        *session = Some(id.clone());
        Ok(Response::new(protocol::InitializedSession { session_id: id }))
    }
}

struct AuditJournal(PathBuf);

#[tonic::async_trait]
impl Journal for AuditJournal {
    async fn read(&self, request: Request<JournalRequest>) -> Result<Response<JournalReply>, Status> {
        let request = request.into_inner();
        if request.schema_version != SCHEMA_VERSION
            || uuid::Uuid::parse_str(&request.session_id).is_err()
            || request.after_sequence.is_some_and(|sequence| sequence > i64::MAX as u64)
        {
            return Err(Status::invalid_argument("invalid journal cursor"));
        }
        let path = self.0.clone();
        let events = tokio::task::spawn_blocking(move || {
            read_committed_activity(&path, &request.session_id, request.after_sequence)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or_else(|| Status::unavailable("journal unavailable"))?;
        Ok(Response::new(JournalReply {
            events: events
                .into_iter()
                .map(|event| JournalEvent {
                    sequence: event.sequence,
                    kind: event.kind,
                    timestamp: event.timestamp,
                    task_id: event.task_id,
                    code: event.code,
                })
                .collect(),
        }))
    }
}

#[tonic::async_trait]
impl ModelCatalog for Catalog {
    type ProposeLocalStream = ReceiverStream<Result<LocalProposalUpdate, Status>>;
    type ProposeCloudStream = Self::ProposeLocalStream;

    async fn list_lm_studio_models(
        &self,
        request: Request<ListLocalModelsRequest>,
    ) -> Result<Response<ListLocalModelsReply>, Status> {
        if request.into_inner().schema_version != SCHEMA_VERSION {
            return Err(Status::invalid_argument("unsupported schema version"));
        }
        let names = tokio::task::spawn_blocking(openai::lm_studio_models)
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| Status::unavailable("LM Studio unavailable"))?;
        Ok(Response::new(ListLocalModelsReply { names }))
    }

    async fn list_local_models(
        &self,
        request: Request<ListLocalModelsRequest>,
    ) -> Result<Response<ListLocalModelsReply>, Status> {
        if request.into_inner().schema_version != SCHEMA_VERSION {
            return Err(Status::invalid_argument("unsupported schema version"));
        }
        let names = tokio::task::spawn_blocking(|| Ollama::connect().and_then(|provider| provider.discover()))
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| Status::unavailable("local model catalog unavailable"))?;
        Ok(Response::new(ListLocalModelsReply { names }))
    }

    async fn list_loaded_models(
        &self,
        request: Request<ListLocalModelsRequest>,
    ) -> Result<Response<ListLoadedModelsReply>, Status> {
        if request.into_inner().schema_version != SCHEMA_VERSION {
            return Err(Status::invalid_argument("unsupported schema version"));
        }
        let loaded = tokio::task::spawn_blocking(|| Ollama::connect().and_then(|provider| provider.loaded_models()))
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| Status::unavailable("loaded local models unavailable"))?;
        let models = loaded
            .into_iter()
            .map(|model| protocol::LoadedModel { name: model.name, size: model.size, size_vram: model.size_vram })
            .collect();
        Ok(Response::new(ListLoadedModelsReply { models }))
    }

    async fn route_local_model(
        &self,
        request: Request<RouteLocalModelRequest>,
    ) -> Result<Response<RouteLocalModelReply>, Status> {
        let request = request.into_inner();
        let strategy = LocalStrategy::from_code(&request.strategy);
        if request.schema_version != SCHEMA_VERSION
            || !valid_model_name(&request.preferred)
            || !request.excluded.iter().all(|name| valid_model_name(name))
        {
            return Err(Status::invalid_argument("invalid local routing request"));
        }
        let strategy = strategy.ok_or_else(|| Status::invalid_argument("unknown local strategy"))?;
        let selected_model = if !strategy.ranks_local() {
            None
        } else {
            tokio::task::spawn_blocking(move || {
                Ollama::connect().and_then(|ollama| ollama.route_local(strategy, &request.preferred, &request.excluded))
            })
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| Status::unavailable("local routing unavailable"))?
            .map(|(model, _)| model)
        };
        Ok(Response::new(RouteLocalModelReply { selected_model }))
    }

    async fn propose_local(
        &self,
        request: Request<LocalProposalRequest>,
    ) -> Result<Response<Self::ProposeLocalStream>, Status> {
        let request = request.into_inner();
        let runtime = LocalRuntime::from_code(&request.runtime)
            .ok_or_else(|| Status::invalid_argument("invalid local runtime"))?;
        if request.schema_version != SCHEMA_VERSION || !valid_model_name(&request.model) {
            return Err(Status::invalid_argument("invalid inference request"));
        }
        let messages: Vec<Value> = serde_json::from_slice(&request.messages_json)
            .map_err(|_| Status::invalid_argument("invalid model messages"))?;
        let tools: Value =
            serde_json::from_slice(&request.tools_json).map_err(|_| Status::invalid_argument("invalid model tools"))?;
        if !tools.is_array() {
            return Err(Status::invalid_argument("invalid model tools"));
        }
        Ok(Response::new(proposal_stream(move |sink| {
            match runtime {
                LocalRuntime::Ollama => {
                    Ollama::connect().and_then(|provider| provider.propose(&request.model, &messages, tools, sink))
                }
                LocalRuntime::LmStudio => openai::propose_lm_studio(&request.model, &messages, tools, sink),
            }
            .map(|proposal| (proposal, 0))
            .map_err(|error| match error {
                GatewayError::Unavailable => Status::failed_precondition("local_runtime_unavailable"),
                GatewayError::Cancelled => Status::cancelled("inference cancelled"),
                _ => Status::invalid_argument("invalid model proposal"),
            })
        })))
    }

    async fn propose_cloud(
        &self,
        request: Request<CloudProposalRequest>,
    ) -> Result<Response<Self::ProposeCloudStream>, Status> {
        let request = request.into_inner();
        if request.schema_version != SCHEMA_VERSION || !valid_model_name(&request.model) {
            return Err(Status::invalid_argument("invalid cloud request"));
        }
        let vendor =
            Vendor::from_code(&request.vendor).ok_or_else(|| Status::invalid_argument("unknown cloud vendor"))?;
        let messages: Vec<Value> = serde_json::from_slice(&request.messages_json)
            .map_err(|_| Status::invalid_argument("invalid model messages"))?;
        Ok(Response::new(proposal_stream(move |sink| {
            openai::propose(vendor, &request.model, &messages, sink).map_err(Status::failed_precondition)
        })))
    }
}

fn proposal_stream(
    propose: impl FnOnce(&mut StreamSink<'_>) -> Result<(Proposal, u64), Status> + Send + 'static,
) -> ReceiverStream<Result<LocalProposalUpdate, Status>> {
    let (sender, receiver) = mpsc::channel(16);
    tokio::task::spawn_blocking(move || {
        let mut sink = |delta: Option<&str>| match delta {
            Some(text) => sender
                .blocking_send(Ok(LocalProposalUpdate { update: Some(Update::Delta(text.to_owned())), used_tokens: 0 }))
                .is_ok(),
            None => !sender.is_closed(),
        };
        let response = propose(&mut sink).and_then(|(proposal, used_tokens)| {
            serde_json::to_vec(&proposal.to_agent_message())
                .map(|bytes| LocalProposalUpdate { update: Some(Update::ProposalJson(bytes)), used_tokens })
                .map_err(|_| Status::internal("invalid model proposal"))
        });
        let _ = sender.blocking_send(response);
    });
    ReceiverStream::new(receiver)
}

pub(crate) struct PipeConnection(pub(crate) NamedPipeServer);

impl Connected for PipeConnection {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for PipeConnection {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buffer)
    }
}

impl AsyncWrite for PipeConnection {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, data)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

pub(crate) fn pipe_name(id: uuid::Uuid) -> String {
    format!(r"\\.\pipe\aiwin11-model-{id}")
}

enum WorkerMode {
    Model,
    Journal(PathBuf),
    CoreSession(PathBuf),
}

async fn serve(id: uuid::Uuid, client_pid: u32, mode: WorkerMode) -> Result<(), Box<dyn std::error::Error>> {
    let server = pipe::listen(&pipe_name(id))?;
    tokio::time::timeout(CONNECT_TIMEOUT, pipe::accept(&server, client_pid)).await??;
    use tokio_stream::StreamExt;
    let incoming = tokio_stream::once(Ok::<_, io::Error>(PipeConnection(server))).chain(tokio_stream::pending());
    let (catalog, journal, bootstrap) = match mode {
        WorkerMode::Model => {
            (Some(ModelCatalogServer::new(Catalog).max_decoding_message_size(4 * 1024 * 1024)), None, None)
        }
        WorkerMode::Journal(path) => {
            (None, Some(JournalServer::new(AuditJournal(path)).max_decoding_message_size(128)), None)
        }
        WorkerMode::CoreSession(store) => {
            (None, None, Some(CoreSessionServer::new(SessionBootstrap { store, session: Mutex::new(None) })))
        }
    };
    tokio::time::timeout(
        WORKER_TIMEOUT,
        Server::builder()
            .add_optional_service(catalog)
            .add_optional_service(journal)
            .add_optional_service(bootstrap)
            .serve_with_incoming(incoming),
    )
    .await??;
    Ok(())
}

/// Handles the worker entry point before initializing Tauri or opening its database.
pub fn run_worker() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next()?;
    let mode = mode.to_str()?;
    if mode == "--task-worker" {
        return Some(crate::task_host::worker_entry(args));
    }
    if mode == "--broker-worker" {
        return Some(crate::broker_host::worker_entry(args));
    }
    if !matches!(mode, "--model-catalog-worker" | "--journal-worker" | "--core-session-worker") {
        return None;
    }
    let id = args.next().and_then(|value| value.to_str().and_then(|value| uuid::Uuid::parse_str(value).ok()));
    let client = args.next().and_then(|value| value.to_str().and_then(|value| value.parse::<u32>().ok()));
    let mode = if mode == "--model-catalog-worker" {
        WorkerMode::Model
    } else {
        match args.next().map(PathBuf::from).filter(|path| path.is_absolute()) {
            Some(path) if mode == "--journal-worker" => WorkerMode::Journal(path),
            Some(path) => WorkerMode::CoreSession(path),
            None => return Some(2),
        }
    };
    if let (Some(id), Some(client), None) = (id, client.filter(|pid| *pid != 0), args.next()) {
        return Some(serve_worker(id, client, mode));
    }
    Some(2)
}

fn serve_worker(id: uuid::Uuid, client: u32, mode: WorkerMode) -> i32 {
    // The parent holds the write handle. EOF also covers an abrupt parent exit.
    std::thread::spawn(|| {
        let _ = io::stdin().read_exact(&mut [0]);
        std::process::exit(0);
    });
    match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => i32::from(runtime.block_on(serve(id, client, mode)).is_err()),
        _ => 1,
    }
}

/// Starts a fresh standard-user worker for each catalog request. Failed calls return to the caller
/// without making an in-process or cloud request.
pub async fn list_local_models() -> Result<Vec<String>, &'static str> {
    call_worker(CALL_TIMEOUT, |mut client| async move {
        let request = ListLocalModelsRequest { schema_version: SCHEMA_VERSION };
        client.list_local_models(request).await.map(|reply| reply.into_inner().names)
    })
    .await
}

pub async fn list_loaded_models() -> Result<Vec<LoadedModel>, &'static str> {
    call_worker(CALL_TIMEOUT, |mut client| async move {
        let request = ListLocalModelsRequest { schema_version: SCHEMA_VERSION };
        client.list_loaded_models(request).await.map(|reply| {
            reply
                .into_inner()
                .models
                .into_iter()
                .map(|model| LoadedModel { name: model.name, size: model.size, size_vram: model.size_vram })
                .collect()
        })
    })
    .await
}

pub async fn list_lm_studio_models() -> Result<Vec<String>, &'static str> {
    call_worker(CALL_TIMEOUT, |mut client| async move {
        client
            .list_lm_studio_models(ListLocalModelsRequest { schema_version: SCHEMA_VERSION })
            .await
            .map(|reply| reply.into_inner().names)
    })
    .await
}

pub async fn route_local_model(
    strategy: LocalStrategy,
    preferred: &str,
    excluded: &[String],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<(String, &'static str)>, GatewayError> {
    let request = RouteLocalModelRequest {
        schema_version: SCHEMA_VERSION,
        strategy: strategy.code().to_owned(),
        preferred: preferred.to_owned(),
        excluded: excluded.to_vec(),
    };
    let route = call_worker(ROUTE_TIMEOUT, |mut client| async move {
        let mut request = Request::new(request);
        request.set_timeout(ROUTE_TIMEOUT);
        client.route_local_model(request).await.map(|reply| reply.into_inner().selected_model)
    });
    let stop = async {
        while !cancelled() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let selected = tokio::select! {
        biased;
        _ = stop => return Err(GatewayError::Cancelled),
        result = route => result.map_err(|_| GatewayError::Unavailable)?,
    };
    let reason = if matches!(strategy, LocalStrategy::Adaptive | LocalStrategy::Balanced) {
        "gpu_verified"
    } else {
        strategy.code()
    };
    Ok(selected
        .filter(|name| {
            valid_model_name(name)
                && (strategy == LocalStrategy::Balanced || name != preferred)
                && !excluded.contains(name)
        })
        .map(|name| (name, reason)))
}

pub async fn propose_local(
    runtime: LocalRuntime,
    model: &str,
    messages: &[Value],
    tools: Value,
    sink: &mut StreamSink<'_>,
) -> Result<Proposal, GatewayError> {
    let request = LocalProposalRequest {
        schema_version: SCHEMA_VERSION,
        runtime: runtime.code().into(),
        model: model.to_owned(),
        messages_json: serde_json::to_vec(messages).map_err(|_| GatewayError::InvalidResponse)?,
        tools_json: serde_json::to_vec(&tools).map_err(|_| GatewayError::InvalidResponse)?,
    };
    if !sink(None) {
        return Err(GatewayError::Cancelled);
    }
    call_worker(INFERENCE_TIMEOUT, |mut client| async move {
        let mut request = Request::new(request);
        request.set_timeout(INFERENCE_TIMEOUT);
        Ok(read_proposal(client.propose_local(request), sink).await)
    })
    .await
    .map_err(|_| GatewayError::InvalidResponse)?
    .map(|(proposal, _)| proposal)
    .map_err(inference_error)
}

pub async fn propose_cloud(
    vendor: Vendor,
    model: &str,
    messages: &[Value],
    sink: &mut StreamSink<'_>,
) -> Result<(Proposal, u64), &'static str> {
    if !sink(None) {
        return Err("TASK_CANCELLED");
    }
    let request = CloudProposalRequest {
        schema_version: SCHEMA_VERSION,
        vendor: vendor.code().into(),
        model: model.into(),
        messages_json: serde_json::to_vec(messages).map_err(|_| "CLOUD_REQUEST_FAILED")?,
    };
    let timeout = Duration::from_secs(55);
    call_worker(timeout, |mut client| async move {
        let mut request = Request::new(request);
        request.set_timeout(timeout);
        Ok(read_proposal(client.propose_cloud(request), sink).await)
    })
    .await
    .map_err(|_| "CLOUD_REQUEST_FAILED")?
    .map_err(cloud_error)
}

async fn read_proposal(
    response: impl Future<Output = Result<Response<tonic::Streaming<LocalProposalUpdate>>, Status>>,
    sink: &mut StreamSink<'_>,
) -> Result<(Proposal, u64), Status> {
    tokio::pin!(response);
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    let mut stream = loop {
        tokio::select! {
            _ = tick.tick() => if !sink(None) { return Err(Status::cancelled("task cancelled")); },
            reply = &mut response => break reply?.into_inner(),
        }
    };
    loop {
        let update = tokio::select! {
            _ = tick.tick() => {
                if !sink(None) { return Err(Status::cancelled("task cancelled")); }
                continue;
            },
            update = stream.message() => update?,
        };
        match update {
            Some(LocalProposalUpdate { update: Some(Update::Delta(text)), .. }) => {
                if !sink(Some(&text)) {
                    return Err(Status::cancelled("task cancelled"));
                }
            }
            Some(LocalProposalUpdate { update: Some(Update::ProposalJson(bytes)), used_tokens }) => {
                let proposal = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|value| {
                        Proposal::from_parts(value.get("calls"), value.get("text").and_then(Value::as_str))
                    })
                    .ok_or_else(|| Status::internal("invalid model proposal"))?;
                return Ok((proposal, used_tokens));
            }
            _ => return Err(Status::internal("incomplete inference stream")),
        }
    }
}

fn cloud_error(status: Status) -> &'static str {
    match (status.code(), status.message()) {
        (tonic::Code::Cancelled, _) | (tonic::Code::FailedPrecondition, "TASK_CANCELLED") => "TASK_CANCELLED",
        (tonic::Code::FailedPrecondition, "CLOUD_KEY_REQUIRED") => "CLOUD_KEY_REQUIRED",
        (tonic::Code::FailedPrecondition, "CREDENTIAL_STORE_UNAVAILABLE") => "CREDENTIAL_STORE_UNAVAILABLE",
        (tonic::Code::FailedPrecondition, "INVALID_API_KEY") => "INVALID_API_KEY",
        (tonic::Code::FailedPrecondition, "MODEL_REQUIRED") => "MODEL_REQUIRED",
        (tonic::Code::FailedPrecondition, "CLOUD_PROPOSAL_INVALID") => "CLOUD_PROPOSAL_INVALID",
        _ => "CLOUD_REQUEST_FAILED",
    }
}

fn inference_error(status: Status) -> GatewayError {
    match (status.code(), status.message()) {
        (tonic::Code::Cancelled, _) => GatewayError::Cancelled,
        (tonic::Code::FailedPrecondition, "local_runtime_unavailable") => GatewayError::Unavailable,
        _ => GatewayError::InvalidResponse,
    }
}

async fn call_worker<T, F>(
    timeout: Duration,
    invoke: impl FnOnce(ModelCatalogClient<Channel>) -> F,
) -> Result<T, &'static str>
where
    F: Future<Output = Result<T, Status>>,
{
    call_peer(timeout, "--model-catalog-worker", None, |channel| invoke(ModelCatalogClient::new(channel))).await
}

pub fn initialize_core(store: &Path) -> Result<String, &'static str> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| "CORE_UNAVAILABLE")?;
                runtime
                    .block_on(call_peer(
                        Duration::from_secs(30),
                        "--core-session-worker",
                        Some(store),
                        |channel| async move {
                            let reply = CoreSessionClient::new(channel)
                                .initialize(protocol::InitializeSession { schema_version: SCHEMA_VERSION })
                                .await?;
                            Ok(reply.into_inner().session_id)
                        },
                    ))
                    .map_err(|_| "CORE_UNAVAILABLE")
            })
            .join()
            .unwrap_or(Err("CORE_UNAVAILABLE"))
    })
}

pub async fn journal_page(store: &Path, session_id: String, after: Option<u64>) -> Result<Vec<Activity>, &'static str> {
    call_peer(CALL_TIMEOUT, "--journal-worker", Some(store), |channel| async move {
        let reply = JournalClient::new(channel)
            .read(JournalRequest { schema_version: SCHEMA_VERSION, session_id, after_sequence: after })
            .await?;
        Ok(reply
            .into_inner()
            .events
            .into_iter()
            .map(|event| Activity {
                sequence: event.sequence,
                kind: event.kind,
                timestamp: event.timestamp,
                task_id: event.task_id,
                code: event.code,
            })
            .collect())
    })
    .await
    .map_err(|_| "ACTIVITY_UNAVAILABLE")
}

async fn call_peer<T, F>(
    timeout: Duration,
    mode: &str,
    store: Option<&Path>,
    invoke: impl FnOnce(Channel) -> F,
) -> Result<T, &'static str>
where
    F: Future<Output = Result<T, Status>>,
{
    let _slot = WORKER_SLOTS.try_acquire().map_err(|_| UNAVAILABLE)?;
    let id = uuid::Uuid::new_v4();
    let mut command = tokio::process::Command::new(std::env::current_exe().map_err(|_| UNAVAILABLE)?);
    command.arg(mode).arg(id.to_string()).arg(std::process::id().to_string());
    if let Some(store) = store {
        command.arg(store);
    }
    let mut worker = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| UNAVAILABLE)?;
    let _lifetime = worker.stdin.take().ok_or(UNAVAILABLE)?;
    let pid = worker.id().ok_or(UNAVAILABLE)?;
    let call = async move {
        let channel = tokio::time::timeout(CONNECT_TIMEOUT, connect_channel(id, pid))
            .await
            .map_err(|_| UNAVAILABLE)?
            .map_err(|_| UNAVAILABLE)?;
        invoke(channel).await.map_err(|_| UNAVAILABLE)
    };
    tokio::select! {
        result = tokio::time::timeout(timeout, call) => result.map_err(|_| UNAVAILABLE)?,
        _ = worker.wait() => Err(UNAVAILABLE),
    }
}

pub(crate) async fn connect_channel(id: uuid::Uuid, pid: u32) -> Result<Channel, tonic::transport::Error> {
    let name = pipe_name(id);
    Endpoint::from_static("http://localhost")
        .connect_with_connector(tower::service_fn(move |_| {
            let name = name.clone();
            async move { pipe::connect(&name, pid).await.map(TokioIo::new) }
        }))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn cloud_stream_preserves_reported_usage() {
        let mut stream = proposal_stream(|sink| {
            assert!(sink(Some("Bonjour")));
            Ok((Proposal::Text("Bonjour".into()), 27))
        });
        let delta = stream.next().await.unwrap().unwrap();
        assert_eq!(delta.update, Some(Update::Delta("Bonjour".into())));
        assert_eq!(delta.used_tokens, 0);
        let final_update = stream.next().await.unwrap().unwrap();
        assert!(matches!(final_update.update, Some(Update::ProposalJson(_))));
        assert_eq!(final_update.used_tokens, 27);
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn worker_test_entry() {
        if let (Ok(id), Ok(parent)) =
            (std::env::var("AIWIN11_MODEL_IPC_TEST_PIPE"), std::env::var("AIWIN11_MODEL_IPC_TEST_PARENT"))
        {
            let mode = if let Some(store) = std::env::var_os("AIWIN11_MODEL_IPC_TEST_SESSION_STORE") {
                WorkerMode::CoreSession(store.into())
            } else {
                std::env::var_os("AIWIN11_MODEL_IPC_TEST_JOURNAL")
                    .map(PathBuf::from)
                    .map_or(WorkerMode::Model, WorkerMode::Journal)
            };
            std::process::exit(serve_worker(id.parse().unwrap(), parent.parse().unwrap(), mode));
        }
    }

    #[tokio::test]
    async fn core_session_is_created_once_by_the_worker() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("core.sqlite3");
        let id = uuid::Uuid::new_v4();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker::tests::worker_test_entry"])
            .env("AIWIN11_MODEL_IPC_TEST_PIPE", id.to_string())
            .env("AIWIN11_MODEL_IPC_TEST_PARENT", std::process::id().to_string())
            .env("AIWIN11_MODEL_IPC_TEST_SESSION_STORE", &store)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        tokio::time::timeout(CALL_TIMEOUT, async {
            let mut client = CoreSessionClient::new(connect_channel(id, pid).await.unwrap());
            let request = protocol::InitializeSession { schema_version: SCHEMA_VERSION };
            let first = client.initialize(request).await.unwrap().into_inner().session_id;
            let second = client.initialize(request).await.unwrap().into_inner().session_id;
            assert_eq!(first, second);
            let core = aiwin11_core::Core::attach_task_worker(&store, &first, std::process::id()).unwrap();
            assert_eq!(core.activity().unwrap().iter().filter(|event| event.kind == "session.started").count(), 1);
        })
        .await
        .unwrap();
        drop(child.stdin.take());
        assert!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await.unwrap().unwrap().success());
    }

    #[tokio::test]
    async fn grpc_catalog_contract_in_another_process() {
        let id = uuid::Uuid::new_v4();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker::tests::worker_test_entry"])
            .env("AIWIN11_MODEL_IPC_TEST_PIPE", id.to_string())
            .env("AIWIN11_MODEL_IPC_TEST_PARENT", std::process::id().to_string())
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let inference_model = std::env::var("AIWIN11_TEST_IPC_INFERENCE").ok();
        let cloud = std::env::var("AIWIN11_TEST_CLOUD_VENDOR")
            .ok()
            .map(|vendor| (vendor, std::env::var("AIWIN11_TEST_CLOUD_MODEL").expect("model required for cloud test")));
        let timeout = if inference_model.is_some() || cloud.is_some() { INFERENCE_TIMEOUT } else { CALL_TIMEOUT };
        tokio::time::timeout(timeout, async {
            let mut client = ModelCatalogClient::new(connect_channel(id, pid).await.unwrap());
            let request = ListLocalModelsRequest { schema_version: SCHEMA_VERSION + 1 };
            let error = client.list_local_models(request).await.unwrap_err();
            assert_eq!(error.code(), tonic::Code::InvalidArgument);
            let error = client.list_loaded_models(request).await.unwrap_err();
            assert_eq!(error.code(), tonic::Code::InvalidArgument);
            let route = RouteLocalModelRequest {
                schema_version: SCHEMA_VERSION,
                strategy: "manual".into(),
                preferred: "configured:latest".into(),
                excluded: vec!["excluded:latest".into()],
            };
            let unsupported = RouteLocalModelRequest { strategy: "cloud".into(), ..route.clone() };
            assert_eq!(client.route_local_model(unsupported).await.unwrap_err().code(), tonic::Code::InvalidArgument);
            assert_eq!(client.route_local_model(route).await.unwrap().into_inner().selected_model, None);
            let bad = LocalProposalRequest {
                schema_version: SCHEMA_VERSION,
                model: "local:latest".into(),
                messages_json: b"null".to_vec(),
                tools_json: b"[]".to_vec(),
                runtime: String::new(),
            };
            assert_eq!(client.propose_local(bad).await.unwrap_err().code(), tonic::Code::InvalidArgument);
            let bad_cloud = CloudProposalRequest {
                schema_version: SCHEMA_VERSION,
                vendor: "unknown".into(),
                model: "test".into(),
                messages_json: b"[]".to_vec(),
            };
            assert_eq!(client.propose_cloud(bad_cloud).await.unwrap_err().code(), tonic::Code::InvalidArgument);
            if std::env::var_os("AIWIN11_TEST_MODEL_IPC").is_some() {
                let request = ListLocalModelsRequest { schema_version: SCHEMA_VERSION };
                client.list_local_models(request).await.expect("Ollama catalog").into_inner();
                client.list_loaded_models(request).await.expect("Ollama loaded models").into_inner();
            }
            if std::env::var_os("AIWIN11_TEST_LM_STUDIO").is_some() {
                let models = client
                    .list_lm_studio_models(ListLocalModelsRequest { schema_version: SCHEMA_VERSION })
                    .await
                    .expect("LM Studio catalog")
                    .into_inner()
                    .names;
                assert_eq!(models, ["fixture/local-model"]);
                let request = LocalProposalRequest {
                    schema_version: SCHEMA_VERSION,
                    runtime: "lm_studio".into(),
                    model: models[0].clone(),
                    messages_json: br#"[{"role":"user","content":"Reply with 42."}]"#.to_vec(),
                    tools_json: b"[]".to_vec(),
                };
                let (proposal, usage) =
                    read_proposal(client.propose_local(request), &mut |_| true).await.expect("LM Studio stream");
                assert!(matches!(proposal, Proposal::Text(text) if text == "42"));
                assert_eq!(usage, 0);
            }
            if let Some((vendor, model)) = cloud {
                let request = CloudProposalRequest {
                    schema_version: SCHEMA_VERSION,
                    vendor,
                    model,
                    messages_json: br#"[{"role":"user","content":"Reply with just the number 42."}]"#.to_vec(),
                };
                let (proposal, used_tokens) =
                    read_proposal(client.propose_cloud(request), &mut |_| true).await.expect("cloud provider response");
                assert!(matches!(proposal, Proposal::Text(text) if !text.trim().is_empty()));
                assert!(used_tokens > 0);
            }
            if let Some(model) = inference_model {
                let request = LocalProposalRequest {
                    schema_version: SCHEMA_VERSION,
                    model,
                    messages_json: br#"[{"role":"user","content":"Answer with just the number 42."}]"#.to_vec(),
                    tools_json: b"[]".to_vec(),
                    runtime: String::new(),
                };
                let mut stream = client.propose_local(request).await.expect("local inference RPC").into_inner();
                let mut text = String::new();
                while let Some(update) = stream.message().await.expect("local inference stream") {
                    match update.update {
                        Some(Update::Delta(delta)) => text.push_str(&delta),
                        Some(Update::ProposalJson(bytes)) => {
                            let final_reply: Value = serde_json::from_slice(&bytes).unwrap();
                            assert_eq!(final_reply["text"].as_str(), Some(text.trim()));
                            assert!(!text.trim().is_empty());
                            return;
                        }
                        None => panic!("empty inference update"),
                    }
                }
                panic!("stream ended without a final proposal");
            }
        })
        .await
        .expect("gRPC response before the deadline");
        drop(child.stdin.take());
        assert!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await.unwrap().unwrap().success());
    }

    #[tokio::test]
    async fn journal_replays_across_connections() {
        let store = std::env::temp_dir().join(format!("aiwin11-journal-{}.sqlite3", uuid::Uuid::new_v4()));
        let core = aiwin11_core::Core::open(&store).unwrap();
        let session = core.session_id().to_owned();
        let cursor = core.activity().unwrap()[0].sequence;
        for _ in 0..105 {
            core.journal("ipc.test", None).unwrap();
        }
        let mut after = cursor;
        for expected_count in [100, 5, 0] {
            let id = uuid::Uuid::new_v4();
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "worker::tests::worker_test_entry"])
                .env("AIWIN11_MODEL_IPC_TEST_PIPE", id.to_string())
                .env("AIWIN11_MODEL_IPC_TEST_PARENT", std::process::id().to_string())
                .env("AIWIN11_MODEL_IPC_TEST_JOURNAL", &store)
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id().unwrap();
            let events = tokio::time::timeout(CALL_TIMEOUT, async {
                let channel = connect_channel(id, pid).await.unwrap();
                JournalClient::new(channel)
                    .read(JournalRequest {
                        schema_version: SCHEMA_VERSION,
                        session_id: session.clone(),
                        after_sequence: Some(after),
                    })
                    .await
                    .unwrap()
                    .into_inner()
                    .events
            })
            .await
            .unwrap();
            assert_eq!(events.len(), expected_count);
            for event in events {
                assert_eq!(event.sequence, after + 1);
                after = event.sequence;
            }
            drop(child.stdin.take());
            assert!(tokio::time::timeout(Duration::from_secs(2), child.wait()).await.unwrap().unwrap().success());
        }
        drop(core);
        std::fs::remove_file(store).unwrap();
    }
}
