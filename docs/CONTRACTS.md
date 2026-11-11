# Component Contracts v1.0

## Transport
Tauri typed commands for UI -> core. Versioned protobuf for local services. Prefer named pipes with ACLs and peer identity verification (verify gRPC transport support during AW-003 prototype). No exposed network interfaces by default. Internal events are task-ordered; consumers handle duplicate delivery.

## Universal command envelope
schema_version, request_id, session_id, task_id, correlation_id, authenticated_actor_id, operation, payload, deadline_ms, idempotency_key. Actor identity must derive from authenticated peer, not a caller-supplied string.

## Universal result
schema_version, request_id, status [success,partial,failed,denied,cancelled,timeout], result, error {code,message,retryable}, duration_ms. Never leak secrets in errors.

## Event envelope
id, sequence, timestamp_utc, session_id, task_id, type, source, payload. Events: task.created, task.started, task.progress, agent.tool.requested, policy.approval.required, tool.execution.completed, model.provider.unavailable, task.completed, task.failed, task.cancelled.

## Model Gateway
ProviderRegistry.Register/Update/Disable/Test; ModelCatalog.Discover/List/DescribeCapabilities; ModelRouter.Resolve/Rank/Select/ExplainSelection; InferenceService.Generate/Stream/Cancel; ResourceMonitor.ReadGPU/ReadVRAM/ReadRAM; UsageService.RecordUsage/EstimateCost/GetStats. Models report input/output modalities, streaming/tool_calling/structured_output/vision, context limit (nullable), hardware (nullable), verified status.

## Agent Runtime
Task.Create/Start/Cancel/Observe; ToolRequest validated against known tool schema and broker policy. Agent manifest declares required capabilities and limits (max steps/calls/runtime/parallelism), but broker grants rights independently.

## Execution Broker
EvaluateAction, RequestApproval, GrantCapability, RevokeCapability, ExecuteAuthorized, CancelExecution, GetAuditTrail. Outcomes ALLOW/DENY/ASK/SIMULATE. Policy matches subject, action, resource, constraints and risk. Check again just before side effect. Canonical path checks.

## Windows Bridge
WindowService.List/Focus/Resize/Minimize/Close; ApplicationService.Discover/Launch/GetStatus; FileService.List/Read/Copy/Move/Delete/Search; ProcessService.List/Inspect/Terminate; DesktopService.CaptureScreen/GetDisplays/GetActiveWindow; UIAutomationService.InspectTree/FindElement/Invoke/SetValue; InputService.MovePointer/Click/Type/Hotkey; ShellService.ExecuteApprovedCommand; SystemService.GetHardwareInfo/GetSystemMetrics.

## Supporting services
MemoryService: Add/Query/Delete/Export and indexing scope. WorkflowService: Create/Validate/Run/Pause/Resume/Cancel. Speech/Image/Video services: provider-neutral asynchronous job status, capabilities and transfer/privacy controls. PluginService: discover MCP tools, permissions, execute through broker. TelemetryService: local metrics and audited traces.

## Storage
SQLite tables: providers, models, model_profiles, agents, sessions, tasks, tool_executions, policies, approvals, events, memory_entries, workflows, artifacts. All schema migrations versioned; credentials outside SQLite. Binary artifact files stored separately with metadata and checksums.

## M1 test acceptance
- User chat opens Notepad through a broker-authorized, audited tool call.
- Unapproved tool is denied with no side effect.
- Missing provider causes safe failure; unauthorized fallback to cloud impossible.
- Cancellation prevents further tool steps.
- No plaintext secrets or remote bind by default.

## First implementation

- The Tauri command `get_snapshot` reads the current session ID and up to 100 session events, newest first. Errors use fixed codes and contain no database diagnostics.
- Cargo generates the Rust types from `contracts/v1/aiwin11.proto`. The command contains a typed operation and an absolute Unix deadline in milliseconds. The core uses the peer identity supplied by the trusted host for authorization. The envelope's actor string has no authority. Cross-process transport and peer authentication are pending.
- Migration 001 uses SQLite `user_version` and creates sessions, events and the tool dispatch reservations. Migration and initial session creation run in one transaction. The core refuses schemas newer than it supports.
- Before dispatch, the broker commits an authorization event and a reservation keyed by session and idempotency key. It checks authorization again before invoking the trusted executor and records the outcome. If a process crashes after reservation or the outcome cannot be written, the reservation prevents repeat dispatch with the same key in that session.

