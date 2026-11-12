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
- Migration 001 uses SQLite `user_version` and creates sessions, events and the tool dispatch reservations. Migration 002 adds `settings` (policy profile) and `tasks` (one row per chat task, see "Tasks and recovery"); a version 1 database migrates in place. Migrations, task recovery and initial session creation run in one transaction. The core refuses schemas newer than it supports.
- Before dispatch, the broker commits an authorization event and a reservation keyed by session and idempotency key. It checks authorization again before invoking the trusted executor and records the outcome. If a process crashes after reservation or the outcome cannot be written, the reservation prevents repeat dispatch with the same key in that session.

## Windows child process

`Core::execute_windows` invokes the fixed `AiWin11.Windows.exe` installed beside the host executable after broker authorization (using the spare bridge described below when one is waiting). It sends one raw v1 `Command` on the child's private stdin and closes the writer. The child returns one raw v1 `Result` on stdout and exits. A command is limited to 16 KiB (room for 2000 characters of Notepad text); a reply to 1 KiB, except window trees (32 KiB) and captures (257 KiB). A command without a dedicated handler in the bridge is denied (`unsupported_command`); the Notepad launch branch is explicit. The host checks the schema, request ID and result fields. There is no listening service. The host derives the actor from its trusted context.

The bridge accepts the Notepad enum, checks the deadline, starts the Windows system executable and looks for a new Notepad window using the process and its window handle. Packaged Notepad is identified by package family. An existing window or reused tab can yield `PARTIAL`, which the broker records as `tool.execution.partial`. The process ID is returned only for a verified window. The bridge stops verifying at the command deadline and then reports. The host waits until that deadline plus two seconds (25 seconds at most) before killing the bridge, so a slow .NET cold start still produces a reported outcome. An approved launch carries a 20-second deadline: window verification still stops 4 seconds after the start, and the margin covers the first start of a newly installed bridge, measured at 17.9 seconds (then 60 ms) right after republishing it on the reference machine.

**Spare bridge.** To keep that startup cost out of every command deadline, the host keeps one bridge process started ahead of time (`prewarm_bridge`, called in a background thread when the desktop starts and after each dispatch). The spare blocks on its private stdin and does nothing until it receives a command, which the host only sends after broker authorization. It has no listener and no network access. `dispatch` takes the spare if it is still running, otherwise starts a bridge on demand, and starts the next spare. When the host exits, the spare's stdin closes and it exits with code 3 without writing anything. Measured on a never-run bridge binary: the spare absorbed about 9 seconds of first start, and the following command completed in under 2 seconds.

## Local provider discovery

The Ollama adapter calls the fixed loopback endpoint without system proxies or redirects. It filters remote aliases from the catalog and checks model details again. Tool calling and vision in discovery are advertised metadata. The tool-calling probe sends a fixed synthetic prompt and checks the returned structured call without dispatching it. Chat tasks send fixed agent instructions, the conversation so far and the tool catalog (see "Agent loop").

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

## Chat task

The desktop exposes `discover_models`, `run_task`, `cancel_task` and `resolve_approval` to its main window. `run_task` takes a `request { provider, prompt, model, cloudApproved, visionModel? }` and a `progress` channel. The prompt is 1 to 2000 characters after trimming (`PROMPT_INVALID` otherwise). The host accepts one task at a time (`TASK_BUSY`).

The task resolves to `TaskOutcome { reply, actions }`:

- `reply` is the model's final answer, validated by the agent (1 to 4000 characters);
- `actions` is the factual report written by the host, one entry per action the agent asked for: `{ action, summary, status, code }`, where `status` is `done`, `partial`, `simulated`, `declined`, `expired`, `denied` or `failed`, and `code` gives the policy rule or error code for anything other than `done`. The model never writes this report.

Prompts, replies, file names, file contents, window titles and typed text are excluded from SQLite events, which hold kinds and stable codes only.

## Agent loop

A task runs at most 8 model proposals; after that it fails with `AGENT_STEP_LIMIT`.

