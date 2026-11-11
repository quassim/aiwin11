# AiWin11 Technical Specification v1.0
Date: 2026-11-11. Status: approved architectural baseline; M1 vertical slice in progress (see README "Périmètre" and docs/CONTRACTS.md for what is implemented).

## Product vision
An AI-native layer over Windows 11, for personal use on NVIDIA/AMD dedicated-GPU systems. A model-agnostic assistant selects local or authorized cloud models for text, vision, voice, image and video; understands user intent; plans multi-step tasks; safely interacts with OS and applications; verifies execution and reports observable results.

## Architecture Decision Records
- ADR-001 Windows 11 x64 desktop app (Tauri 2 + React/TypeScript).
- ADR-002 Rust core supervisor, IPC, service lifecycle and event stream.
- ADR-003 Rust policy and execution broker, independent of LLM, deny by default.
- ADR-004 .NET Windows bridge for Win32/UIA and Windows actions.
- ADR-005 Python agent runtime in an unprivileged separate process.
- ADR-006 provider-agnostic Model Gateway with capability metadata, explicit provider profiles, privacy filters and validated fallback.
- ADR-007 gRPC/Protobuf over authenticated, ACL-protected local transports where supported; Tauri IPC for frontend.
- ADR-008 SQLite locally for tasks, events, configurations; credentials via Windows secure storage; do not expose APIs to LAN by default.
- ADR-009 MCP tools pass through the AiWin11 policy broker.
- ADR-010 progressive evolution through architecture decision records.

## Process topology
1. aiwin11-desktop (UI)
2. aiwin11-core (service supervisor, event bus, sessions)
3. aiwin11-broker (policies, approvals, audited execution)
4. aiwin11-windows (.NET Windows bridge)
5. aiwin11-agent (planning, tool calls, verification)
6. aiwin11-model (model router and adapters)
External local runtimes may be managed separately (Ollama, LM Studio).

## Modules
M01 Provider & Model Hub (P0)
M02 Windows Integration Engine (P0)
M03 Agent Runtime (P0)
M04 Security & Permissions (P0)
M05 Context & Memory (P1)
M06 Multimodal Engine (P1/P2)
M07 Automation & Workflow Studio (P1)
M08 Extensions & MCP (P1)
M09 Desktop Experience (P0/P1)
M10 Observability & Diagnostics (P1)

## Windows action selection
Prefer official OS/application APIs, then UI Automation, then permitted visual UI recognition, then emulated mouse and keyboard. UI references must account for focus, DPI, multi-monitor and stale nodes. Do not attempt to bypass secure desktops or Windows integrity boundaries.

## Model routing
Strategies: manual, local_only, cloud_only, local_first, quality_first, cost_optimized, adaptive. Filter on capabilities (tool-calling, JSON output, vision, streaming, context), policy, cost and hardware availability before ranking. If strict privacy or local_only is active, no cloud fallback. Detect real provider capabilities and GPU support rather than assuming NVIDIA/AMD equivalence. Use cost budgets per task.

## Agent task lifecycle
CREATED -> PLANNING -> RUNNING -> COMPLETED / FAILED; intermediate AWAITING_APPROVAL, PAUSED, CANCELLING, CANCELLED, RECOVERING. Stages Understand, Plan, Authorize, Execute, Observe, Verify, Recover, Report. Authorization is re-evaluated before each tool effect. Cancellation stops new work and attempts safe termination of running tasks.

## Security invariants
- Default deny; unknown tools/actions denied.
- LLM outputs, web pages, documents, MCP tool content are untrusted data, not authorization.
- Broker validates identity and policy for each side effect, with time/scope-limited grants.
- No blanket elevated process, no agent self-grants, no unapproved exfiltration.
- Resolve/canonicalize filesystem paths and symlinks before policy decisions and at use time.
- Audit actions, decisions and actual outcomes while redacting secrets.
- Dry-run and backups where feasible; do not promise arbitrary rollback.
- Stop button, cancellation acknowledgment, rate limiting, isolation, prompt injection testing.

## First successful vertical slice
Chat command "Ouvre le Bloc-notes" -> core -> model gateway -> agent's structured tool request -> broker policy -> .NET Windows bridge launches Notepad -> verifies process/window -> audit -> streamed UI result. No admin rights.

## Non-functional initial targets (benchmarks, not guarantees)
Palette p95 < 200ms, simple policy evaluation p95 < 50ms, UI navigation p95 < 150ms, cold UI ready < 3s on defined reference machine, host idle memory < 350MB excluding inference. Reliability: isolated provider/agent crashes do not crash UI/core. CI must test typed contracts, permissions and basic workflow.

## Out-of-scope for M1
Autonomous elevation, LAN-facing services, unattended destructive actions, unrestricted shell, video generation, advanced graph workflows, multi-agent delegation. These are subsequent milestones after safe foundation.
