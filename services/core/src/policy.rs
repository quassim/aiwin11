//! Policy engine: decides what happens to an action the agent proposed, before any
//! side effect. The broker still checks every command at execution time; this engine only
//! decides whether the host may request the matching capability, and whether the user must
//! approve first.
//!
//! Decisions are data: a matrix of (action, autonomy profile) -> effect, then fixed
//! invariants that no profile can relax (local-only tools, screen data, critical actions,
//! simulation mode). Unknown values and storage errors resolve to the most restrictive choice.

use serde::{Deserialize, Serialize};

use crate::manifest::{manifest, Risk};

/// What the engine decided for one proposed action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Run without asking.
    Allow,
    /// Ask the user for this exact action first.
    Ask,
    /// Refuse; nothing runs.
    Deny,
    /// Dry run only: report what would happen, change nothing.
    Simulate,
}

/// Autonomy profiles from the specification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Autonomy {
    /// Reads only; every side effect is refused.
    Observation,
    /// Reads run; every side effect asks first. Default.
    #[default]
    Assisted,
    /// Low-risk side effects inside AiWin11's own scope run without asking.
    Limited,
    /// Wider autonomy; critical actions and screen reading still ask.
    Advanced,
}

impl Autonomy {
    pub fn code(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Assisted => "assisted",
            Self::Limited => "limited",
            Self::Advanced => "advanced",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Observation, Self::Assisted, Self::Limited, Self::Advanced]
            .into_iter()
            .find(|autonomy| autonomy.code() == code)
    }
}

/// User-chosen settings, persisted by the core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicySettings {
    pub autonomy: Autonomy,
    /// Every side effect becomes a dry run, whatever the profile.
    pub simulate: bool,
}

/// Actions the agent can propose. Each maps to one tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentAction {
    LaunchApplication,
    ListFiles,
    ReadFile,
    CopyFile,
    MoveFile,
    TrashFile,
    InspectWindow,
    DescribeWindow,
    TypeText,
    ListProcesses,
    SystemStatus,
    ReadClipboard,
    WriteClipboard,
    ListWindows,
    ArrangeWindow,
    ListServices,
    AdminService,
    AdminCleanTemp,
    ClickControl,
    SetControlValue,
    Remember,
    Recall,
    SearchMemory,
    RunDiagnostic,
    OrganizeFiles,
    McpListTools,
    McpCall,
    GenerateImage,
}

impl AgentAction {
    pub const ALL: [Self; 28] = [
        Self::LaunchApplication,
        Self::ListFiles,
        Self::ReadFile,
        Self::CopyFile,
        Self::MoveFile,
        Self::TrashFile,
        Self::InspectWindow,
        Self::DescribeWindow,
        Self::TypeText,
        Self::ListProcesses,
        Self::SystemStatus,
        Self::ReadClipboard,
        Self::WriteClipboard,
        Self::ListWindows,
        Self::ArrangeWindow,
        Self::ListServices,
        Self::AdminService,
        Self::AdminCleanTemp,
        Self::ClickControl,
        Self::SetControlValue,
        Self::Remember,
        Self::Recall,
        Self::SearchMemory,
        Self::RunDiagnostic,
        Self::OrganizeFiles,
        Self::McpListTools,
        Self::McpCall,
        Self::GenerateImage,
    ];

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.code() == code)
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::LaunchApplication => "launch_application",
            Self::ListFiles => "list_files",
            Self::ReadFile => "read_file",
            Self::CopyFile => "copy_file",
            Self::MoveFile => "move_file",
            Self::TrashFile => "trash_file",
            Self::InspectWindow => "inspect_window",
            Self::DescribeWindow => "describe_window",
            Self::TypeText => "type_text",
            Self::ListProcesses => "list_processes",
            Self::SystemStatus => "system_status",
            Self::ReadClipboard => "read_clipboard",
            Self::WriteClipboard => "write_clipboard",
            Self::ListWindows => "list_windows",
            Self::ArrangeWindow => "arrange_window",
            Self::ListServices => "list_services",
            Self::AdminService => "admin_service",
            Self::AdminCleanTemp => "admin_clean_temp",
            Self::ClickControl => "click_control",
            Self::SetControlValue => "set_control_value",
            Self::Remember => "remember",
            Self::Recall => "recall",
            Self::SearchMemory => "search_memory",
            Self::RunDiagnostic => "run_diagnostic",
            Self::OrganizeFiles => "organize_files",
            Self::McpListTools => "mcp_list_tools",
            Self::McpCall => "mcp_call",
            Self::GenerateImage => "generate_image",
        }
    }

    /// Changes something on the machine (as opposed to reading). From the tool manifest.
    pub fn has_side_effect(self) -> bool {
        manifest(self).side_effect
    }

    /// Reads what another application shows on screen. From the tool manifest.
    pub fn reads_screen(self) -> bool {
        manifest(self).reads_screen
    }

    /// Never offered to, nor run for, a cloud model: file contents and screen data stay local.
    pub fn local_only(self) -> bool {
        manifest(self).local_only
    }

    /// Reads private data the user did not hand over (the clipboard): always asks.
    pub fn private_data(self) -> bool {
        manifest(self).private_data
    }

    /// Always asks, in every profile: the manifest rates it critical.
    pub fn critical(self) -> bool {
        manifest(self).risk == Risk::Critical
    }
}