1. The gateway sends the fixed French agent instructions, the user's request and the previous tool steps, in the provider's own format (Ollama: `tool_calls` + `tool_name`; OpenAI Chat Completions: call IDs + `tool_call_id`), with the tool catalog. Local requests use `num_ctx` 16384 and `num_predict` 1024.
2. The streamed reply is folded into a proposal: tool calls or a text answer (see "Streaming and progress").
3. The Python child (protocol v2, at most 32 KiB in and out, four seconds) validates the proposal against its registry: exactly one call, a registered tool, exactly the declared arguments, each within its allowed values or validator. It returns a typed tool request or a reply, or exits with code 2.
4. The host parses the decision with unknown fields denied and validates it again.
5. A reply ends the task. A tool request goes through the policy gate, runs if allowed, and its observation is appended to the conversation for the next proposal (`agent.tool.requested`, then `agent.observation`).

Observations are untrusted data: file contents, window trees and descriptions are framed as `<<<DEBUT … FIN>>>` with an instruction not to follow them; failures return stable codes. Instructions found in a file or a window can at most make the model propose an action, which still goes through the policy engine, the user's approval when required, and the broker.

### Tools

| Tool | Arguments | Policy action | Capability | Local only |
|---|---|---|---|---|
| `launch_application` | `application: "notepad"` | launchApplication | LaunchNotepad | no |
| `list_text_files` | — | listFiles | FileRead | yes |
| `read_text_file` | `name` | readFile | FileRead | yes |
| `copy_text_file` | `source`, `destination` | copyFile | FileCopy | yes |
| `move_text_file` | `source`, `destination` | moveFile | FileMove | yes |
| `trash_text_file` | `name` | trashFile | FileTrash | yes |
| `inspect_window` | — | inspectWindow | WindowLookup, then WindowInspect | yes |
| `describe_window` | — | describeWindow | WindowLookup, then WindowCapture | yes, and only with a chosen vision model |
| `type_in_notepad` | `text` (1–2000 chars) | typeText | TypeText | yes |

File names are flat ASCII `.txt` names (letters, digits, space, `-`, `_`; no reserved device names; at most 128 bytes); source and destination must differ ignoring case. No tool takes a path, a window handle, a process ID or a command line: the host decides every target. Local-only tools are not offered to cloud models and are denied by the policy engine if proposed anyway.

## Policy engine

`policy::evaluate(settings, action, context)` returns `Allow`, `Ask`, `Deny` or `Simulate`, with the rule that produced it. It starts from a matrix (action × profile) and applies invariants that no profile can relax:

| Action | Observation | Assisted (default) | Limited | Advanced |
|---|---|---|---|---|
| list / read files | Allow | Allow | Allow | Allow |
| inspect / describe a window | Ask | Ask | Ask | Ask |
| launch Notepad, copy, type in Notepad | Deny | Ask | Allow | Allow |
| rename | Deny | Ask | Ask | Allow |
| trash | Deny | Ask | Ask | Ask |

Invariants, in order: local-only tools are denied to cloud models (`local_only`); a profile denial stays a denial; with **simulation mode** every remaining side effect becomes a dry run (`simulation_mode`); screen reading always asks (`screen_data`); trash always asks (`critical_action`). Unknown or unreadable settings resolve to Observation.

Settings live in the `settings` table (`policy.autonomy`, `policy.simulate`), are changed through `set_policy_settings` (main window only, journaled as `settings.policy.changed`) and are read once per task (`policy.profile` event): a running task keeps the settings it started with. `get_policy_overview` returns the settings and the decision table computed by the core.

Model preferences (`model.provider` = `ollama`|`openai`, `model.local`, `model.cloud`, `model.vision`) are read with `get_model_preferences` and written with `set_model_preferences` (`ModelPreferences { provider, localModel, cloudModel, visionModel }`; identifiers ≤ 256 characters without control characters, otherwise `SETTINGS_INVALID`), journaled as `settings.models.changed` with the provider code only. The cloud approval and the API key are never stored there. Invalid stored values resolve to the local provider with nothing selected.

### Gate for one action