## Windows child process

`Core::execute_windows` invokes the fixed `AiWin11.Windows.exe` installed beside the host executable after broker authorization. It sends one raw v1 `Command` on the child's private stdin and closes the writer. The child returns one raw v1 `Result` on stdout and exits. Each message is limited to 1 KiB. The host checks the schema, request ID and result fields. There is no listening service. The host derives the actor from its trusted context.

The bridge accepts the Notepad enum, checks the deadline, starts the Windows system executable and looks for a new Notepad window using the process and its window handle. Packaged Notepad is identified by package family. An existing window or reused tab can yield `PARTIAL`, which the broker records as `tool.execution.partial`. The process ID is returned only for a verified window. The bridge stops verifying at the command deadline and then reports. The host waits until that deadline plus two seconds (25 seconds at most) before killing the bridge, so a slow .NET cold start still produces a reported outcome. An approved launch carries a 20-second deadline: window verification still stops 4 seconds after the start, and the margin covers the first start of a newly installed bridge, which was observed above 10 seconds once on the reference machine.

## Local provider discovery

The Ollama adapter calls the fixed loopback endpoint without system proxies or redirects. It filters remote aliases from the catalog and checks model details again. Tool calling and vision in discovery are advertised metadata. The tool-calling probe sends a fixed synthetic prompt and checks the returned structured call without dispatching it. Chat tasks send fixed agent instructions, the user's request and the tool catalog (see "Agent planning").

## UI Automation inspector

The main window exposes `inspect_foreground` through the Inspector page. Its three-second countdown gives the user time to switch windows. The host issues a v1 `InspectForeground` command through the broker under the desktop inspector role. The agent role cannot authorize this operation. The bridge reads the control view of the foreground window and discards the result if the foreground window changes during collection.

`UiTree` returns up to 40 nodes, six levels below the root at most, and a truncation flag. It covers controls belonging to the foreground window’s process. Each node contains a control type ID, enabled state, focus state and a one-based parent index. Zero denotes the root parent. Only structure is returned to the calling window. The SQLite journal holds the decision and outcome codes. The inspector uses no provider.

## Foreground window capture

The Inspector page invokes `capture_foreground` after the same countdown. The broker authorizes the v1 `CaptureForeground` operation for the desktop inspector role. The bridge requests rendering of the selected HWND with Win32 PrintWindow. It rejects windows with display-affinity restrictions and discards the image if the foreground window changes during the operation.

`WindowCapture` carries a JPEG of at most 256 KiB with dimensions capped at 1024 px per side. The Rust host drains the pipe during execution and caps the full capture reply at 257 KiB. The image stays in memory and is displayed in the calling WebView. SQLite receives the decision and outcome codes.

The preview offers explicit analysis with an installed Ollama vision model. The broker authorizes `AnalyzeCapture` for the desktop inspector role. The gateway checks the model's local metadata and requires `/api/status` to confirm that Ollama cloud features are disabled immediately before sending the JPEG to loopback. The description is rendered as plain text. The journal records authorization and the outcome code.

## Controlled Character Map input

The Inspector page submits `enter_character_map_digit` after explicit user action and a three-second countdown. The broker authorizes the v1 `CharacterMapDigit` operation for the desktop inspector role and requires one decimal digit. The bridge checks the foreground process path against the system copy of Character Map, then uses UI Automation to identify visible, editable control 104 and validate its native handle. It checks the foreground window and deadline immediately before sending a bounded WM_SETTEXT to that control, then verifies the value with WM_GETTEXT. The field value stays inside the bridge. The journal holds decision and outcome codes.

## Local text preview and copy

The Files page submits `preview_local_text` with one file name. The broker authorizes the v1 `PreviewLocalText` operation for the desktop inspector role. The core resolves the candidate under the dedicated local app-data `files` folder and validates the opened handle's final path, file attributes and link count before reading. It accepts flat ASCII `.txt` names and UTF-8 content up to 64 KiB. The result is displayed as plain text in the requesting window. SQLite records the operation and outcome codes. The filename and text stay out of the audit event and model gateways.

`copy_local_text` takes the source and destination names and requires the separate v1 `CopyLocalText` permission. Both names follow the preview rules. The core validates the source and opens the destination directory by handle. It writes and flushes a staging file before publishing it by a same-directory rename with replacement disabled. The copied bytes preserve the source BOM and line endings. A failed write or rename triggers cleanup by file handle. An abrupt process termination can leave a staging file. SQLite records the operation and outcome codes without the names or contents.