/// Facts about the running task that rules depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionContext {
    pub local_model: bool,
    /// The user gave a temporary grant for this action that is still in force.
    pub temporary_grant: bool,
    /// The specialized agent running the task: it may only use its own tools.
    pub profile: AgentProfile,
}

/// Specialized agents (ROADMAP R8, "permissions par agent"): each one only sees and may only
/// use the tools of its domain. Chosen by the user for each request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentProfile {
    /// Every tool.
    #[default]
    General,
    /// Files, Notepad, clipboard, memory.
    Office,
    /// Diagnostics, processes, services, windows, administrator operations.
    WindowsAdmin,
    /// Tools without side effects only.
    ReadOnly,
}

impl AgentProfile {
    pub fn code(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Office => "office",
            Self::WindowsAdmin => "windows_admin",
            Self::ReadOnly => "read_only",
        }
    }

    pub fn allows(self, action: AgentAction) -> bool {
        use AgentAction as A;
        match self {
            Self::General => true,
            Self::ReadOnly => !action.has_side_effect(),
            Self::Office => matches!(
                action,
                A::LaunchApplication
                    | A::ListFiles
                    | A::ReadFile
                    | A::CopyFile
                    | A::MoveFile
                    | A::TrashFile
                    | A::OrganizeFiles
                    | A::TypeText
                    | A::ReadClipboard
                    | A::WriteClipboard
                    | A::Remember
                    | A::Recall
                    | A::SearchMemory
                    | A::GenerateImage
            ),
            Self::WindowsAdmin => matches!(
                action,
                A::SystemStatus
                    | A::ListProcesses
                    | A::ListServices
                    | A::RunDiagnostic
                    | A::AdminService
                    | A::AdminCleanTemp
                    | A::ListWindows
                    | A::ArrangeWindow
                    | A::InspectWindow
                    | A::Recall
                    | A::SearchMemory
            ),
        }
    }
}

/// A temporary grant can only relax an `Ask` for a side effect that is neither critical nor
/// a screen read.
pub fn grantable(action: AgentAction) -> bool {
    action.has_side_effect() && !action.critical() && !action.reads_screen() && !action.private_data()
}

/// An effect and the rule that produced it, for the audit trail and the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub effect: Effect,
    pub rule: &'static str,
}

use AgentAction as A;
use Autonomy as P;
use Effect as E;

/// Base matrix: effect of each action in each profile, before invariants.
/// Order of columns: Observation, Assisted, Limited, Advanced.
const MATRIX: [(AgentAction, [Effect; 4]); 28] = [
    (A::GenerateImage, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::McpListTools, [E::Ask, E::Ask, E::Allow, E::Allow]),
    (A::McpCall, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::OrganizeFiles, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::RunDiagnostic, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::SearchMemory, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::Recall, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::Remember, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::SetControlValue, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::ClickControl, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::AdminService, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::AdminCleanTemp, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::ListServices, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::ListWindows, [E::Ask, E::Ask, E::Ask, E::Ask]),
    (A::ArrangeWindow, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::ReadClipboard, [E::Ask, E::Ask, E::Ask, E::Ask]),
    (A::WriteClipboard, [E::Deny, E::Ask, E::Ask, E::Ask]),
    (A::SystemStatus, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::ListFiles, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::ListProcesses, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::ReadFile, [E::Allow, E::Allow, E::Allow, E::Allow]),
    (A::InspectWindow, [E::Ask, E::Ask, E::Ask, E::Ask]),
    (A::DescribeWindow, [E::Ask, E::Ask, E::Ask, E::Ask]),
    (A::LaunchApplication, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::CopyFile, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::TypeText, [E::Deny, E::Ask, E::Allow, E::Allow]),
    (A::MoveFile, [E::Deny, E::Ask, E::Ask, E::Allow]),
    (A::TrashFile, [E::Deny, E::Ask, E::Ask, E::Ask]),
];

fn column(autonomy: Autonomy) -> usize {
    match autonomy {
        P::Observation => 0,
        P::Assisted => 1,
        P::Limited => 2,
        P::Advanced => 3,
    }
}

