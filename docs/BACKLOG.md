# AiWin11 Prioritized Backlog
Estimates in developer-days, not guaranteed elapsed time.

## P0 — 46–71 d-dev
| ID | Work | Dependency | Estimate |
|---|---|---|---|
| AW-001 | Initialize monorepo, tooling, CI | - | 2–3 |
| AW-002 | Tauri app + React navigation | AW-001 | 3–4 |
| AW-003 | Protobuf v1 contracts + codegen | AW-001 | 3–4 |
| AW-004 | Core host, session and events | AW-003 | 4–6 |
| AW-005 | Broker and policy engine | AW-003 | 6–9 |
| AW-006 | Windows Bridge: windows/apps | AW-005 | 4–6 |
| AW-007 | Model Gateway + local provider | AW-003 | 4–6 |
| AW-008 | First cloud provider + secrets | AW-007 | 3–4 |
| AW-009 | Provider/model selection UI | AW-002/007 | 3–5 |
| AW-010 | Agent runtime planning/tool calls | AW-004/005/007 | 6–9 |
| AW-011 | Streaming chat and task progress | AW-010 | 4–6 |
| AW-012 | E2E desktop scenario | AW-006/010 | 4–6 |

## P1 — 68–102 d-dev
AW-013 UIA inspector (6–9); AW-014 screen capture/vision (5–8); AW-015 controlled input (4–6); AW-016 safe file operations (5–7); AW-017 audit/recovery (5–8); AW-018 memory/indexing (6–9); AW-019 MCP SDK (6–9); AW-020 adaptive GPU routing (6–9); AW-021 STT/TTS (5–8); AW-022 overlay/tray/hotkeys (5–7); AW-023 workflows/triggers (7–10); AW-024 security/robustness tests (8–12).

## P2 — 57–88 d-dev
AW-025 multi-agent (8–12); AW-026 visual workflow editor (10–15); AW-027 image generation (5–8); AW-028 video jobs (6–10); AW-029 advanced event automation (8–12); AW-030 cost/model monitoring (5–8); AW-031 plugin catalog (7–11); AW-032 packaging/updates/diagnostics (8–12).

## Lot sequence
A: AW-001–005; B: AW-006–012; C: AW-013–017; D: AW-018–024; E: AW-025–032.

## Status (2026-11-12)
- Lot A (AW-001–005): done; the broker is completed by a data-driven policy engine with autonomy profiles and simulation mode. Cross-process gRPC transport is still pending (the core runs inside Tauri).
- Lot B (AW-006–012): done and validated live with Ollama `qwen3:8b`.
- Lot C (AW-013–017): done for the agent — window inspection, local vision description, Notepad text input, file read/copy/rename/trash, persistent tasks with crash recovery. Live validation of the latest steps is part of the final verification.
- Lots D and E: not started.

## Definition of done
Working code, typed contracts, documented setup, relevant tests green, verified broker enforcement and audit of all side effects. Do not represent mocks as complete Windows integration.
