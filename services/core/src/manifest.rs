//! Tool manifests: the reviewed facts about each agent tool (risk, dry run, reversibility,
//! deadline, broker capabilities). The policy engine derives its invariants from these
//! facts instead of hard-coded lists, and the Security page shows them to the user.
//!
//! A manifest describes what a tool is allowed to be; it never grants anything. Grants
//! still come from the policy decision, the user's approval and the broker's check.

use serde::Serialize;

use crate::policy::AgentAction;

/// Risk of a tool's worst legitimate outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Reads AiWin11's own data or system facts; changes nothing.
    Low,
    /// Changes something inside AiWin11's scope, or reads another application.
    Medium,
    /// Changes something the user may care about; recoverable.
    High,
    /// Destructive or security-relevant: always asks, whatever the profile.
    Critical,
}

/// How a side effect can be undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reversibility {
    /// Nothing changes.
    NoEffect,
    /// The user can undo it directly (close a window, delete a copy, rename back).
    Reversible,
    /// Recoverable from AiWin11's trash.
    Recoverable,
    /// Cannot be undone.
    Irreversible,
}

/// Broker capability needed by a tool, by name (see `broker::Capability`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityName {
    LaunchNotepad,
    FileRead,
    FileCopy,
    FileMove,
    FileTrash,
    WindowLookup,
    WindowInspect,
    WindowCapture,
    TypeText,
    ProcessList,
    SystemStatus,
    ClipboardRead,
    ClipboardWrite,
    WindowList,
    WindowArrange,
    ServiceList,
    AdminOperation,
    LaunchAllowedApp,
    ControlInvoke,
    MemoryWrite,
    MemoryRead,
    Diagnostic,
    McpServer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolManifest {
    pub action: AgentAction,
    /// Name in the model's tool catalog and the Python agent's registry.
    pub tool: &'static str,
    pub version: u32,
    pub risk: Risk,
    /// Changes something on the machine (as opposed to reading).
    pub side_effect: bool,
    /// Reads what another application shows on screen.
    pub reads_screen: bool,
    /// Reads private data the user did not hand over (clipboard): always asks, never grantable.
    pub private_data: bool,
    /// Never offered to, nor run for, a cloud model.
    pub local_only: bool,
    /// The host can describe the effect without performing it (simulation mode).
    pub dry_run: bool,
    pub reversibility: Reversibility,
    /// Longest time the execution may take once authorized, in milliseconds.
    pub timeout_ms: u32,
    pub capabilities: &'static [CapabilityName],
}

use CapabilityName as C;
use Reversibility as R;

