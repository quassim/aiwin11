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

The bridge accepts the Notepad enum, checks the deadline, starts the Windows system executable and looks for a new Notepad window using the process and its window handle. Packaged Notepad is identified by package family. An existing window or reused tab can yield `PARTIAL`, which the broker records as `tool.execution.partial`. The process ID is returned only for a verified window. The host stops waiting for the bridge after seven seconds.

## Local provider discovery

The Ollama adapter calls the fixed loopback endpoint without system proxies or redirects. It filters remote aliases from the catalog and checks model details again. Tool calling and vision in discovery are advertised metadata. The tool-calling probe sends a fixed synthetic prompt and checks the returned structured call without dispatching it. The Notepad task sends a fixed instruction for its one supported action. The adapter receives no free-form user text.

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

The desktop exposes `discover_models`, `run_task` and `cancel_task` to its main window. The host recognizes the supported request and routes it to the chosen local Ollama model. The Python child receives the model's proposed calls as a single bounded JSON v1 message and returns a bounded JSON v1 operation. This private handoff uses inherited process pipes and a small JSON format to keep the Python child dependency-free. The host accepts only the fixed `launch_notepad` operation and constructs the Protobuf command itself. The child has no broker endpoint.

The host accepts one task at a time. Cancellation wins only before dispatch. A scoped approval applies to the current request, and the broker evaluates it at execution time. Task and execution events share a task ID. Prompts and model replies are excluded from SQLite events.

## Optional OpenAI provider

`run_task` takes a `request` with an explicit `provider` selector and separate `cloudApproved` and `approved` flags. Its `progress` channel delivers the persisted request, planning and agent proposal events to the calling window as they occur. The host records the cloud authorization decision before inference. Each provider failure ends the request. OpenAI receives the selected model ID, fixed action instruction and tool schema over HTTPS with `store: false`. Its response is converted to the same proposal format as Ollama and validated by the Python agent before broker authorization.

The main window can save, remove and check the presence of the `AiWin11/OpenAI` credential. Rust stores it as a generic entry in Windows Credential Manager for the current user on the local machine. The presence command returns a boolean. The key is read in the native gateway for the OpenAI request and excluded from SQLite, agent input and error messages.
