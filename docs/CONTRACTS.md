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

The local routing strategy (`settings` key `routing.local_strategy`, commands `get_local_strategy` / `set_local_strategy`, journaled as `settings.routing.strategy.changed`) applies to interactive Ollama tasks: `manual` (default) keeps the configured model; `adaptive` picks the loaded model with the largest fraction held in VRAM, ties favoring the configured model; `qualityFirst` picks the largest installed local model (size on disk from `/api/tags`) that advertises tool calling; `costOptimized` keeps the configured model if it is already loaded, otherwise picks the smallest loaded model, so that no new model is loaded. A model other than the configured one must pass the synthetic tool-call probe before use; otherwise the configured model is retained. The older `routing.adaptive_local = true` switch is read as `adaptive`; unknown codes mean `manual`. Any strategy other than `manual` disables cloud fallback. The requesting window receives a `modelSelected` update with the model identifier and the reason (`gpu_verified`, `quality_first`, `cost_optimized` or `configured_local`); the journal records only the reason code (`model.selected`).

Local video uses `start_video` and `video_status`. Only the main desktop can authorize `GenerateLocalVideo`. A bounded ComfyUI API graph must use the local node allowlist, one text placeholder and one `SaveVideo` output. The host submits to the configured loopback port with proxies and redirects disabled, then polls the returned job ID. The generated file stays in ComfyUI's output directory. The audit records the operation and outcome codes.

Signed desktop updates use Tauri Updater with a public key embedded at packaging time. `check_update` consults the AiWin11 GitHub release manifest over HTTPS and returns an available version. `install_update` takes that version, rechecks the release and verifies the downloaded installer signature before installation. The task slot must be idle at installation and stays reserved until the installer starts. An ordinary development build reports that updates are unavailable. Packaging generates the NSIS installer, its signature and the release manifest.

Saved workflows use the existing encrypted routine storage. A workflow created by the editor carries a fixed header and one to eight consecutive numbered lines. The core validates the entire sequence before dispatch, runs each step as a separate Ollama agent loop inside one cancellable task, and disables cloud fallback for that task. Scheduled workflows retain the Observation policy. Step events carry the ordinal and outcome. A line prefixed with `[si échec]` runs after a failed or unverified action in the preceding executed step. A refusal or expired approval stops the sequence. Manual resume prepares a new workflow from the selected step. Later steps do not inherit tool references or conversation history from earlier steps.
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
- Migration 003 adds `grants` (temporary permissions, see "Temporary grants"); versions 1 and 2 migrate in place.
- Migration 001 uses SQLite `user_version` and creates sessions, events and the tool dispatch reservations. Migration 002 adds `settings` (policy profile) and `tasks` (one row per chat task, see "Tasks and recovery"); a version 1 database migrates in place. Migrations, task recovery and initial session creation run in one transaction. The core refuses schemas newer than it supports. Migration 007 adds `models` (model manifests, see below). Before migrating an existing store (version above 0 and below the current one) the core copies it with `VACUUM INTO` to `backups/aiwin11-<UTC stamp>-before-migration.sqlite3` next to the store, and refuses to start if that copy fails.

Backups (`backup.rs`): `backup_data` (journaled `data.backup.created` `manual`), `list_backups` → `{ backups: [{ name, bytes }], pendingRestore }`, newest first, at most 5 kept; names `aiwin11-YYYYMMDD-HHMMSSmmm-<manual|before-migration|before-restore>.sqlite3`, anything else ignored. `schedule_restore { name }` checks the backup (read-only open, `PRAGMA integrity_check` = ok, `user_version` from 1 to the current one) and writes `restore.pending`; `cancel_restore` removes it (`data.restore.scheduled|cancelled`). At the next launch, before the store opens, the request is consumed, the backup checked again, the current store saved as `before-restore`, then copied over; the result is journaled `data.restore.applied` or `data.restore.failed` with its code (`BACKUP_INVALID`, `BACKUP_FAILED`). Memory content stays DPAPI-encrypted in backups.

Model manifests (R3, migration 007, table `models`: `id` "ollama:<name>", `provider`, `name`, `tool_calling` `unverified|not_advertised|verified|failed`, `vision` `advertised|not_advertised`, `probe_ms`, `probed_at`, `updated_at`; migration 008 adds `context_length` (tokens, from `<architecture>.context_length` in `/api/show`), `parameter_size` and `quantization` (Ollama labels, kept only if at most 32 bytes of letters, digits and `._-`) and `size_bytes` (size on disk from `/api/tags`); migration 009 adds `tokens_per_second` (from `eval_count`/`eval_duration` of the probe reply, only with at least 4 generated tokens; a rough figure since the probe is short) and `vram_bytes` (`size_vram` from `/api/ps` right after the probe); public model identifiers, codes, numbers and timestamps only). `probe_local_model { model }` now records its result (`model.manifest.updated` with the status code); a routed candidate that passes the probe is recorded `verified` (vision, limits, size, speed and VRAM kept). `get_model_manifests` returns up to 64 manifests, most recently probed first. Routing strategies other than `manual` never pick a model whose manifest is `failed` or `not_advertised`; probing it again from the Models page updates its status.
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

The Inspector can list the current desktop displays through the broker's desktop-only `ListDisplays` operation. The Windows bridge uses a per-monitor DPI-aware thread context and returns up to 32 rectangles in virtual desktop pixels, with one primary monitor. The audit records the outcome code.

`CaptureDisplay` accepts one complete display descriptor from that list. The desktop-only operation checks that the descriptor still matches an active display before and after copying its pixels. It returns the same bounded JPEG preview as foreground-window capture. The image stays in memory. The journal records the outcome without the captured pixels.

The Inspector can submit the captured JPEG to `transcribe_capture`. The broker restricts the v1 `TranscribeCapture` operation to the desktop role. The existing Ollama image path requires a local vision model and disabled cloud features. The model receives a fixed transcription instruction with no tools. The text is rendered in the WebView and the audit records only the outcome.

