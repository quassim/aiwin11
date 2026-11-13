//! Schema-validated YAML configuration export and import for AiWin11 (ROADMAP §2.2).

use crate::{
    model_gateway::LocalStrategy,
    policy::{evaluate, ActionContext, AgentAction, AgentProfile, Autonomy, Effect, PolicySettings},
    AgentModels, Core,
};

const MAX_YAML_BYTES: usize = 8 * 1024;

fn autonomy_code(autonomy: Autonomy) -> &'static str {
    match autonomy {
        Autonomy::Observation => "observation",
        Autonomy::Assisted => "assisted",
        Autonomy::Limited => "limited",
        Autonomy::Advanced => "advanced",
    }
}

fn parse_autonomy(raw: &str) -> Option<Autonomy> {
    match raw {
        "observation" => Some(Autonomy::Observation),
        "assisted" => Some(Autonomy::Assisted),
        "limited" => Some(Autonomy::Limited),
        "advanced" => Some(Autonomy::Advanced),
        _ => None,
    }
}

fn unquote(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.contains('"') {
            return Err("YAML_INVALID");
        }
        Ok(inner.to_owned())
    } else if trimmed.starts_with('"') || trimmed.ends_with('"') {
        Err("YAML_INVALID")
    } else {
        Ok(trimmed.to_owned())
    }
}

fn parse_bool(raw: &str) -> Result<bool, &'static str> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("YAML_INVALID"),
    }
}

/// Exports the core policy, privacy, routing and approval-required actions as schema-validated YAML.
pub fn export_yaml(core: &Core) -> String {
    let policy = core.policy_settings();
    let models = core.agent_models();
    let strategy = core.local_strategy().code();
    let mut out = String::new();
    out.push_str("schema_version: 1\n");
    out.push_str(&format!("execution_mode: {}\n", autonomy_code(policy.autonomy)));
    out.push_str(&format!("simulate: {}\n", policy.simulate));
    out.push_str(&format!("privacy_mode: {}\n", core.private_mode()));
    out.push_str(&format!("local_strategy: {strategy}\n"));
    out.push_str("agent_models:\n");
    for (key, value) in [
        ("office", models.office.as_str()),
        ("windows_admin", models.windows_admin.as_str()),
        ("development", models.development.as_str()),
        ("research", models.research.as_str()),
        ("multimedia", models.multimedia.as_str()),
        ("read_only", models.read_only.as_str()),
        ("planning", models.planning.as_str()),
    ] {
        out.push_str(&format!("  {key}: \"{value}\"\n"));
    }
    out.push_str("require_approval_for:\n");
    let ctx = ActionContext { local_model: true, temporary_grant: false, profile: AgentProfile::General, tools: None };
    for action in AgentAction::ALL {
        if evaluate(policy, action, ctx).effect == Effect::Ask {
            out.push_str(&format!("  - {}\n", action.code()));
        }
    }
    out
}

/// Validates a YAML configuration against the AiWin11 v1 schema and applies it to `core`.
pub fn import_yaml(core: &mut Core, raw: &str) -> Result<(), &'static str> {
    if raw.len() > MAX_YAML_BYTES || raw.chars().any(|c| c.is_control() && c != '\n' && c != '\r') {
        return Err("YAML_INVALID");
    }
    let mut seen_version = false;
    let mut autonomy: Option<Autonomy> = None;
    let mut simulate: Option<bool> = None;
    let mut privacy: Option<bool> = None;
    let mut strategy: Option<LocalStrategy> = None;
    let mut models = AgentModels::default();
    let mut section: Option<&str> = None;

    for line in raw.lines() {
        let line_no_comment = line.split('#').next().unwrap_or("").trim_end();
        if line_no_comment.trim().is_empty() {
            continue;
        }
        if let Some(item) = line_no_comment.strip_prefix("  - ") {
            if section != Some("require_approval_for") {
                return Err("YAML_INVALID");
            }
            let code = unquote(item)?;
            if !AgentAction::ALL.iter().any(|action| action.code() == code) {
                return Err("YAML_UNKNOWN_ACTION");
            }
            continue;
        }
        if line_no_comment.starts_with("  ") {
            if section != Some("agent_models") {
                return Err("YAML_INVALID");
            }
            let (key, value) = line_no_comment.trim().split_once(':').ok_or("YAML_INVALID")?;
            let model = unquote(value)?;
            if model.len() > 120 {
                return Err("YAML_INVALID");
            }
            match key.trim() {
                "office" => models.office = model,
                "windows_admin" => models.windows_admin = model,
                "development" => models.development = model,
                "research" => models.research = model,
                "multimedia" => models.multimedia = model,
                "read_only" => models.read_only = model,
                "planning" => models.planning = model,
                _ => return Err("YAML_UNKNOWN_KEY"),
            }
            continue;
        }
        section = None;
        let (key, value) = line_no_comment.split_once(':').ok_or("YAML_INVALID")?;
        let key = key.trim();
        let value = value.trim();
        match key {
            "schema_version" => {
                if value != "1" {
                    return Err("YAML_UNSUPPORTED_VERSION");
                }
                seen_version = true;
            }
            "execution_mode" => autonomy = Some(parse_autonomy(&unquote(value)?).ok_or("YAML_INVALID")?),
            "simulate" => simulate = Some(parse_bool(value)?),
            "privacy_mode" => privacy = Some(parse_bool(value)?),
            "local_strategy" => {
                strategy = Some(LocalStrategy::from_code(&unquote(value)?).ok_or("YAML_INVALID")?);
            }
            "agent_models" if value.is_empty() => section = Some("agent_models"),
            "require_approval_for" if value.is_empty() => section = Some("require_approval_for"),
            _ => return Err("YAML_UNKNOWN_KEY"),
        }
    }

    if !seen_version {
        return Err("YAML_UNSUPPORTED_VERSION");
    }
    let next_policy =
        PolicySettings { autonomy: autonomy.ok_or("YAML_INVALID")?, simulate: simulate.ok_or("YAML_INVALID")? };
    core.set_policy_settings(next_policy).map_err(|_| "YAML_PERSIST_FAILED")?;
    core.set_private_mode(privacy.ok_or("YAML_INVALID")?)?;
    core.set_local_strategy(strategy.ok_or("YAML_INVALID")?)?;
    core.set_agent_models(models)?;
    Ok(())
}