/// One manifest per agent action, in `AgentAction::ALL` order.
pub const MANIFESTS: [ToolManifest; 27] = [
    ToolManifest {
        action: AgentAction::LaunchApplication,
        tool: "launch_application",
        version: 1,
        risk: Risk::Medium,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: false,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 20_000,
        capabilities: &[C::LaunchNotepad, C::LaunchAllowedApp],
    },
    ToolManifest {
        action: AgentAction::ListFiles,
        tool: "list_text_files",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 5_000,
        capabilities: &[C::FileRead],
    },
    ToolManifest {
        action: AgentAction::ReadFile,
        tool: "read_text_file",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 5_000,
        capabilities: &[C::FileRead],
    },
    ToolManifest {
        action: AgentAction::CopyFile,
        tool: "copy_text_file",
        version: 1,
        risk: Risk::Medium,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 5_000,
        capabilities: &[C::FileCopy],
    },
    ToolManifest {
        action: AgentAction::MoveFile,
        tool: "move_text_file",
        version: 1,
        risk: Risk::High,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 5_000,
        capabilities: &[C::FileMove],
    },
    ToolManifest {
        action: AgentAction::TrashFile,
        tool: "trash_text_file",
        version: 1,
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Recoverable,
        timeout_ms: 5_000,
        capabilities: &[C::FileTrash],
    },
    ToolManifest {
        action: AgentAction::InspectWindow,
        tool: "inspect_window",
        version: 1,
        risk: Risk::Medium,
        side_effect: false,
        reads_screen: true,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::WindowLookup, C::WindowInspect],
    },
    ToolManifest {
        action: AgentAction::DescribeWindow,
        tool: "describe_window",
        version: 1,
        risk: Risk::Medium,
        side_effect: false,
        reads_screen: true,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 105_000,
        capabilities: &[C::WindowLookup, C::WindowCapture],
    },
    ToolManifest {
        action: AgentAction::TypeText,
        tool: "type_in_notepad",
        version: 1,
        risk: Risk::Medium,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 15_000,
        capabilities: &[C::TypeText],
    },
    ToolManifest {
        action: AgentAction::ListProcesses,
        tool: "list_processes",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::ProcessList],
    },
    ToolManifest {
        action: AgentAction::SystemStatus,
        tool: "get_system_status",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::SystemStatus],
    },
    ToolManifest {
        action: AgentAction::ReadClipboard,
        tool: "read_clipboard",
        version: 1,
        risk: Risk::High,
        side_effect: false,
        reads_screen: false,
        private_data: true,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::ClipboardRead],
    },
    ToolManifest {
        action: AgentAction::WriteClipboard,
        tool: "write_clipboard",
        version: 1,
        // The previous clipboard content is lost: irreversible, hence critical.
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Irreversible,
        timeout_ms: 15_000,
        capabilities: &[C::ClipboardWrite],
    },
    ToolManifest {
        action: AgentAction::ListWindows,
        tool: "list_windows",
        version: 1,
        risk: Risk::Medium,
        side_effect: false,
        reads_screen: true,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::WindowList],
    },
    ToolManifest {
        action: AgentAction::ArrangeWindow,
        tool: "arrange_window",
        version: 1,
        risk: Risk::Medium,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 15_000,
        capabilities: &[C::WindowArrange],
    },
    ToolManifest {
        action: AgentAction::ListServices,
        tool: "list_services",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::ServiceList],
    },
    ToolManifest {
        action: AgentAction::AdminService,
        tool: "admin_service",
        version: 1,
        // Elevated: always asks, never grantable, plus the Windows UAC prompt.
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 90_000,
        capabilities: &[C::AdminOperation],
    },
    ToolManifest {
        action: AgentAction::AdminCleanTemp,
        tool: "admin_clean_temp",
        version: 1,
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Irreversible,
        timeout_ms: 90_000,
        capabilities: &[C::AdminOperation],
    },
    ToolManifest {
        action: AgentAction::ClickControl,
        tool: "click_control",
        version: 1,
        // A button can do anything in its application (send, delete, pay): always asks.
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Irreversible,
        timeout_ms: 15_000,
        capabilities: &[C::ControlInvoke],
    },
    ToolManifest {
        action: AgentAction::SetControlValue,
        tool: "set_control_value",
        version: 1,
        // Replaces what a field holds in another application: always asks.
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Irreversible,
        timeout_ms: 15_000,
        capabilities: &[C::ControlInvoke],
    },
    ToolManifest {
        action: AgentAction::Remember,
        tool: "remember",
        version: 1,
        risk: Risk::Medium,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        // Deletable from the Memory page.
        reversibility: R::Reversible,
        timeout_ms: 5_000,
        capabilities: &[C::MemoryWrite],
    },
    ToolManifest {
        action: AgentAction::Recall,
        tool: "recall",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 5_000,
        capabilities: &[C::MemoryRead],
    },
    ToolManifest {
        action: AgentAction::SearchMemory,
        tool: "search_memory",
        version: 1,
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 90_000,
        capabilities: &[C::MemoryRead],
    },
    ToolManifest {
        action: AgentAction::RunDiagnostic,
        tool: "run_diagnostic",
        version: 1,
        // Network details (addresses) stay on the machine: local models only.
        risk: Risk::Low,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 15_000,
        capabilities: &[C::Diagnostic],
    },
    ToolManifest {
        action: AgentAction::OrganizeFiles,
        tool: "organize_files",
        version: 1,
        // Up to 20 renames approved together: always shown as one previewed plan.
        risk: Risk::High,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Reversible,
        timeout_ms: 60_000,
        capabilities: &[C::FileMove],
    },
    ToolManifest {
        action: AgentAction::McpListTools,
        tool: "mcp_list_tools",
        version: 1,
        // Starts the user's declared server to read its tool list.
        risk: Risk::Medium,
        side_effect: false,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: false,
        reversibility: R::NoEffect,
        timeout_ms: 60_000,
        capabilities: &[C::McpServer],
    },
    ToolManifest {
        action: AgentAction::McpCall,
        tool: "mcp_call",
        version: 1,
        // An external tool can do anything: always asks, never grantable.
        risk: Risk::Critical,
        side_effect: true,
        reads_screen: false,
        private_data: false,
        local_only: true,
        dry_run: true,
        reversibility: R::Irreversible,
        timeout_ms: 60_000,
        capabilities: &[C::McpServer],
    },
];

/// The manifest of an action. Every action has one (checked by a test).
pub fn manifest(action: AgentAction) -> &'static ToolManifest {
    MANIFESTS.iter().find(|manifest| manifest.action == action).unwrap_or(&MANIFESTS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_exactly_one_manifest_in_order() {
        assert_eq!(MANIFESTS.map(|manifest| manifest.action), AgentAction::ALL);
    }

    #[test]
    fn manifests_are_consistent() {
        for manifest in MANIFESTS {
            let name = manifest.tool;
            assert!(!manifest.capabilities.is_empty(), "{name}: a tool needs a broker capability");
            assert!(manifest.timeout_ms > 0 && manifest.timeout_ms <= 120_000, "{name}");
            // A read has no effect to undo; a side effect must say how it is undone.
            assert_eq!(manifest.side_effect, manifest.reversibility != Reversibility::NoEffect, "{name}");
            // Every side effect can be simulated, so simulation mode never runs it for real.
            assert!(!manifest.side_effect || manifest.dry_run, "{name}");
            // Screen data and private data never leave the machine.
            assert!(!manifest.reads_screen || manifest.local_only, "{name}");
            assert!(!manifest.private_data || manifest.local_only, "{name}");
            // Irreversible tools must be critical (none exists yet).
            assert!(manifest.reversibility != Reversibility::Irreversible || manifest.risk == Risk::Critical, "{name}");
            assert!(manifest.risk > Risk::Low || !manifest.side_effect, "{name}");
        }
    }

    #[test]
    fn manifests_match_the_model_tool_catalog_and_the_python_agent() {
        let catalog = crate::model_gateway::tool_catalog(false, true, true, true);
        let mut offered: Vec<String> = catalog
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap().to_owned())
            .collect();
        offered.sort();
        let mut declared: Vec<String> = MANIFESTS.iter().map(|manifest| manifest.tool.to_owned()).collect();
        declared.sort();
        assert_eq!(offered, declared);
        let agent = include_str!("../../agent/agent.py");
        for manifest in MANIFESTS {
            assert!(agent.contains(&format!("\"{}\":", manifest.tool)), "{} missing from agent.py", manifest.tool);
        }
    }
}