The Inspector page invokes `capture_foreground` after the same countdown. The broker authorizes the v1 `CaptureForeground` operation for the desktop inspector role. The bridge requests rendering of the selected HWND with Win32 PrintWindow. It rejects windows with display-affinity restrictions and discards the image if the foreground window changes during the operation.

`WindowCapture` carries a JPEG of at most 256 KiB with dimensions capped at 1024 px per side. The Rust host drains the pipe during execution and caps the full capture reply at 257 KiB. The image stays in memory and is displayed in the calling WebView. SQLite receives the decision and outcome codes.

The preview offers explicit analysis with an installed Ollama vision model. The broker authorizes `AnalyzeCapture` for the desktop inspector role. The gateway checks the model's local metadata and requires `/api/status` to confirm that Ollama cloud features are disabled immediately before sending the JPEG to loopback. The description is rendered as plain text. The journal records authorization and the outcome code.

## Controlled Character Map input

The Inspector page submits `enter_character_map_digit` after explicit user action and a three-second countdown. The broker authorizes the v1 `CharacterMapDigit` operation for the desktop inspector role and requires one decimal digit. The bridge checks the foreground process path against the system copy of Character Map, then uses UI Automation to identify visible, editable control 104 and validate its native handle. It checks the foreground window and deadline immediately before sending a bounded WM_SETTEXT to that control, then verifies the value with WM_GETTEXT. The field value stays inside the bridge. The journal holds decision and outcome codes.

## Grouped plan: organize_files

`organize_files { moves: [{ source, destination }] }` (1 to 20, flat `.txt` names, all sources and destinations distinct and disjoint) previews every rename first (readable source, free destination); one failure rejects the whole plan before any approval. The approval (`organizeFiles`) shows the complete plan and covers only it. Then each rename runs through the broker (`FileMove`, one command each) and the plan stops at the first failure (reported `partial`). Matrix: Observation deny, otherwise ask (never automatic).

## Agent workspace folder

`set_workspace { path }` stores `files.workspace` (journaled `settings.workspace.changed`, `user_folder` or `dedicated`); an empty path restores the dedicated folder. `LocalFiles::validate_workspace` canonicalizes the path (links resolved) and requires an existing directory on a local disk, strictly inside the user's profile and outside `AppData`, otherwise `WORKSPACE_NOT_ALLOWED` / `WORKSPACE_NOT_FOUND`. It is applied at the next start (and re-validated then; an invalid folder falls back to the dedicated one). The same file rules apply (flat `.txt` names, handle checks, no replacement, trash in AiWin11's data folder). The settings table may therefore hold this folder path.

## Local text preview and copy

The desktop and local agent share the same flat ASCII filename rules for `.txt`, `.md` and `.csv`. All three formats are treated as UTF-8 text. Preview renders plain text and copy preserves the source bytes.

The main desktop window can call `move_local_text`, `trash_local_text`, `list_trash` and `restore_local_text`. Restore accepts the canonical UUID of a trash entry and its flat text filename. It opens the source and destination directories by handle and restores the original filename into the active workspace without replacement. The broker restricts listing and restoring trash to the desktop role. Audit events contain outcome codes.

The Files page submits `preview_local_text` with one file name. The broker authorizes the v1 `PreviewLocalText` operation for the desktop inspector role. The core resolves the candidate under the active workspace and validates the opened handle's final path, file attributes and link count before reading. It accepts flat ASCII `.txt`, `.md` and `.csv` names and UTF-8 content up to 64 KiB. The result is displayed as plain text in the requesting window. SQLite records the operation and outcome codes. The filename and text stay out of the audit event and model gateways.

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
| `search_text_files` | `query` (2–64 chars) | searchFiles | FileRead | yes |
| `read_text_file` | `name` | readFile | FileRead | yes |
| `copy_text_file` | `source`, `destination` | copyFile | FileCopy | yes |
| `move_text_file` | `source`, `destination` | moveFile | FileMove | yes |
| `trash_text_file` | `name` | trashFile | FileTrash | yes |
| `inspect_window` | — | inspectWindow | WindowLookup, then WindowInspect | yes |
| `describe_window` | — | describeWindow | WindowLookup, then WindowCapture | yes, and only with a chosen vision model |
| `type_in_notepad` | `text` (1–2000 chars) | typeText | TypeText | yes |

File names are flat ASCII names in `.txt`, `.md` or `.csv`. The stem accepts letters, digits, spaces, `-` and `_`, excluding reserved device names. The full name is at most 128 bytes. Source and destination must differ ignoring case. No tool takes a path, a window handle, a process ID or a command line: the host decides every target. Local-only tools are not offered to cloud models and are denied by the policy engine if proposed anyway.

## Policy engine

`policy::evaluate(settings, action, context)` returns `Allow`, `Ask`, `Deny` or `Simulate`, with the rule that produced it. It starts from a matrix (action × profile) and applies invariants that no profile can relax:

| Action | Observation | Assisted (default) | Limited | Advanced |
|---|---|---|---|---|
| list / read files | Allow | Allow | Allow | Allow |
| inspect / describe a window | Ask | Ask | Ask | Ask |
| launch Notepad, copy, type in Notepad | Deny | Ask | Allow | Allow |
| rename | Deny | Ask | Ask | Allow |
| trash | Deny | Ask | Ask | Ask |

Invariants, in order: local-only tools are denied to cloud models (`local_only`); a profile denial stays a denial; with **simulation mode** every remaining side effect becomes a dry run (`simulation_mode`); screen reading always asks (`screen_data`); reading private data such as the clipboard always asks (`private_data`); critical actions (trash, clipboard write) always ask (`critical_action`); a temporary grant may then turn a remaining `Ask` into `Allow` (`temporary_grant`). Unknown or unreadable settings resolve to Observation.

Settings live in the `settings` table (`policy.autonomy`, `policy.simulate`), are changed through `set_policy_settings` (main window only, journaled as `settings.policy.changed`) and are read once per task (`policy.profile` event): a running task keeps the settings it started with. `get_policy_overview` returns the settings and the decision table computed by the core.