1. **Decide.** The engine's decision is journaled as `policy.evaluated` with code `effect:action:rule`. A denial is reported (`denied`) and returned to the model as an observation.
2. **Prepare, read-only.** Copy, rename and trash dry-run first (readable source, free destination); window tools locate the window. A failed preparation is reported (`failed`) and asks nothing.
3. **Authorize.** `Simulate`: `policy.simulated`, reported `simulated`, the model learns what would have happened. `Ask`: approval flow below. `Allow` or approved: the host requests exactly one capability.
4. **Execute.** Side effects run in the DISPATCHING phase (task state `dispatching`); the broker checks the capability, identity, scope, identifiers and deadline again at execution time, and audits the decision and outcome. Each command has its own request ID and idempotency key.

### Broker capabilities

`broker::Policy` is a set of `Capability` grants, empty by default. Agent rules: launch the Notepad enum (`LaunchNotepad`); list and preview text (`FileRead`); copy (`FileCopy`); rename (`FileMove`); trash (`TrashLocalText`, `FileTrash`); locate (`WindowLookup`), inspect (`WindowInspect`) and capture (`WindowCapture`) a window; type into Notepad (`TypeText`, text 1–2000 characters). Desktop-inspector rules (Inspector and Files pages) are unchanged. Anything else is `default_deny`.

## Action approval (AWAITING_APPROVAL)

When the decision is `Ask`, the host:

1. records `policy.approval.required` and moves the task to AWAITING_APPROVAL (task state `awaiting_approval`);
2. sends `{ "type": "approvalRequired", "approvalId", "action", "expiresInMs" }` on the progress channel. `action` describes the exact action: `launch`, `listFiles`, `readFile`, `copyFile` / `moveFile` (source, destination, characters), `trashFile` (name, characters), `inspectWindow` (application, title), `describeWindow` (application, title, model) or `typeText` (full text, characters). The approval ID is a random UUID generated by the host and known only to the requesting window;
3. waits up to 60 seconds for `resolve_approval({ approvalId, approved })`, main window only. Any other ID returns `APPROVAL_NOT_PENDING`; an ID is consumed by its first decision or by expiry;
4. records `policy.approval.granted`, `policy.approval.declined` or `policy.approval.expired`. A refusal or expiry is also journaled as `policy.denied` (`user_declined` / `approval_expired`), reported in the outcome, returned to the model as an observation, and requests no capability.

`cancel_task` succeeds while planning or while an approval is pending, and wakes the waiting task, which ends as `TASK_CANCELLED`. Once an action is dispatching it is no longer cancellable. The model and the agent cannot answer an approval: they never see its ID and have no access to the command.

## Agent file tools

All file tools cover the dedicated `files` folder under the app's local data directory, with the rules of the Files page: handle-verified paths, no links (hard links refused when read), 64 KiB maximum, UTF-8. Audit events hold codes only (`file.list.*`, `file.preview.*`, `file.copy.*`, `file.move.*`, `file.trash.*`).

- `list_text_files`: at most 100 sorted names of regular `.txt` files. `read_text_file`: the text without BOM, at most 16 000 characters passed to the model.
- `copy_text_file`: staging file, flush, same-folder rename with replacement disabled, cleanup on failure.
- `move_text_file`: rename of the handle-opened file inside the folder, replacement disabled.
- `trash_text_file`: the file is moved, by handle-anchored rename, into a new subfolder of AiWin11's own `trash` folder (next to `files`), keeping its name. Nothing is deleted; it can be restored by moving it back. A failed trash leaves no empty folder.

## Agent window tools

`inspect_window` and `describe_window` take no argument: the model cannot choose the window.

1. **Locate.** v1 `LocateWindow { exclude_process_id: <host pid> }`: the bridge walks the top-level z-order and returns the first window that is visible, not minimized, unowned, not cloaked, not tool/topmost/no-activate, not a shell window, with a title, and not owned by the host or the bridge: `WindowInfo { window, process_id, title ≤ 120 chars, process_name ≤ 64 chars }`. The title is shown to the user in the approval; the model sees it only after approval.
2. **Inspect.** v1 `InspectWindow { window, process_id }`: the bridge checks before and after the walk that the handle still exists and belongs to that process, then returns up to 80 nodes, 8 levels deep: control type, enabled, focus and accessible name (single line, ≤ 80 chars, always empty for password fields). No pixels. Audit `window.inspect.*`.
3. **Describe.** v1 `CaptureWindow { window, process_id }`: the bridge renders exactly that window with PrintWindow (it need not be in the foreground), checks the process before and after, refuses display-affinity windows, and returns a JPEG of at most 1024 px per side and 256 KiB. The host passes it to the chosen local Ollama vision model, after checking again that the model is local and that Ollama's cloud features are disabled. Only the text description reaches the agent; the image is kept in memory for the call and never stored or sent elsewhere. Audit `window.capture.*`, `vision.describe.*`.

