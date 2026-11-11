use std::{io::Read, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const OLLAMA: &str = "http://127.0.0.1:11434";
const LOCAL_INFERENCE_TIMEOUT: Duration = Duration::from_secs(90);
/// Longest model reply forwarded to the agent, in characters.
pub(crate) const MAX_REPLY_CHARS: usize = 4000;
pub(crate) const AGENT_INSTRUCTIONS: &str = "Tu es l’agent d’AiWin11 sous Windows 11. \
Tu disposes uniquement des outils fournis. Si la demande de l’utilisateur correspond à un outil, \
appelle cet outil une seule fois avec des arguments valides. Sinon, réponds brièvement en français \
sans appeler d’outil. Si la demande exige une action qu’aucun outil ne permet, dis-le clairement. \
N’affirme jamais qu’une action a été réalisée.";

/// Tool schemas offered to every provider. The Python agent holds the matching registry.
pub(crate) fn tool_catalog(strict: bool) -> Value {
    let mut function = json!({
        "name": "launch_application",
        "description": "Ouvre une nouvelle fenêtre d’une application Windows autorisée.",
        "parameters": {
            "type": "object",
            "properties": {
                "application": {
                    "type": "string",
                    "enum": ["notepad"],
                    "description": "notepad = Bloc-notes"
                }
            },
            "required": ["application"],
            "additionalProperties": false
        }
    });
    if strict {
        function["strict"] = Value::Bool(true);
    }
    json!([{ "type": "function", "function": function }])
}

/// A provider-neutral proposal: one or more tool calls, or a text answer.
#[derive(Debug, PartialEq)]
pub(crate) enum Proposal {
    Calls(Value),
    Text(String),
}

impl Proposal {
    pub(crate) fn from_parts(calls: Option<&Value>, text: Option<&str>) -> Option<Self> {
        if let Some(calls) = calls.filter(|calls| calls.as_array().is_some_and(|list| !list.is_empty())) {
            return Some(Self::Calls(calls.clone()));
        }
        text.map(str::trim)
            .filter(|text| !text.is_empty() && text.chars().count() <= MAX_REPLY_CHARS)
            .map(|text| Self::Text(text.to_owned()))
    }

    pub(crate) fn to_agent_message(&self) -> Value {
        match self {
            Self::Calls(calls) => json!({ "calls": calls }),
            Self::Text(text) => json!({ "text": text }),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub name: String,
    pub tool_calling_advertised: bool,
    pub vision_advertised: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GatewayError {
    Unavailable,
    InvalidResponse,
    UnknownModel,
    NonLocalModel,
    CloudNotDisabled,
    VisionUnsupported,
}

impl GatewayError {
    pub fn vision_code(&self) -> &'static str {
        match self {
            Self::CloudNotDisabled => "OLLAMA_CLOUD_NOT_DISABLED",
            Self::UnknownModel | Self::NonLocalModel | Self::VisionUnsupported => "VISION_MODEL_REQUIRED",
            _ => "VISION_UNAVAILABLE",
        }
    }
}

pub trait LocalModelProvider {
    fn discover(&self) -> Result<Vec<String>, GatewayError>;
    fn describe(&self, model: &str) -> Result<Model, GatewayError>;
    fn probe_tool_calling(&self, model: &str) -> Result<bool, GatewayError>;
}

pub struct Ollama {
    client: Client,
}

#[derive(Deserialize)]
struct Catalog {
    models: Vec<CatalogEntry>,
}

#[derive(Deserialize)]
struct CatalogEntry {
    name: String,
    #[serde(flatten)]
    location: Location,
}

#[derive(Deserialize)]
struct Details {
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(flatten)]
    location: Location,
}

#[derive(Deserialize)]
struct ProbeReply {
    done: bool,
    message: ProbeMessage,
    #[serde(flatten)]
    location: Location,
}

#[derive(Deserialize)]
struct ProbeMessage {
    role: String,
    #[serde(default)]
    tool_calls: Vec<ProbeCall>,
}

#[derive(Deserialize)]
struct ProbeCall {
    function: ProbeFunction,
}

#[derive(Deserialize)]
struct ProbeFunction {
    name: String,
    arguments: Value,
}

#[derive(Deserialize)]
struct Location {
    #[serde(default)]
    remote_host: String,
    #[serde(default)]
    remote_model: String,
}

impl Location {
    fn is_local(&self, name: &str) -> bool {
        self.remote_host.is_empty()
            && self.remote_model.is_empty()
            && !name.ends_with("-cloud")
            && !name.ends_with(":cloud")
    }
}

impl Ollama {
    fn ensure_cloud_disabled(&self) -> Result<(), GatewayError> {
        let status: Value = Self::receive(self.client.get(format!("{OLLAMA}/api/status")))
            .map_err(|_| GatewayError::CloudNotDisabled)?;
        require_local_mode(&status)
    }

    pub fn vision_models(&self) -> Result<Vec<String>, GatewayError> {
        self.ensure_cloud_disabled()?;
        let mut models = Vec::new();
        for name in self.discover()? {
            match self.details(&name) {
                Ok(model) if model.vision_advertised => models.push(name),
                Ok(_) | Err(GatewayError::NonLocalModel) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(models)
    }

    pub(crate) fn describe_capture(&self, model: &str, jpeg: &[u8]) -> Result<String, GatewayError> {
        if !self.describe(model)?.vision_advertised {
            return Err(GatewayError::VisionUnsupported);
        }
        self.ensure_cloud_disabled()?;
        let reply: Value = Self::receive(
            self.client.post(format!("{OLLAMA}/api/chat"))
                .timeout(LOCAL_INFERENCE_TIMEOUT)
                .json(&json!({
                    "model": model,
                    "stream": false,
                    "think": false,
                    "options": { "temperature": 0, "num_predict": 256 },
                    "messages": [{
                        "role": "user",
                        "content": "Décris brièvement en français ce qui est visible dans cette fenêtre. Reste factuel.",
                        "images": [STANDARD.encode(jpeg)]
                    }]
                })),
        )?;
        let location: Location = serde_json::from_value(reply.clone())
            .map_err(|_| GatewayError::InvalidResponse)?;
        if !location.is_local(model) {
            return Err(GatewayError::NonLocalModel);
        }
        if reply.get("done") != Some(&Value::Bool(true))
            || reply.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
        {
            return Err(GatewayError::InvalidResponse);
        }
        reply.pointer("/message/content").and_then(Value::as_str).map(str::trim)
            .filter(|text| !text.is_empty() && text.len() <= 8192)
            .map(str::to_owned).ok_or(GatewayError::InvalidResponse)
    }

    fn details(&self, model: &str) -> Result<Model, GatewayError> {
        let details = Self::receive(
            self.client.post(format!("{OLLAMA}/api/show")).json(&json!({ "model": model })),
        )?;
        describe(model, details)
    }

    pub fn connect() -> Result<Self, GatewayError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| GatewayError::Unavailable)?;
        Ok(Self { client })
    }

    /// Sends the user's request with the tool catalog to a local model.
    pub(crate) fn propose(&self, model: &str, prompt: &str) -> Result<Proposal, GatewayError> {
        if !self.describe(model)?.tool_calling_advertised {
            return Err(GatewayError::InvalidResponse);
        }
        let reply: Value = Self::receive(
            self.client
                .post(format!("{OLLAMA}/api/chat"))
                .timeout(LOCAL_INFERENCE_TIMEOUT)
                .json(&json!({
                    "model": model,
                    "stream": false,
                    "think": false,
                    "options": { "temperature": 0, "num_predict": 512 },
                    "messages": [
                        { "role": "system", "content": AGENT_INSTRUCTIONS },
                        { "role": "user", "content": prompt }
                    ],
                    "tools": tool_catalog(false)
                })),
        )?;
        ollama_proposal(model, &reply)
    }

    pub(crate) fn receive<T: DeserializeOwned>(
        request: reqwest::blocking::RequestBuilder,
    ) -> Result<T, GatewayError> {
        let response = request.send().map_err(|_| GatewayError::Unavailable)?;
        if !response.status().is_success() {
            return Err(GatewayError::Unavailable);
        }

        let mut body = Vec::new();
        response
            .take((MAX_CATALOG_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| GatewayError::InvalidResponse)?;
        if body.len() > MAX_CATALOG_BYTES {
            return Err(GatewayError::InvalidResponse);
        }
        serde_json::from_slice(&body).map_err(|_| GatewayError::InvalidResponse)
    }
}

impl LocalModelProvider for Ollama {
    fn discover(&self) -> Result<Vec<String>, GatewayError> {
        let catalog: Catalog = Self::receive(self.client.get(format!("{OLLAMA}/api/tags")))?;
        Ok(local_names(catalog))
    }

    fn describe(&self, model: &str) -> Result<Model, GatewayError> {
        if !self.discover()?.iter().any(|name| name == model) {
            return Err(GatewayError::UnknownModel);
        }
        self.details(model)
    }

    fn probe_tool_calling(&self, model: &str) -> Result<bool, GatewayError> {
        if !self.describe(model)?.tool_calling_advertised {
            return Ok(false);
        }
        let reply = Self::receive(
            self.client
                .post(format!("{OLLAMA}/api/chat"))
                .timeout(LOCAL_INFERENCE_TIMEOUT)
                .json(&json!({
                    "model": model,
                    "stream": false,
                    "think": false,
                    "options": { "temperature": 0, "num_predict": 128 },
                    "messages": [{
                        "role": "user",
                        "content": "Call the capability_probe tool once with no arguments."
                    }],
                    "tools": [{
                        "type": "function",
                        "function": {
                            "name": "capability_probe",
                            "description": "A synthetic test. This tool performs no action.",
                            "parameters": { "type": "object", "properties": {}, "additionalProperties": false }
                        }
                    }]
                })),
        )?;
        verify_probe(model, reply)
    }
}

fn ollama_proposal(model: &str, reply: &Value) -> Result<Proposal, GatewayError> {
    let location: Location = serde_json::from_value(reply.clone())
        .map_err(|_| GatewayError::InvalidResponse)?;
    if !location.is_local(model) {
        return Err(GatewayError::NonLocalModel);
    }
    if reply.get("done") != Some(&Value::Bool(true))
        || reply.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
    {
        return Err(GatewayError::InvalidResponse);
    }
    Proposal::from_parts(
        reply.pointer("/message/tool_calls"),
        reply.pointer("/message/content").and_then(Value::as_str),
    ).ok_or(GatewayError::InvalidResponse)
}

fn verify_probe(model: &str, reply: ProbeReply) -> Result<bool, GatewayError> {
    if !reply.location.is_local(model) {
        return Err(GatewayError::NonLocalModel);
    }
    if !reply.done || reply.message.role != "assistant" {
        return Err(GatewayError::InvalidResponse);
    }
    Ok(matches!(reply.message.tool_calls.as_slice(), [call]
        if call.function.name == "capability_probe"
            && call.function.arguments.as_object().is_some_and(serde_json::Map::is_empty)))
}

fn local_names(catalog: Catalog) -> Vec<String> {
    let mut names: Vec<_> = catalog
        .models
        .into_iter()
        .filter(|entry| !entry.name.is_empty() && entry.location.is_local(&entry.name))
        .map(|entry| entry.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

fn require_local_mode(status: &Value) -> Result<(), GatewayError> {
    if status.pointer("/cloud/disabled") == Some(&Value::Bool(true)) {
        Ok(())
    } else {
        Err(GatewayError::CloudNotDisabled)
    }
}

fn describe(model: &str, details: Details) -> Result<Model, GatewayError> {
    if !details.location.is_local(model) {
        return Err(GatewayError::NonLocalModel);
    }
    Ok(Model {
        name: model.into(),
        tool_calling_advertised: details.capabilities.iter().any(|item| item == "tools"),
        vision_advertised: details.capabilities.iter().any(|item| item == "vision"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_requires_explicit_cloud_disabled_status() {
        assert_eq!(require_local_mode(&json!({"cloud":{"disabled":true}})), Ok(()));
        for status in [json!({}), json!({"cloud":{"disabled":false}}), json!({"cloud":{"disabled":"true"}})] {
            assert_eq!(require_local_mode(&status), Err(GatewayError::CloudNotDisabled));
        }
    }

    #[test]
    fn excludes_remote_aliases_and_cloud_names_from_the_local_catalog() {
        let catalog = serde_json::from_value(json!({ "models": [
            { "name": "local:latest" },
            { "name": "remote:latest", "remote_model": "upstream" },
            { "name": "alias:latest", "remote_host": "https://example.test" },
            { "name": "model-cloud" },
            { "name": "model:cloud" }
        ]})).unwrap();
        assert_eq!(local_names(catalog), ["local:latest"]);
    }

    #[test]
    fn checks_remote_metadata_again_when_describing_a_model() {
        let details = serde_json::from_value(json!({
            "capabilities": ["tools"],
            "remote_host": "https://example.test"
        })).unwrap();
        assert_eq!(describe("local:latest", details), Err(GatewayError::NonLocalModel));
    }

    #[test]
    fn capability_metadata_distinguishes_missing_support_from_advertised_support() {
        let details = serde_json::from_value(json!({ "capabilities": ["tools"] })).unwrap();
        let model = describe("local:latest", details).unwrap();
        assert!(model.tool_calling_advertised);
        assert!(!model.vision_advertised);
    }

    #[test]
    fn ollama_proposals_prefer_tool_calls_and_bound_text() {
        let call = json!([{ "function": { "name": "launch_application", "arguments": { "application": "notepad" } } }]);
        let reply = |calls: Value, content: Value| json!({
            "done": true,
            "message": { "role": "assistant", "tool_calls": calls, "content": content }
        });
        assert_eq!(
            ollama_proposal("local:latest", &reply(call.clone(), json!(""))),
            Ok(Proposal::Calls(call.clone()))
        );
        assert_eq!(
            ollama_proposal("local:latest", &reply(json!([]), json!("  Bonjour  "))),
            Ok(Proposal::Text("Bonjour".into()))
        );
        for invalid in [
            reply(json!([]), json!("   ")),
            reply(json!(null), json!("é".repeat(MAX_REPLY_CHARS + 1))),
            json!({ "done": false, "message": { "role": "assistant", "content": "x" } }),
            json!({ "done": true, "message": { "role": "user", "content": "x" } }),
        ] {
            assert_eq!(ollama_proposal("local:latest", &invalid), Err(GatewayError::InvalidResponse));
        }
        let mut remote = reply(call, json!(""));
        remote["remote_host"] = json!("https://example.test");
        assert_eq!(ollama_proposal("local:latest", &remote), Err(GatewayError::NonLocalModel));
    }

    #[test]
    fn tool_catalog_offers_only_the_registered_application() {
        let catalog = tool_catalog(true);
        assert_eq!(catalog.as_array().unwrap().len(), 1);
        assert_eq!(catalog[0]["function"]["name"], "launch_application");
        assert_eq!(catalog[0]["function"]["parameters"]["properties"]["application"]["enum"], json!(["notepad"]));
        assert_eq!(catalog[0]["function"]["strict"], true);
        assert!(tool_catalog(false)[0]["function"].get("strict").is_none());
    }

    #[test]
    fn verifies_only_the_expected_structured_tool_call() {
        let reply = json!({
            "done": true,
            "message": {
                "role": "assistant",
                "tool_calls": [{ "function": { "name": "capability_probe", "arguments": {} } }]
            }
        });
        assert_eq!(
            verify_probe("local:latest", serde_json::from_value(reply.clone()).unwrap()),
            Ok(true)
        );
        let mut invalid = reply;
        invalid["message"]["tool_calls"][0]["function"]["arguments"] = json!({ "extra": true });
        assert_eq!(
            verify_probe("local:latest", serde_json::from_value(invalid).unwrap()),
            Ok(false)
        );
    }
}