Model preferences (`model.provider` = `ollama`|`openai`, `model.local`, `model.cloud`, `model.vision`) are read with `get_model_preferences` and written with `set_model_preferences` (`ModelPreferences { provider, localModel, cloudModel, visionModel }`; identifiers ≤ 256 characters without control characters, otherwise `SETTINGS_INVALID`), journaled as `settings.models.changed` with the provider code only. The cloud approval and the API key are never stored there. Invalid stored values resolve to the local provider with nothing selected.

### Specialized agents

`TaskRequest.agent` (`general` default, `office`, `windowsAdmin`, `readOnly`) selects a specialized agent. `AgentProfile::allows` lists its tools: Office (files, organize, Notepad, clipboard, memory, launch), Windows administration (system status, processes, services, diagnostics, admin operations, windows, inspection, memory reads), read-only (every tool without side effect). The catalog sent to the model only contains the agent's tools, and `policy::evaluate` denies any other first (rule `agent_scope`), before every other rule. Journaled `agent.profile`. Scheduled runs use the general agent with the Observation profile.

Delegation: only the general agent has `delegate { agent: office|windows_admin|read_only, request }` (allowed; `agent_scope` denies it to every specialized agent, so delegation is one level deep). The sub-request runs in the same task with the chosen agent's tool set, the same provider, models and policy settings; each of its actions is gated and approved on its own and appears in the task's report. Only its reply returns to the general agent, framed as data. Journaled `agent.delegated`.

### Tool manifests

`manifest::MANIFESTS` holds one reviewed `ToolManifest` per agent action: `tool` (catalog and agent registry name), `version`, `risk` (`low`, `medium`, `high`, `critical`), `sideEffect`, `readsScreen`, `localOnly`, `dryRun`, `reversibility` (`noEffect`, `reversible`, `recoverable`, `irreversible`), `timeoutMs` and the broker `capabilities` it needs. The policy invariants read these facts: `local_only` from `localOnly`, `screen_data` from `readsScreen`, `critical_action` from `risk = critical`, simulation from `sideEffect`. Tests check that every action has a manifest, that the manifests match the model tool catalog and `agent.py`, and consistency rules (a side effect is simulable and says how it is undone; screen readers are local only; an irreversible tool is critical). `get_policy_overview` returns them as `manifests`; the Security page shows them. A manifest never grants anything.

### Temporary grants

Migration 003 adds `grants` (`id`, `action` code, `created_at`, `expires_at`, `revoked_at`). `create_grant { action, minutes }` (1 to 480 minutes) lets one action run without asking; it is refused with `GRANT_NOT_ALLOWED` unless `policy::grantable(action)` (a side effect that is neither critical nor a screen read). `revoke_grant { id }` ends it at once (`GRANT_NOT_ACTIVE` if unknown, expired or already revoked). `get_grants` returns `{ active, grantable }`. Journaled as `grant.created` (`<action>:<N>min`) and `grant.revoked` (`<action>`).

The host reads grants at each evaluation, just before execution (`ActionContext.temporary_grant`), so revocation and expiry apply to the very next action. In `policy::evaluate` a grant is the last rule: it only turns a profile `Ask` into `Allow` (rule `temporary_grant`), after the local-only, profile denial, simulation, screen-data and critical-action rules, which it never relaxes.

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

- `list_text_files` returns up to 100 sorted names of regular `.txt`, `.md` and `.csv` files. `search_text_files` matches the query, case-insensitively, in the names and contents of those files: at most 20 files, each with up to three trimmed matching lines of 160 characters; unreadable files are skipped; the query is not journaled (`file.search.*`). `read_text_file` passes up to 16 000 characters of text to the model after removing the BOM.
- `copy_text_file`: staging file, flush, same-folder rename with replacement disabled, cleanup on failure.
- `move_text_file`: rename of the handle-opened file inside the folder, replacement disabled.
- `trash_text_file`: the file is moved, by handle-anchored rename, into a new subfolder of AiWin11's own `trash` folder (next to `files`), keeping its name. Nothing is deleted; it can be restored by moving it back. A failed trash leaves no empty folder.

## Agent window tools

`inspect_window` and `describe_window` take no argument: the model cannot choose the window.

1. **Locate.** v1 `LocateWindow { exclude_process_id: <host pid> }`: the bridge walks the top-level z-order and returns the first window that is visible, not minimized, unowned, not cloaked, not tool/topmost/no-activate, not a shell window, with a title, and not owned by the host or the bridge: `WindowInfo { window, process_id, title ≤ 120 chars, process_name ≤ 64 chars }`. The title is shown to the user in the approval; the model sees it only after approval.
2. **Inspect.** v1 `InspectWindow { window, process_id }`: the bridge checks before and after the walk that the handle still exists and belongs to that process, then returns up to 80 nodes, 8 levels deep: control type, enabled, focus and accessible name (single line, ≤ 80 chars, always empty for password fields). No pixels. Audit `window.inspect.*`.
3. **Describe.** v1 `CaptureWindow { window, process_id }`: the bridge renders exactly that window with PrintWindow (it need not be in the foreground), checks the process before and after, refuses display-affinity windows, and returns a JPEG of at most 1024 px per side and 256 KiB. The host passes it to the chosen local Ollama vision model, after checking again that the model is local and that Ollama's cloud features are disabled. Only the text description reaches the agent; the image is kept in memory for the call and never stored or sent elsewhere. Audit `window.capture.*`, `vision.describe.*`.

The desktop Inspector page is unchanged: it still reads the foreground window's structure only, without names, after its countdown.

## Agent process listing

`list_processes` (no argument, local models only) is a read allowed in every profile. The host sends `ListProcesses { limit: 15 }` (the broker accepts 1 to 25, agent peer with `Capability::ProcessList` only). The bridge enumerates processes with `System.Diagnostics.Process` as the standard user, skips the Idle process, and returns `ProcessList { processes, total }` sorted by working set, each `ProcessEntry { process_id, name (≤ 64 characters), working_set_bytes, cpu_time_ms (0 when protected), has_window }`; reply at most 8 KiB. The host rejects unsorted, oversized or malformed lists. The agent receives names, IDs, memory in MB and CPU seconds framed as untrusted data; journaled as `process.list.completed|failed`. Nothing is ever stopped or modified.