## First chat task

The desktop exposes `discover_models`, `run_task` and `cancel_task` to its main window. `run_task` accepts a free-form request of 1 to 2000 characters after trimming (`PROMPT_INVALID` otherwise) and resolves to a `TaskOutcome`: `{ kind: "launched", application: "notepad", processId }` or `{ kind: "reply", text }`.

The host accepts one task at a time. Cancellation wins only before dispatch. Tool actions require an approval given after the action is proposed (see "Action approval"); the broker evaluates the decision at execution time. Task and execution events share a task ID. Prompts and model replies are excluded from SQLite events.

## Action approval (AWAITING_APPROVAL)

`run_task`'s request carries `provider`, `prompt`, `model` and `cloudApproved` only. When the agent's decision is a validated tool request, the host:

1. records `policy.approval.required` and moves the task to AWAITING_APPROVAL;
2. sends `{ "type": "approvalRequired", "approvalId", "tool", "application", "expiresInMs" }` on the progress channel. The approval ID is a random UUID generated by the host and known only to the requesting window;
3. waits up to 60 seconds for `resolve_approval({ approvalId, approved })`, a command granted to the main window only. Any other ID returns `APPROVAL_NOT_PENDING`; an ID is consumed by its first decision or by expiry;
4. records `policy.approval.granted`, `policy.approval.declined` or `policy.approval.expired` and emits it as a stage;
5. builds the command and its deadline, then submits it to the broker with the decision as policy. Declined and expired approvals therefore reach the broker and are audited as `policy.denied`; they end with `ACTION_DECLINED` or `APPROVAL_EXPIRED`.

`cancel_task` succeeds while planning or while an approval is pending, and wakes the waiting task, which ends as `TASK_CANCELLED` without dispatch. The model and the agent cannot answer an approval: they never see its ID and have no access to the command.

## Agent planning (AW-010, protocol v2)

1. The gateway sends fixed French agent instructions (system message), the user's request (user message) and the tool catalog to the selected provider. The catalog holds one tool, `launch_application`, whose only argument `application` is an enum limited to `notepad`. OpenAI receives the schema with `strict: true`.
2. The provider reply is normalized into a proposal: `{ "calls": [...] }` when the model returned tool calls, otherwise `{ "text": "..." }` with 1 to 4000 characters. Truncated, empty or malformed replies fail with `MODEL_UNAVAILABLE_OR_INVALID` or `CLOUD_PROPOSAL_INVALID`.
3. The Python child receives `{ "version": 2, "proposal": ... }` (at most 32 KiB) on stdin. It validates the proposal against its tool registry: exactly one call, a registered tool name, exactly the declared arguments, each within its allowed values. It returns `{ "version": 2, "action": "tool", "tool", "arguments" }` or `{ "version": 2, "action": "reply", "text" }` (at most 32 KiB), or exits with code 2. The host drains stdout concurrently and kills the child after four seconds.
4. The host parses the decision with unknown fields denied and validates it again. A reply ends the task with the `agent.reply` event and no broker call. A tool request is mapped by the host to the Protobuf `LaunchApplication` operation and submitted to the broker under the agent peer identity.

Model output is untrusted data. The registry bounds what the agent may request; the broker still decides what runs.

## Agent loop and read-only file tools (AW-016, first step)

A task runs at most 5 model proposals. Each proposal is validated by the Python agent and the host as above, then:

- a text reply ends the task (`agent.reply`);
- `launch_application` ends the task through the approval flow below;
- `list_text_files` or `read_text_file` runs a read-only observation, records `agent.tool.requested` then `agent.observation`, and appends the call and its result to the conversation sent to the model for the next proposal.

After 5 proposals without a final answer the task fails with `AGENT_STEP_LIMIT`.

File tools cover the dedicated `files` folder under the app's local data directory, with the same rules as the Files page: flat ASCII `.txt` names, handle-verified paths, no links, 64 KiB maximum, UTF-8. `list_text_files` returns at most 100 sorted names of regular `.txt` files; `read_text_file` returns the text without BOM.