The desktop Inspector page is unchanged: it still reads the foreground window's structure only, without names, after its countdown.

## Agent text input in Notepad

`type_in_notepad(text)` writes into the Notepad window **this task launched and verified**: the host keeps that process ID from the launch result; without one, nothing is asked or written and the model is told to launch Notepad first. v1 `TypeInNotepad { process_id, text }`: the bridge checks the process is Notepad (package family or system path), finds its visible editor (`RichEditD2DPT` or `Edit`), refuses a non-empty document (`document_not_empty`, so restored tabs or user text are never replaced), writes with `WM_SETTEXT` (no simulated keystrokes) and reads the text back. Outcomes: verified (`done`), sent but not verified (`partial`, `text_unverified`), or refused. The approval shows the full text. Audit `input.notepad.*`.

## Tasks and recovery

Each chat task has a row in `tasks` (`id`, `session_id`, `state`, `outcome`, timestamps), updated at each step: `created`, `planning`, `awaiting_approval`, `dispatching`, then `completed` (outcome `ok`), `failed` (error code) or `cancelled`. Final states never change.

At startup, tasks still in a non-final state were interrupted by a crash or a forced exit. They are closed as `interrupted`, keeping their last state as outcome, and journaled as `task.recovered` in the new session. Nothing is re-run, so an action that may have happened is never repeated; a task interrupted while `dispatching` is flagged in the Activity page so the user can check its result. Within a session, the broker's idempotency reservations already prevent repeated dispatch.

`get_tasks` (main window only) returns the 30 most recent tasks across sessions. `get_snapshot` returns up to 100 events of the current session with their task ID and code.

## Streaming and progress (AW-011)

Both providers are called with `stream: true`. The gateway folds Ollama's NDJSON chunks or OpenAI's SSE `data:` events into one proposal, with the same bounds (2 MiB body, 4000 reply characters, one tool call). Every Ollama chunk is checked for remote metadata. Tool-call fragments are assembled silently and never forwarded; OpenAI call IDs are kept for the next turn.

`run_task`'s `progress` channel carries `TaskUpdate` messages:

- `{ "type": "stage", "stage": "<event kind>" }` after persisted lifecycle events (`task.created`, `task.planning`, `agent.tool.requested`, `agent.observation`, `policy.simulated`, `policy.approval.granted|declined|expired`, `agent.reply`);
- `{ "type": "approvalRequired", ... }` when an action awaits the user's decision;
- `{ "type": "delta", "text": "..." }` for each streamed text fragment.

Deltas are unvalidated model output, shown as a draft and never journaled; the window clears the draft at each tool step and when the task ends. After each chunk the host checks the task phase: a cancellation requested while the model streams stops reading, which closes the provider connection. A provider that sends no chunk at all can still hold the task until its timeout (90 s local, 45 s OpenAI).

Adding a tool requires a catalog entry, an agent registry entry, a host mapping, a policy action and matrix row, a Protobuf operation, a broker capability and rule, and bridge or core support.

## Optional OpenAI provider

The host records the cloud authorization decision (`model.cloud.authorized` / `model.cloud.denied`) before inference. OpenAI receives the selected model ID, the fixed agent instructions, the user's request, the previous steps and a catalog limited to `launch_application`, over HTTPS with `store: false`, only when `cloudApproved` is set for that request. No file content, window data or image is ever sent to it. Its response is converted to the same proposal format as Ollama and validated by the Python agent.

The main window can save, remove and check the presence of the `AiWin11/OpenAI` credential. Rust stores it as a generic entry in Windows Credential Manager for the current user on the local machine. The presence command returns a boolean. The key is read in the native gateway for the OpenAI request and excluded from SQLite, agent input and error messages.