## Allowed applications

`launch_application { application: notepad|charmap|paint }` keeps one policy action. Notepad needs `Capability::LaunchNotepad`; Character Map and Paint need `Capability::LaunchAllowedApp` (`APPLICATION_CHARACTER_MAP`, `APPLICATION_PAINT`). The bridge starts the fixed System32 executable (`charmap.exe`, `mspaint.exe`) without arguments nor handle inheritance and verifies a new window of the expected process (`charmap`, `mspaint`) for up to 6 s. Only a Notepad window launched this way can then receive `type_in_notepad`.

## Model routing (R3)

Strategies: `manual` (default: the chosen model, nothing else) and `local_first` (`routing.cloud_fallback`, `get_cloud_fallback` / `set_cloud_fallback`, journaled `settings.routing.changed`). With `local_first`, `TaskRequest.cloudFallbackModel` names the OpenAI model; the window sends it only when the user ticked the cloud approval for this request. If Ollama is unreachable on the **first** step (no local observation exists yet), the core re-checks the setting and private mode, journals `model.provider.unavailable` then `model.fallback.cloud`, and continues with OpenAI, local-only tools removed (the policy engine sees a cloud model). Otherwise the task fails as before. Cloud budget: `routing.cloud_token_budget` (1 000 to 2 000 000 tokens, default 50 000; `get_cloud_budget` / `set_cloud_budget`, `settings.budget.changed`). OpenAI requests ask for `stream_options.include_usage`; the tokens reported (or a conservative estimate) are added per task and journaled as `model.usage` (`tokens:N`). Before each cloud call the budget is checked; once spent the task fails with `CLOUD_BUDGET_EXCEEDED` (`model.budget.exceeded`). No price is hard-coded. Decision records: `policy.evaluated` codes are `effect:action:rule:risk` (risk from the tool manifest: `low|medium|high|critical`). Several folders (`folders.rs`): besides the primary folder (dedicated or chosen), up to 4 extra folders (`get_extra_folders` → `{ saved, aliases }`, `set_extra_folders { paths }`, each checked by `LocalFiles::validate_workspace`; setting `files.extra_folders`, journaled `settings.folders.changed` `count:N`; applied at the next start). The agent names their files `alias/name.txt`, the alias being the folder name lowercased with accents folded and other characters turned into `-`, made unique. Unknown alias: `FOLDER_UNKNOWN`; a copy or rename never crosses folders (`FILE_FOLDER_MISMATCH`), a destination without alias stays in the source folder. Each folder is a `LocalFiles` store with the same checks, grants and audit. Trashed files from an extra folder are restored into the primary folder by the Files page.

Protected files (per-resource rule): `get_protected_files` / `set_protected_files { patterns }` (at most 20 file name patterns of 1 to 64 characters, `*` and `?` wildcards, case-insensitive, no `\ / : < > " |`; setting `policy.protected_files`, journaled `settings.policy.protected.changed` `count:N`, never the patterns). Checked by the file service inside each audited agent operation: protected files are left out of `list_text_files` and `search_text_files`; reading, copying (source or destination), renaming (either name) and trashing fail with `FILE_PROTECTED` (`file.*.failed`). The user's own Files page is not restricted. Blocked applications (per-resource rule): `get_blocked_applications` / `set_blocked_applications { applications }` (codes of the closed list `notepad|charmap|paint`; setting `policy.blocked_applications`, journaled `settings.policy.applications.changed` with the codes or `none`). Protected windows: `get_protected_windows` / `set_protected_windows { patterns }` (same pattern rules as files; setting `policy.protected_windows`, journaled `settings.policy.windows.changed` `count:N`). Each pattern is matched against the window title and the process name. Matching windows are left out of `list_windows`; locating, inspecting, capturing, arranging or clicking in one fails with `WINDOW_PROTECTED` before any command reaches the bridge (`policy.denied` `window_protected`). The broker refuses a blocked application to the agent at dispatch (`policy.denied` `resource_blocked`); the task also refuses it before asking any approval (`policy.resource.denied` with the code). Startup (T-001): after its first paint the window calls `ui_ready`; the first call of a launch journals `ui.ready` (source `desktop`, code `ms:N`, milliseconds since the process started) and returns N; `get_runtime_status` includes `uiReadyMs` (null before). Local fallback model (T-009): `get_local_fallback_model` (name or null) / `set_local_fallback_model { model }` (empty clears; setting `routing.local_fallback_model`, journaled `settings.routing.fallback.changed` `local|none`). If the local model fails with `MODEL_UNAVAILABLE_OR_INVALID` before any step ran, the core switches once to that model, provided it differs, is not marked unusable in the manifests and passes the tool-call probe: `model.fallback.local` (`probe_verified`), then `modelSelected` with reason `local_fallback`. Never to a cloud model; a provider outage (`MODEL_PROVIDER_UNAVAILABLE`) is not covered by it. Before each cloud call the core journals `model.cloud.request` (code: vendor, `openai` or `deepseek`) and sends it as a stage, so the window can show that data is leaving the PC. Cost cap: `get_cloud_cost` / `set_cloud_cost { cost: { priceMilliPerMillion, maxMilliPerTask } }` (thousandths of the billing currency, shown in euros, 0 = not set, at most 1 000 000; settings `routing.cloud_price_milli` and `routing.cloud_max_cost_milli`, journaled `settings.cost.changed` `price:N+max:N`). With both set, the estimated cost of the tokens counted so far (rounded up) is checked before each cloud call; once it reaches the cap the task fails with `CLOUD_COST_EXCEEDED` (`model.cost.exceeded`, `milli:N`). The token budget still applies. The rate older versions kept in browser storage is moved to these settings once. `get_model_resources` returns Ollama's loaded models (`/api/ps`: name, size, size_vram) with `placement` (`gpu` all in GPU memory, `partial` partly offloaded to the CPU, `cpu` no GPU memory used, `unknown` inconsistent figures) and `gpu_percent` (rounded down; 99 % or more but not all is still `partial`). The Models page lists each task type with its model; voice, image and video show "not yet available" (no engine).

## Private mode and desktop surfaces