/// Exports the active security policy, resource guards and constraints as structured JSON.
pub fn export_policy_json(core: &Core) -> String {
    let policy = core.policy_settings();
    let ctx = ActionContext { local_model: true, temporary_grant: false, profile: AgentProfile::General, tools: None };
    let actions: Vec<serde_json::Value> = AgentAction::ALL
        .iter()
        .map(|action| {
            let decision = evaluate(policy, *action, ctx);
            let manifest = crate::manifest::manifest(*action);
            serde_json::json!({
                "action": action.code(),
                "tool": manifest.tool,
                "effect": format!("{:?}", decision.effect).to_lowercase(),
                "rule": decision.rule,
                "risk": format!("{:?}", manifest.risk).to_lowercase(),
                "dryRun": manifest.dry_run,
                "localOnly": manifest.local_only,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schemaVersion": 1,
        "subject": "agent",
        "autonomy": autonomy_code(policy.autonomy),
        "simulate": policy.simulate,
        "privateMode": core.private_mode(),
        "actions": actions,
        "resources": {
            "protectedFiles": core.protected_files(),
            "protectedWindows": core.protected_windows(),
            "blockedApps": core.blocked_applications(),
        },
        "constraints": {
            "maxFilesPerSearch": 20,
            "maxListedWindows": 20,
            "requireBackupBeforeMigration": true,
            "maxGrantSeconds": 3600,
        },
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_config_round_trips_and_validates_schema() {
        let dir = std::env::temp_dir().join(format!("aiwin11-yaml-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut core = Core::open(&dir.join("src.sqlite")).unwrap();
        core.set_policy_settings(PolicySettings { autonomy: Autonomy::Limited, simulate: true }).unwrap();
        core.set_private_mode(true).unwrap();
        core.set_local_strategy(LocalStrategy::Adaptive).unwrap();
        core.set_agent_models(AgentModels { planning: "qwen3:8b".into(), ..AgentModels::default() }).unwrap();

        let exported = export_yaml(&core);
        assert!(exported.contains("schema_version: 1"));
        assert!(exported.contains("execution_mode: limited"));
        assert!(exported.contains("privacy_mode: true"));
        assert!(exported.contains("planning: \"qwen3:8b\""));

        let mut target = Core::open(&dir.join("dst.sqlite")).unwrap();
        import_yaml(&mut target, &exported).unwrap();
        assert_eq!(target.policy_settings(), PolicySettings { autonomy: Autonomy::Limited, simulate: true });
        assert!(target.private_mode());
        assert_eq!(target.local_strategy(), LocalStrategy::Adaptive);
        assert_eq!(target.agent_models().planning, "qwen3:8b");

        assert_eq!(import_yaml(&mut target, "schema_version: 2\n"), Err("YAML_UNSUPPORTED_VERSION"));
        assert_eq!(import_yaml(&mut target, "schema_version: 1\nunknown: 1\n"), Err("YAML_UNKNOWN_KEY"));

        let policy_json = export_policy_json(&target);
        let parsed: serde_json::Value = serde_json::from_str(&policy_json).unwrap();
        assert_eq!(parsed["subject"], "agent");
        assert_eq!(parsed["constraints"]["requireBackupBeforeMigration"], true);
        assert!(parsed["actions"].as_array().is_some_and(|list| list.len() == AgentAction::ALL.len()));
        drop(core);
        drop(target);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