- **Local only.** The host offers file tools, and grants `Policy.allow_agent_file_reads`, only when the provider is Ollama and the folder is available. File contents never reach a cloud provider. A file tool returned in any other case is rejected (`AGENT_REJECTED`).
- **Broker.** Each call is a v1 `ListLocalText` or `PreviewLocalText` command under the agent identity, with its own request ID and idempotency key and the chat task ID. The broker allows these reads for the agent only with `allow_agent_file_reads`; it never extends to copy, move, delete or launch. Each call is audited (`policy.allowed`, then `file.list.*` or `file.preview.*`), without names or contents.
- **No approval for reads.** Reading the dedicated folder is a low-risk observation, as in the specification's policy table. Actions with side effects keep their approval.
- **Untrusted data.** Observations reach the model framed as untrusted data (`<<<DEBUT … FIN>>>`), with at most 16 000 characters per file. Tool failures return only a stable error code. Instructions inside a file cannot do more than the user's own request: any side effect still requires the user's approval and the broker.
- **Context.** Local requests use `num_ctx` 16384 and `num_predict` 1024 so that observations fit.

### Agent file copy

`copy_text_file(source, destination)` is offered with the file tools (local models only). Both names follow the file rules and must differ, ignoring case.

1. **Dry run.** Under the read grant, the host reads the source and checks that no entry named `destination` exists (`file.preview.*` audit). If that fails (`FILE_UNAVAILABLE`, `FILE_DESTINATION_EXISTS`, `FILE_SAME_NAME`…), no approval is requested: the failure code goes back to the model as an observation and the loop continues.
2. **Approval.** `approvalRequired.action` is `{ "kind": "copyFile", "source", "destination", "characters" }`, shown to the user as is. Launch approvals use `{ "kind": "launch", "application": "notepad" }`.
3. **Broker.** The host submits a v1 `CopyLocalText` command under the agent identity with `Policy.allow_agent_file_copy` set only if the user approved. That grant covers copy only. A refusal is audited as `policy.denied` and ends with `ACTION_DECLINED` or `APPROVAL_EXPIRED`.
4. **Copy.** The existing copy runs: staging file, flush, same-folder rename with replacement disabled, cleanup on failure. The task ends with `TaskOutcome { "kind": "copied", "source", "destination" }` or the file error code; the model does not narrate the result.

## Streaming and progress (AW-011)

Both providers are called with `stream: true`. The gateway folds Ollama's NDJSON chunks or OpenAI's SSE `data:` events into the same proposal as above, with the same bounds (2 MiB body, 4000 reply characters, one tool call). Every Ollama chunk is checked for remote metadata. Tool-call fragments are assembled silently and never forwarded.

`run_task`'s `progress` channel carries `TaskUpdate` messages:

- `{ "type": "stage", "stage": "<event kind>" }` after each persisted lifecycle event (`task.created`, `task.planning`, `agent.reply`, `agent.tool.requested`, `policy.approval.granted|declined|expired`);
- `{ "type": "approvalRequired", ... }` when an action awaits the user's decision (see "Action approval");
- `{ "type": "delta", "text": "..." }` for each streamed text fragment.

Deltas are unvalidated model output, shown as a draft and never journaled. The window discards the draft when the task ends and displays only the agent-validated reply from `TaskOutcome`. After each chunk the host checks the task phase: a cancellation requested while the model streams stops reading, which closes the provider connection, and the task ends as `TASK_CANCELLED` without dispatch. A provider that sends no chunk at all can still hold the task until its timeout (90 s local, 45 s OpenAI). Adding a tool requires a catalog entry, an agent registry entry, a host mapping, a Protobuf operation, a broker rule and bridge support.

## Optional OpenAI provider

`run_task` takes a `request` with an explicit `provider` selector and a `cloudApproved` flag (tool actions are approved separately, see "Action approval"). Its `progress` channel delivers the persisted request, planning and agent proposal events to the calling window as they occur. The host records the cloud authorization decision before inference. Each provider failure ends the request. OpenAI receives the selected model ID, the fixed agent instructions, the user's request and the tool catalog over HTTPS with `store: false`, only when `cloudApproved` is set for that request. Its response is converted to the same proposal format as Ollama and validated by the Python agent before broker authorization.

The main window can save, remove and check the presence of the `AiWin11/OpenAI` credential. Rust stores it as a generic entry in Windows Credential Manager for the current user on the local machine. The presence command returns a boolean. The key is read in the native gateway for the OpenAI request and excluded from SQLite, agent input and error messages.