`privacy.strict` (`get_privacy` / `set_privacy`, journaled `settings.privacy.changed` `strict|hybrid`): when on, an OpenAI task fails with `PRIVATE_MODE` before anything is sent (`model.cloud.denied`, code `private_mode`); an unreadable setting counts as on. The tray icon (menu: open, stop all tasks, quit) and the global shortcut Ctrl+Alt+Space only show the main window or stop tasks; they never start an action. Every Tauri command is declared in `build.rs` and allowed for the main window in `capabilities/`.

## Packaging

`scripts/package.ps1` publishes the bridge and the elevated helper (Release, framework-dependent) and copies the agent to `target/companions`, then runs `tauri build --config src-tauri/tauri.bundle.json` (NSIS, `installMode: currentUser`, companions installed next to `AiWin11.exe`). The agent's interpreter is `AIWIN11_PYTHON` when set, otherwise the first `python.exe` on the PATH outside `WindowsApps`. The release build publishes the helper as a single file and pins it: `package.ps1` sets `AIWIN11_ADMIN_SHA256`, and before each UAC launch the core opens `AiWin11.Admin.exe` with read sharing only (no write, delete or rename until the helper exits) and checks its SHA-256 (`ADMIN_HELPER_TAMPERED` otherwise). Development builds only take the lock. Remaining limit: a per-user install directory is writable by the user, so a program running as the user could replace `AiWin11.Admin.exe` before an approved UAC prompt; a per-machine install for the helper (Program Files) and code signing are planned (R9).

## Floating mode

`set_floating { enabled }` (main window only) makes the main window always on top, 440×620 logical pixels, in the bottom-right corner of its monitor's work area, or restores 1060×700 centered. It changes nothing else.

## Elevated helper (option B)

`admin_service { service, operation: start|stop|restart|automatic|manual|disabled }` and `admin_clean_temp {}` are critical (always ask, never grantable, denied in Observation, local models only). They run only if the user enabled `admin.helper.enabled` in Security (`get_admin_helper` / `set_admin_helper`, journaled `settings.admin.changed`); otherwise `ADMIN_HELPER_DISABLED`. Service names must match `[A-Za-z0-9_.-]{1,64}`; protected services (security, updates, services Windows needs) are refused before any prompt (`ADMIN_REFUSED`), and again by the helper. After the user's approval the broker authorizes `AdminService` / `AdminCleanTemp` (`Capability::AdminOperation`), then the core starts `AiWin11.Admin.exe` next to its executable with `ShellExecuteExW("runas")` and fixed arguments (`service <action> <name>` or `clean-temp`), waits up to 90 s and maps its exit code: 0 done, 2 `ADMIN_INVALID`, 3 `ADMIN_REFUSED`, 4 `ADMIN_FAILED`, 5 `ADMIN_NOT_ELEVATED`; a declined UAC prompt is `ADMIN_UAC_DECLINED`, no exit in time `ADMIN_TIMEOUT`. The helper checks it is elevated, re-validates its arguments against its own closed list, performs one operation and exits; it reads no file, opens no listener and runs no shell. Cleaning deletes files older than one day under `%SystemRoot%\Temp`, skipping reparse points and files in use. Events `admin.operation.completed|failed`.

## Approved diagnostics (ShellService)

`run_diagnostic { diagnostic: network|wifi|time }` (read, allowed, local models only) runs exactly one fixed System32 executable with fixed arguments: `ipconfig.exe /all`, `netsh.exe wlan show interfaces`, `w32tm.exe /query /status`. No shell, no argument from the model, 10 s at most, output decoded from the OEM code page, control characters removed, at most 12 000 characters, framed as untrusted data. Broker: `RunDiagnostic` with `Capability::Diagnostic`, unspecified values refused. Events `diagnostic.run.completed|failed`.

## Agent service listing

`list_services` (no argument, local models only) is a read allowed in every profile. The host sends `ListServices { limit: 20 }` (broker: 1 to 25, `Capability::ServiceList`). The bridge uses `System.ServiceProcess.ServiceController` (NuGet `System.ServiceProcess.ServiceController` 10.0.2) as the standard user and returns `ServiceList { services, total, running, stopped_automatic }`: automatic services that are stopped first, then running services, by name; each `ServiceEntry { name (≤ 64), display_name (≤ 120), running, start_mode }`. The host rejects incoherent counts and malformed entries. Nothing is ever started, stopped or reconfigured (that will need the elevated helper, R2 option B). Events `service.list.completed|failed`.

## Agent system status

`describe_screen {}` (local models only, local vision model required, always asks: screen data; action `describeScreen`, `Capability::ScreenCapture`): when protected windows are configured, the host first reads the window list itself and refuses with `WINDOW_PROTECTED` (`policy.denied` `window_protected`) if one of them is visible (not minimized). It then lists the displays and captures the primary one (`ListDisplays`, `CaptureDisplay`, now allowed to the agent peer with `ScreenCapture`), at most 1024 px per side, and has the local vision model describe it; the image is dropped right after. Events `displays.*`, `display.capture.*`, `vision.describe.*`.

`list_applications {}` (local models only, allowed in every profile, action `listApplications`, `Capability::ProcessList`): the bridge answers `ApplicationList` with exactly one `ApplicationStatus { application, installed, processes, windows }` per application of the closed list (Notepad, Character Map, Paint); the host refuses duplicates, missing entries and more windows than processes. The observation adds whether the user blocked each one. Events `application.list.completed|failed`.

`inspect_process { process_id }` (local models only, allowed in every profile like `list_processes`, action `inspectProcess`, `Capability::ProcessList`): the PID must come from this task's last `list_processes`, otherwise `PROCESS_NOT_LISTED`; if the ID now names another program, `PROCESS_CHANGED`. The host sends `InspectProcess { process_id }`; the bridge answers `ProcessDetails { process_id, name, parent_process_id, start_time_unix_ms, working_set_bytes, private_bytes, cpu_time_ms, threads, handles, has_window, executable_path (≤ 260), description, company (≤ 120) }`, each field left empty when Windows refuses it (protected or elevated process). Never the command line or window titles. The host checks the asked ID and short printable texts; the model receives it framed as untrusted data. Events `process.inspect.completed|failed`.

