use std::{io::Read, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, redirect::Policy};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const OLLAMA: &str = "http://127.0.0.1:11434";
const LOCAL_INFERENCE_TIMEOUT: Duration = Duration::from_secs(90);
pub(crate) const NOTEPAD_INSTRUCTION: &str =
    "Ouvre le Bloc-notes en appelant launch_notepad une fois, sans argument.";

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

    pub fn propose_notepad(&self, model: &str) -> Result<Value, GatewayError> {
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
                    "options": { "temperature": 0, "num_predict": 128 },
                    "messages": [{ "role": "user", "content": NOTEPAD_INSTRUCTION }],
                    "tools": [{ "type": "function", "function": {
                        "name": "launch_notepad",
                        "description": "Request opening Windows Notepad",
                        "parameters": { "type": "object", "properties": {}, "additionalProperties": false }
                    }}]
                })),
        )?;
        let location: Location = serde_json::from_value(reply.clone())
            .map_err(|_| GatewayError::InvalidResponse)?;
        if !location.is_local(model) {
            return Err(GatewayError::NonLocalModel);
        }
        let calls = reply.pointer("/message/tool_calls").ok_or(GatewayError::InvalidResponse)?;
        if reply.get("done") != Some(&Value::Bool(true))
            || reply.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
        {
            return Err(GatewayError::InvalidResponse);
        }
        Ok(calls.clone())
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