/// Base effect from the matrix; an action missing from it is denied.
pub fn base_effect(action: AgentAction, autonomy: Autonomy) -> Effect {
    MATRIX.iter().find(|(entry, _)| *entry == action).map_or(E::Deny, |(_, effects)| effects[column(autonomy)])
}

/// Decides the effect of one proposed action. Invariants are applied in this order and can
/// only make the outcome more restrictive than the matrix, except simulation, which replaces
/// any non-denied side effect with a dry run.
pub fn evaluate(settings: PolicySettings, action: AgentAction, context: ActionContext) -> Decision {
    if !context.profile.allows(action) {
        return Decision { effect: E::Deny, rule: "agent_scope" };
    }
    if action.local_only() && !context.local_model {
        return Decision { effect: E::Deny, rule: "local_only" };
    }
    let base = base_effect(action, settings.autonomy);
    if base == E::Deny {
        return Decision { effect: E::Deny, rule: "profile" };
    }
    if action.has_side_effect() && settings.simulate {
        return Decision { effect: E::Simulate, rule: "simulation_mode" };
    }
    if action.reads_screen() {
        return Decision { effect: E::Ask, rule: "screen_data" };
    }
    if action.private_data() {
        return Decision { effect: E::Ask, rule: "private_data" };
    }
    if action.critical() {
        return Decision { effect: E::Ask, rule: "critical_action" };
    }
    if base == E::Ask && context.temporary_grant && grantable(action) {
        return Decision { effect: E::Allow, rule: "temporary_grant" };
    }
    Decision { effect: base, rule: "profile" }
}