`get_system_status` (no argument, local models only) is a read allowed in every profile. The host sends `GetSystemStatus {}` (agent peer with `Capability::SystemStatus`). The bridge answers `SystemStatus { memory_total_bytes, memory_available_bytes, cpu_load_percent (sampled over 250 ms with GetSystemTimes), logical_processors, uptime_seconds, drives }`, with at most 8 fixed drives `DriveStatus { name, total_bytes, free_bytes }` and at most 4 display adapters `GpuInfo { name, driver_version, memory_bytes }` read with WMI `Win32_VideoController` (NuGet `System.Management` 10.0.2; memory as declared by Windows, sometimes capped at 4 GiB). The host rejects incoherent figures (available memory above total, load above 100, free space above size). It also carries `SystemSettings` (current user, read only): `apps_light_theme`, `system_light_theme`, `transparency` (optional, from HKCU `Themes\Personalize`), `ui_language` (BCP 47, at most 16 characters), `time_zone` (Windows identifier, at most 64), `power_source` (`UNKNOWN|AC|BATTERY`, `GetSystemPowerStatus`), optional `battery_percent` (0 to 100, absent without a battery) and `battery_saver`. Malformed settings are dropped without rejecting the metrics; the agent is told they are unavailable. The agent receives rounded figures; journaled as `system.status.completed|failed`.

## Agent control click (UI Automation Invoke)

`inspect_window` now numbers each control `[n]` (1-based, walk order) and the task keeps the result. `click_control { control: n }` targets only a control of that result; the approval shows the window, the application and the control. Critical and irreversible in its manifest (a button can send, delete or pay): always asks. The broker accepts `InvokeControl { window, process_id, index 1..80, control_type, name }` with `Capability::ControlInvoke`; the bridge re-walks the same window, checks that control `index` still has the same type and name (`control_changed` otherwise), then uses the Invoke, Toggle or SelectionItem pattern (`not_invokable` if none). `set_control_value { control: n, text }` (1 to 2000 characters) sends the same command with `value` set: the bridge refuses password and read-only fields and uses ValuePattern.SetValue, then reads the value back. Critical, always asks with the full text shown. Events `control.invoke.completed|failed`.

`find_control { query }` (2 to 64 characters, local models only, allowed in every profile, action `findControl`): searches the names and type names of the controls of the window this task already inspected (case-insensitive, at most 20 matches) and returns their numbers [n] for `click_control` / `set_control_value`, framed as untrusted data. Nothing new is read from the screen or sent to the bridge; without a prior `inspect_window` it says so. Journaled `control.find.completed` (`matches:N`).

## Agent window management

`list_windows` (no argument) reads window titles, so the manifest marks it `readsScreen`: it asks in every profile (`screen_data`) and is never grantable. The bridge walks the Z order with the same filters as `LocateWindow` (visible, unowned, not cloaked, not topmost/tool windows, not the host nor the bridge, a title), minimized windows included, and returns `WindowList { windows }` (at most 20 `WindowInfo` with `minimized`). The host validates the list and keeps it in the task; the model sees numbered lines framed as untrusted data.

`arrange_window { window: 1..20, arrangement: focus|minimize|restore|maximize }` can only target a window of the same task's last list: there is no way to pass a handle or a process ID. Without a list the host answers `WINDOW_NOT_LISTED`. The approval shows the window title, application and arrangement. Matrix: Observation deny, Assisted ask, Limited and Advanced allow (reversible, simulable, grantable). The bridge re-checks that the handle still belongs to the listed process (`window_unavailable` otherwise), applies `ShowWindow`/`SetForegroundWindow` and verifies the state for up to one second (`Success`, or `Partial` with `state_unverified`, e.g. when Windows refuses a foreground change). A window is never closed. Events `window.list.completed|failed`, `window.arrange.completed|partial|failed`.

Resizing (`arrange_window` arrangements `snap_left`, `snap_right`, `center`; proto `ARRANGEMENT_SNAP_LEFT|SNAP_RIGHT|CENTER`): the bridge computes the rectangle from the work area of the window's own screen (left or right half, or two thirds centered), restores a minimized or maximized window first, then checks the result within 16 px; a window that keeps another size (fixed-size dialogs) is reported `state_unverified`. No free coordinates are accepted, so a window cannot be moved off-screen. Same action, approval and audit as the other arrangements.

## Agent clipboard tools

`read_clipboard` (no argument) and `write_clipboard { text }` (1 to 2000 characters, no NUL) are offered to local models only. Their manifests make the read `private_data` (asks in every profile, rule `private_data`, never grantable) and the write `critical` and `irreversible` (the previous content is lost: asks in every profile, never grantable, denied in Observation). The approval for a write shows the full text. The bridge uses the Win32 clipboard API (Unicode text only, retried until the deadline when another application holds the clipboard): `ReadClipboard {}` returns `ClipboardContent { text (≤ 4000 characters), truncated, has_text }`; `WriteClipboard { text }` empties the clipboard, sets the text, reads it back and answers `Success` or `Partial` (`text_unverified`). Capabilities `ClipboardRead` and `ClipboardWrite` are separate. The text read is framed as untrusted data for the model and never journaled; events `clipboard.read.completed|failed`, `clipboard.write.completed|partial|failed`.

## Agent text input in Notepad

`type_in_notepad(text)` writes into the Notepad window **this task launched and verified**: the host keeps that process ID from the launch result; without one, nothing is asked or written and the model is told to launch Notepad first. v1 `TypeInNotepad { process_id, text }`: the bridge checks the process is Notepad (package family or system path), finds its visible editor (`RichEditD2DPT` or `Edit`), refuses a non-empty document (`document_not_empty`, so restored tabs or user text are never replaced), writes with `WM_SETTEXT` (no simulated keystrokes) and reads the text back. Outcomes: verified (`done`), sent but not verified (`partial`, `text_unverified`), or refused. The approval shows the full text. Audit `input.notepad.*`.

## Tasks and recovery

