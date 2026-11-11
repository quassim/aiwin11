# AiWin11 — Codex instructions

You are implementing AiWin11, a personal AI-native platform for Windows 11 x64 with a dedicated NVIDIA/AMD GPU and a balanced local/cloud model strategy.

## Non-negotiable architecture
- Desktop: Tauri 2 + React + TypeScript.
- Core supervisor and policy/execution broker: Rust.
- Windows integration bridge: .NET / C# (Win32 and Windows UI Automation).
- Agent runtime: Python process separated from trusted services.
- Model Gateway: provider-neutral; Ollama/local and one optional cloud provider initially.
- Contracts: versioned Protobuf, typed API, event envelope.
- Persistence: local SQLite; credentials in Windows Credential Manager/DPAPI, never in plain SQLite.
- No autonomous system calls that bypass the broker. Deny by default.
- All app components run as standard user initially; no elevation or auto-start services without explicit review.
- Default private; never send screen data, files, or secrets to cloud without explicit policy.

## Development conduct
1. Implement tickets sequentially, beginning with AW-001 through AW-005, then the M1 vertical slice AW-006 through AW-012.
2. Before modifying code, inspect repo, document assumptions, and produce a short implementation plan.
3. Favor small commits, straightforward abstractions, pinned dependencies and repeatable development commands.
4. Do not claim Windows integration tested if running outside Windows; add Windows CI/tests and disclose limitations.
5. Never implement blanket shell execution, unrestricted filesystem access, or implicit fallback local->cloud.
6. Every tool action must validate policy at execution time, generate an audit event, and support clear errors.
7. Avoid hardcoded model names or fake capability claims; test provider features.
8. Do not silently send telemetry or expose loopback servers to the LAN.
9. Prefer accessible, polished, keyboard-usable UI with clear local/cloud badges and agent activity.
10. Deliver a working vertical slice: user requests "Ouvre le Bloc-notes", gateway selects an allowed model, agent requests a structured tool, broker authorizes, Windows bridge launches notepad, result is verified and audited.

## Definition of done
- Build and tests pass on a real Windows 11 test runner; non-Windows limitations explicitly documented.
- Code linted and typed.
- Relevant tests exist (unit, integration, security).
- No tool operation bypasses policy broker.
- Public APIs and migrations documented.
- README includes setup, run, and troubleshooting instructions.

Read docs/SPECIFICATION.md, docs/BACKLOG.md, docs/CONTRACTS.md before coding.