/// The full table for the UI: effect of every action in every profile, for a local model.
pub fn table(simulate: bool) -> Vec<(AgentAction, [Effect; 4])> {
    let context = ActionContext { local_model: true, temporary_grant: false, profile: AgentProfile::General };
    AgentAction::ALL
        .into_iter()
        .map(|action| {
            let effects = [P::Observation, P::Assisted, P::Limited, P::Advanced]
                .map(|autonomy| evaluate(PolicySettings { autonomy, simulate }, action, context).effect);
            (action, effects)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: ActionContext =
        ActionContext { local_model: true, temporary_grant: false, profile: AgentProfile::General };
    const CLOUD: ActionContext =
        ActionContext { local_model: false, temporary_grant: false, profile: AgentProfile::General };
    const GRANTED: ActionContext =
        ActionContext { local_model: true, temporary_grant: true, profile: AgentProfile::General };

    #[test]
    fn specialized_agents_only_use_their_own_tools() {
        let with = |profile| ActionContext { profile, ..LOCAL };
        let decide = |profile, action| evaluate(PolicySettings::default(), action, with(profile));
        assert_eq!(decide(AgentProfile::Office, A::AdminService).rule, "agent_scope");
        assert_eq!(decide(AgentProfile::WindowsAdmin, A::TrashFile).rule, "agent_scope");
        assert_ne!(decide(AgentProfile::WindowsAdmin, A::ListServices).effect, E::Deny);
        for action in AgentAction::ALL {
            let read_only = decide(AgentProfile::ReadOnly, action);
            assert_eq!(
                read_only.effect == E::Deny && read_only.rule == "agent_scope",
                action.has_side_effect(),
                "{action:?}"
            );
            assert_eq!(decide(AgentProfile::General, action), evaluate(PolicySettings::default(), action, LOCAL));
        }
    }

    #[test]
    fn a_temporary_grant_only_turns_an_ask_into_allow() {
        for autonomy in [P::Observation, P::Assisted, P::Limited, P::Advanced] {
            for action in AgentAction::ALL {
                let without = evaluate(PolicySettings { autonomy, simulate: false }, action, LOCAL);
                let with = evaluate(PolicySettings { autonomy, simulate: false }, action, GRANTED);
                if without.effect == E::Ask && grantable(action) {
                    assert_eq!(with, Decision { effect: E::Allow, rule: "temporary_grant" }, "{autonomy:?} {action:?}");
                } else {
                    assert_eq!(with, without, "{autonomy:?} {action:?}");
                }
            }
            // Never for critical actions or screen reading, never past simulation or the cloud rule.
            assert_eq!(effect(autonomy, false, A::TrashFile, GRANTED), effect(autonomy, false, A::TrashFile, LOCAL));
            assert_eq!(effect(autonomy, false, A::InspectWindow, GRANTED), E::Ask);
            assert_ne!(effect(autonomy, true, A::CopyFile, GRANTED), E::Allow);
        }
        let cloud_granted = ActionContext { local_model: false, temporary_grant: true, profile: AgentProfile::General };
        assert_eq!(effect(P::Assisted, false, A::CopyFile, cloud_granted), E::Deny);
        assert_eq!(effect(P::Observation, false, A::CopyFile, GRANTED), E::Deny, "a denial stays a denial");
    }

    fn effect(autonomy: Autonomy, simulate: bool, action: AgentAction, context: ActionContext) -> Effect {
        evaluate(PolicySettings { autonomy, simulate }, action, context).effect
    }

    #[test]
    fn every_action_has_a_matrix_row() {
        for action in AgentAction::ALL {
            assert!(MATRIX.iter().any(|(entry, _)| *entry == action), "{action:?}");
        }
    }

    #[test]
    fn assisted_is_the_default_and_asks_for_every_side_effect() {
        assert_eq!(PolicySettings::default(), PolicySettings { autonomy: P::Assisted, simulate: false });
        for action in AgentAction::ALL {
            // Listing MCP tools starts a program the user declared: it asks too.
            let asks =
                action.has_side_effect() || action.reads_screen() || action.private_data() || action == A::McpListTools;
            let expected = if asks { E::Ask } else { E::Allow };
            assert_eq!(effect(P::Assisted, false, action, LOCAL), expected, "{action:?}");
        }
    }

    #[test]
    fn observation_denies_every_side_effect_but_still_reads() {
        for action in AgentAction::ALL {
            let got = effect(P::Observation, false, action, LOCAL);
            if action.has_side_effect() {
                assert_eq!(got, E::Deny, "{action:?}");
            } else {
                assert_ne!(got, E::Deny, "{action:?}");
            }
        }
    }

    #[test]
    fn no_profile_reads_the_screen_or_trashes_without_asking() {
        for autonomy in [P::Observation, P::Assisted, P::Limited, P::Advanced] {
            for action in [A::InspectWindow, A::DescribeWindow] {
                assert_eq!(effect(autonomy, false, action, LOCAL), E::Ask, "{autonomy:?} {action:?}");
            }
            assert_ne!(effect(autonomy, false, A::TrashFile, LOCAL), E::Allow, "{autonomy:?}");
        }
    }

    #[test]
    fn autonomy_only_grows_from_observation_to_advanced() {
        let rank = |effect| match effect {
            E::Deny => 0,
            E::Ask => 1,
            E::Simulate => 1,
            E::Allow => 2,
        };
        for action in AgentAction::ALL {
            let effects = [P::Observation, P::Assisted, P::Limited, P::Advanced]
                .map(|autonomy| rank(effect(autonomy, false, action, LOCAL)));
            assert!(effects.windows(2).all(|pair| pair[0] <= pair[1]), "{action:?} {effects:?}");
        }
    }

    #[test]
    fn cloud_models_get_no_local_only_tool_in_any_mode() {
        for autonomy in [P::Observation, P::Assisted, P::Limited, P::Advanced] {
            for simulate in [false, true] {
                for action in AgentAction::ALL.into_iter().filter(|action| action.local_only()) {
                    let decision = evaluate(PolicySettings { autonomy, simulate }, action, CLOUD);
                    assert_eq!(decision, Decision { effect: E::Deny, rule: "local_only" });
                }
            }
        }
        assert_eq!(effect(P::Assisted, false, A::LaunchApplication, CLOUD), E::Ask);
    }

    #[test]
    fn simulation_turns_allowed_or_asked_side_effects_into_dry_runs_only() {
        for autonomy in [P::Assisted, P::Limited, P::Advanced] {
            for action in AgentAction::ALL {
                let got = evaluate(PolicySettings { autonomy, simulate: true }, action, LOCAL);
                if action.has_side_effect() {
                    assert_eq!(got, Decision { effect: E::Simulate, rule: "simulation_mode" }, "{action:?}");
                } else {
                    assert_eq!(got.effect, effect(autonomy, false, action, LOCAL), "{action:?}");
                }
            }
        }
        assert_eq!(effect(P::Observation, true, A::CopyFile, LOCAL), E::Deny, "simulation never relaxes a denial");
    }

    #[test]
    fn profile_codes_round_trip_and_unknown_codes_are_rejected() {
        for autonomy in [P::Observation, P::Assisted, P::Limited, P::Advanced] {
            assert_eq!(Autonomy::from_code(autonomy.code()), Some(autonomy));
        }
        assert_eq!(Autonomy::from_code("root"), None);
        assert_eq!(table(false).len(), AgentAction::ALL.len());
        let simulated = table(true);
        let copy = simulated.iter().find(|(action, _)| *action == A::CopyFile).unwrap().1;
        assert_eq!(copy, [E::Deny, E::Simulate, E::Simulate, E::Simulate]);
    }
}