Each chat task has a row in `tasks` (`id`, `session_id`, `state`, `outcome`, timestamps), updated at each step: `created`, `planning`, `awaiting_approval`, `dispatching`, `paused`, `recovering`, then `completed` (outcome `ok`), `failed` (error code) or `cancelled`. Final states never change.

**Pause.** `pause_task` (main window only) asks the running task to pause at its next safe point: before its next model step, never in the middle of an action or an approval. The task then records `task.paused`, waits, and records `task.resumed` when `resume_task` is called. A paused task can be cancelled or stopped. Paused time does not count against the 15-minute runtime limit; after 30 minutes of pause the task stops with `PAUSE_EXPIRED` (`task.limit.reached`, code `pause`).

**Recover.** The read-only system tools that never ask the user (`list_processes`, `get_system_status`, `list_windows`, `list_services`) are retried **once**, after one second, when the bridge call fails with `WINDOW_UNAVAILABLE` (bridge starting, unreachable or empty reply): state `recovering`, events `task.recovering` (`retry_once`) and `task.recovery.finished` (`recovered` or `failed`). Denials and other errors are never retried, and no side effect or approval is ever repeated.

At startup, tasks still in a non-final state were interrupted by a crash or a forced exit. They are closed as `interrupted`, keeping their last state as outcome, and journaled as `task.recovered` in the new session. Nothing is re-run, so an action that may have happened is never repeated; a task interrupted while `dispatching` is flagged in the Activity page so the user can check its result. Within a session, the broker's idempotency reservations already prevent repeated dispatch.

`get_tasks` (main window only) returns the 30 most recent tasks across sessions. `get_snapshot` returns up to 100 events of the current session with their task ID and code.

### Lifecycle events and global stop

Each task journals `task.created`, then `task.started` (code: provider, `ollama` or `openai`), `task.planning`, one `task.progress` per agent step (code `step:N/16`, never content; a task stops after 16 steps, `AGENT_STEP_LIMIT`, or 15 minutes, `AGENT_TIME_LIMIT` with `task.limit.reached`), and ends with `task.completed`, `task.failed` (error code) or `task.cancelled`. When the provider cannot be reached the task journals `model.provider.unavailable` (code: provider) and fails with `MODEL_UNAVAILABLE_OR_INVALID` (Ollama) or `CLOUD_REQUEST_FAILED` (OpenAI). There is never an automatic fallback to another provider.

`stop_all_tasks` ("Stoppe toutes les tâches", a button always visible in the side bar) returns `idle` (nothing running), `stopped` (the task was planning or awaiting approval and is cancelled now) or `requested` (an action is executing: it is not interrupted midway, and the task stops right after it without any further step). It is journaled as `tasks.stop.requested` with that code; a requested stop is confirmed by `task.cancelled` when the task ends, which the window shows as "Arrêt confirmé".

### History retention, erasure and export

Past sessions older than the retention period (`history.retention_days` in `settings`, 1 to 3650 days, default 90) are deleted at startup, in the startup transaction, with their events, tasks and dispatch reservations; the current session is never purged. A purge is journaled as `history.purged` (code `sessions:N`). `get_history_settings` returns `{ retentionDays }`; `set_retention_days` (out of range: `SETTINGS_INVALID`) stores and applies it at once, journaled as `settings.retention.changed` (`days:N`). `clear_history` deletes all past sessions (refused with `TASK_BUSY` while a task runs) and journals `history.cleared` (`sessions:N`). `export_audit` writes every kept event, oldest first, as `{ "format": "aiwin11.audit.v1", "events": [{ sequence, id, sessionId, taskId, kind, source, code, timestamp }] }` to `%LOCALAPPDATA%\app.aiwin11.desktop\exports\aiwin11-audit-<UTC>.json`, returns the path and journals `history.exported` (`entries:N`). Events only ever hold codes, so the export holds no user content.

## Streaming and progress (AW-011)

Both providers are called with `stream: true`. The gateway folds Ollama's NDJSON chunks or OpenAI's SSE `data:` events into one proposal, with the same bounds (2 MiB body, 4000 reply characters, one tool call). Every Ollama chunk is checked for remote metadata. Tool-call fragments are assembled silently and never forwarded; OpenAI call IDs are kept for the next turn.

`run_task`'s `progress` channel carries `TaskUpdate` messages:

- `{ "type": "stage", "stage": "<event kind>" }` after persisted lifecycle events (`task.created`, `task.planning`, `agent.tool.requested`, `agent.observation`, `policy.simulated`, `policy.approval.granted|declined|expired`, `agent.reply`);
- `{ "type": "approvalRequired", ... }` when an action awaits the user's decision;
- `{ "type": "delta", "text": "..." }` for each streamed text fragment.

Deltas are unvalidated model output, shown as a draft and never journaled; the window clears the draft at each tool step and when the task ends. After each chunk the host checks the task phase: a cancellation requested while the model streams stops reading, which closes the provider connection. A provider that sends no chunk at all can still hold the task until its timeout (90 s local, 45 s OpenAI).

Adding a tool requires a catalog entry, an agent registry entry, a host mapping, a policy action and matrix row, a Protobuf operation, a broker capability and rule, and bridge or core support.

## Cloud vendors (OpenAI, DeepSeek)

`cloud.vendor` (`openai` default, `deepseek`; `get_cloud_vendor` / `set_cloud_vendor`, journaled `settings.cloud.changed`) selects the Chat Completions endpoint, the request body and the credential: `AiWin11/OpenAI` or `AiWin11/DeepSeek` in Windows Credential Manager (key commands act on the selected vendor). DeepSeek (`https://api.deepseek.com/chat/completions`) is called without OpenAI-only fields (`store`, `parallel_tool_calls`, strict schemas) and with `max_tokens`. Same rules as OpenAI: approval per request, private mode, token budget, launch tool only, nothing local-only is ever sent. The model identifier is typed by the user (none is hard-coded).

## Optional OpenAI provider

The host records the cloud authorization decision (`model.cloud.authorized` / `model.cloud.denied`) before inference. OpenAI receives the selected model ID, the fixed agent instructions, the user's request, the previous steps and a catalog limited to `launch_application`, over HTTPS with `store: false`, only when `cloudApproved` is set for that request. No file content, window data or image is ever sent to it. Its response is converted to the same proposal format as Ollama and validated by the Python agent.

