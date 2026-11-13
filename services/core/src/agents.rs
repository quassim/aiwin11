//! Agents defined by the user, as data (ROADMAP R8, "agents spécialisés définis en données").
//!
//! A definition names a built-in specialized agent as its base and can only narrow it: its
//! tools are a subset of the base's, its role adds instructions, its step limit is at most the
//! core's. The policy engine enforces the tool list at execution time (rule `agent_tools`), so a
//! definition never grants anything: it is a further restriction. A user agent never delegates
//! (the general agent is not a valid base), and a delegation never carries an approval or a
//! temporary grant from one agent to another.

use serde::{Deserialize, Serialize};

use crate::{
    manifest::MANIFESTS,
    policy::{AgentAction, AgentProfile},
    Core,
};

pub const MAX_CUSTOM_AGENTS: usize = 8;
pub const MAX_ROLE_CHARS: usize = 600;
const MAX_NAME: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinition {
    /// Short identifier: lowercase letters, digits and `-`.
    pub name: String,
    /// The built-in agent it narrows (never the general agent).
    pub base: AgentProfile,
    /// Extra instructions given to the model, in the user's words.
    pub role: String,
    /// Tool names it may use; empty: every tool of its base.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Local Ollama model for this agent; empty: the model of its base, or the request's.
    #[serde(default)]
    pub model: String,
    /// Model steps allowed for one request.
    #[serde(default = "default_steps")]
    pub max_steps: u32,
}

fn default_steps() -> u32 {
    crate::task::MAX_AGENT_STEPS as u32
}

pub fn valid_agent_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl AgentDefinition {
    pub fn validated(self) -> Result<Self, &'static str> {
        let role = self.role.trim().to_owned();
        let tools_ok = self.tools.len() <= MANIFESTS.len()
            && self.tools.iter().enumerate().all(|(index, tool)| {
                !self.tools[..index].contains(tool)
                    && MANIFESTS.iter().any(|manifest| manifest.tool == tool && self.base.allows(manifest.action))
            });
        let ok = valid_agent_name(&self.name)
            && self.base != AgentProfile::General
            && !role.is_empty()
            && role.chars().count() <= MAX_ROLE_CHARS
            && !role.contains('\0')
            && tools_ok
            && (self.model.is_empty() || crate::valid_model_name(&self.model))
            && (1..=crate::task::MAX_AGENT_STEPS as u32).contains(&self.max_steps);
        ok.then_some(Self { role, ..self }).ok_or("AGENT_DEFINITION_INVALID")
    }

    /// The actions it may use, as a mask over `AgentAction::ALL`; `None`: all of its base.
    pub fn mask(&self) -> Option<u64> {
        (!self.tools.is_empty()).then(|| {
            MANIFESTS
                .iter()
                .filter(|manifest| self.tools.iter().any(|tool| tool == manifest.tool))
                .fold(0, |mask, manifest| mask | manifest.action.bit())
        })
    }

    pub fn permits(&self, action: AgentAction) -> bool {
        self.base.allows(action) && self.mask().is_none_or(|mask| mask & action.bit() != 0)
    }
}

impl Core {
    pub fn custom_agents(&self) -> Vec<AgentDefinition> {
        self.setting("agents.custom")
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<Vec<AgentDefinition>>(&json).ok())
            .map(|agents| agents.into_iter().filter_map(|agent| agent.validated().ok()).collect())
            .unwrap_or_default()
    }

    /// Replaces the user's agents (at most 8, unique names, each validated).
    pub fn set_custom_agents(&mut self, agents: Vec<AgentDefinition>) -> Result<(), &'static str> {
        if agents.len() > MAX_CUSTOM_AGENTS {
            return Err("AGENT_DEFINITION_INVALID");
        }
        let agents: Vec<AgentDefinition> =
            agents.into_iter().map(AgentDefinition::validated).collect::<Result<_, _>>()?;
        let mut names: Vec<&str> = agents.iter().map(|agent| agent.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != agents.len() {
            return Err("AGENT_DEFINITION_INVALID");
        }
        let json = serde_json::to_string(&agents).map_err(|_| "AGENT_DEFINITION_INVALID")?;
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('agents.custom', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [json],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.journal("settings.agents.changed", None).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    pub fn custom_agent(&self, name: &str) -> Option<AgentDefinition> {
        self.custom_agents().into_iter().find(|agent| agent.name == name)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn definition() -> AgentDefinition {
        AgentDefinition {
            name: "veille".into(),
            base: AgentProfile::Research,
            role: "  Tu fais une veille sur les fichiers de notes.  ".into(),
            tools: vec!["search_text_files".into(), "read_text_file".into()],
            model: String::new(),
            max_steps: 4,
        }
    }

    #[test]
    fn definitions_only_narrow_their_base() {
        let agent = definition().validated().unwrap();
        assert_eq!(agent.role, "Tu fais une veille sur les fichiers de notes.");
        assert!(agent.permits(AgentAction::ReadFile) && agent.permits(AgentAction::SearchFiles));
        assert!(!agent.permits(AgentAction::ListFiles), "outside its own list");
        assert!(!agent.permits(AgentAction::LaunchApplication), "outside its base");
        assert!(!agent.permits(AgentAction::Delegate), "never delegates");
        let all = AgentDefinition { tools: Vec::new(), ..definition() }.validated().unwrap();
        assert!(all.mask().is_none() && all.permits(AgentAction::ListFiles));
        for bad in [
            AgentDefinition { base: AgentProfile::General, ..definition() },
            AgentDefinition { name: "Veille".into(), ..definition() },
            AgentDefinition { role: " ".into(), ..definition() },
            AgentDefinition { tools: vec!["launch_application".into()], ..definition() },
            AgentDefinition { tools: vec!["read_text_file".into(), "read_text_file".into()], ..definition() },
            AgentDefinition { max_steps: 0, ..definition() },
            AgentDefinition { max_steps: 99, ..definition() },
            AgentDefinition { model: "bad model".into(), ..definition() },
        ] {
            assert_eq!(bad.validated(), Err("AGENT_DEFINITION_INVALID"));
        }
    }

    #[test]
    fn definitions_are_stored_with_unique_names() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        core.set_custom_agents(vec![definition()]).unwrap();
        assert_eq!(core.custom_agent("veille").unwrap().max_steps, 4);
        assert_eq!(core.set_custom_agents(vec![definition(), definition()]), Err("AGENT_DEFINITION_INVALID"));
        assert_eq!(core.custom_agents().len(), 1, "a refused list changes nothing");
    }
}