The main window can save, remove and check the presence of the `AiWin11/OpenAI` credential. Rust stores it as a generic entry in Windows Credential Manager for the current user on the local machine. The presence command returns a boolean. The key is read in the native gateway for the OpenAI request and excluded from SQLite, agent input and error messages.

## Local image generation (R7)

`image.endpoint` (`get_image_engine` returns it with an availability probe of `/sdapi/v1/sd-models`; `set_image_engine`; journaled `settings.image.changed`) accepts only `http://127.0.0.1:<port>` or `http://localhost:<port>`, so prompts and images never leave the PC. Agent tool `generate_image { prompt ≤ 1000 }` (local models only; Observation deny, Assisted ask with the prompt shown, Limited/Advanced allow; office and general agents) calls `POST /sdapi/v1/txt2img` (512×512, 20 steps, 180 s at most) after broker authorization (`GenerateImage`, `Capability::ImageGenerate`), checks the PNG signature and size (≤ 16 MiB) and writes a new `image-<uuid>.png` in `%LOCALAPPDATA%\app.aiwin11.desktop\images` (never replacing a file). `open_images_folder` opens it in Explorer. Events `image.generated|failed`.

## MCP extensions (R6)

`mcp.servers` (settings, JSON; `get_mcp_servers` / `set_mcp_servers`, journaled `settings.mcp.changed`) holds at most 5 user-declared stdio servers `{ name [A-Za-z0-9_-]{1,32}, command (absolute path to an existing file), args (≤ 16) }`. No shell: the executable is started directly, with a reduced environment (SystemRoot, PATH, USERPROFILE, APPDATA, LOCALAPPDATA, TEMP, TMP), one process per request: `initialize` (protocol `2025-06-18`), `notifications/initialized`, then one `tools/list` or `tools/call`, 60 s at most, then the process is stopped. Agent tools, local models only: `mcp_list_tools { server }` (asks in Observation and Assisted, allowed in Limited/Advanced) and `mcp_call { server, tool, arguments ≤ 4000 characters }` (critical: always asks, the approval shows the full arguments). Both pass through the broker (`McpRequest`, `Capability::McpServer`); the server's own authorization never replaces this. Results (text content, ≤ 8000 characters; tool lists ≤ 50 entries) are framed as untrusted data. Events `mcp.request.completed|failed`.

## Durable memory (R6)

Migration 004 adds `memory_entries` (`id`, `created_at`, `content`). `content` is DPAPI ciphertext (`CryptProtectData`, current Windows user, entropy `AiWin11.memory.v1`); the plain text never reaches SQLite (tested by scanning the file). Agent tools, local models only: `remember { text }` (1 to 1000 characters; Observation deny, Assisted ask with the full text shown, Limited/Advanced allow; at most 200 notes) and `recall {}` (read, allowed). Both pass through the broker (`RememberNote` / `RecallNotes`, capabilities `MemoryWrite` / `MemoryRead`; the note itself is not in the command). Notes reach the model framed as data. `get_memories`, `delete_memory { id? }` (none = all) serve the **Mémoire** page. Events `memory.saved` (code `note` or `routine`), `memory.deleted` (`entries:N`), no content.

`search_memory { query }` (read, allowed) returns the 5 closest notes: cosine similarity of local embeddings (`/api/embed`, model `memory.embedding_model`, set in the Mémoire page; vectors computed on demand, never stored) or, without a model or on any embedding error, keyword overlap.

Dictation (R7, window only): `dictate` sends `Dictate { max_seconds: 10 }` as the desktop peer (never the agent; the broker refuses it for agents). The bridge uses `System.Speech` (`SpeechRecognitionEngine`, dictation grammar, installed offline recognizers, UI culture first, then French) on the default microphone and returns `DictationResult { status: recognized|silence|no_recognizer|no_microphone, text ≤ 2000, language }`. The text only fills the request field; nothing is stored. Events `dictation.completed|failed`.

Speech (R7, window only): replies can be read aloud with `speechSynthesis`, restricted to voices with `localService === true` (Windows voices on this PC; online voices are never used). Speech recognition is not offered: WebView2's recognizer is an online service.

Saved requests (migration 005, `memory_entries.kind = 'routine'`, same DPAPI encryption): `save_routine { text }`, `get_routines`, deletion with `delete_memory { id }` (clearing all notes keeps routines). Picking one only fills the request field: it never runs without the user sending it.

Scheduling (R8, migration 006: `schedule` "HH:MM" local time, `last_run` local day): `set_routine_schedule { id, schedule? }` (journaled `routine.schedule.changed`). While AiWin11 is open, a scheduler thread checks every 30 s and runs each due request once a day, only when no task is running, with `TaskRequest.scheduled = true`: local provider required (`SCHEDULED_LOCAL_ONLY`), Observation profile forced (`task.scheduled`), so every side effect is refused; anything that would still ask (screen, clipboard, window list) is denied with rule `unattended`, as nobody can approve. The reply (or error code) is stored encrypted as a `report` (50 kept), listed by `get_reports`.

A schedule may include a weekday suffix, such as `07:30@12345` for Monday to Friday. The weekday digits are sorted and unique, from Sunday (`0`) to Saturday (`6`). A time alone still runs daily. Updating the schedule preserves the date of the last execution.

The schedule value `startup` selects the first scheduler cycle after AiWin11 starts and has a configured local model. The same once-per-local-day execution marker applies. Startup routines use the same Observation profile as timed routines.

## Storage invariant: nothing sensitive in SQLite

Every write to `events` passes event kinds, codes and task IDs through `journal_code`: a value that is not a short machine code (ASCII letters, digits and `_.:+/-`, at most 96 bytes) is stored as `REDACTED`. The test `the_schema_only_holds_identifiers_codes_and_timestamps` lists every column of the schema with what it holds; adding a column fails it until the column is reviewed. `free_text_never_reaches_the_database_file` scans the database file for injected free text.
