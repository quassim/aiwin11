use std::{
    io::Read,
    time::{Duration, Instant},
};

use aiwin11_contracts::v1::command::Operation;
use reqwest::{
    blocking::{Client, Response},
    redirect,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    broker::{Peer, Policy},
    Core,
};

const LOCAL_NODES: &[&str] = &[
    "UNETLoader",
    "CLIPLoader",
    "VAELoader",
    "CLIPTextEncode",
    "ModelSamplingSD3",
    "EmptyHunyuanLatentVideo",
    "KSampler",
    "KSamplerAdvanced",
    "VAEDecode",
    "CreateVideo",
    "SaveVideo",
];
const MAX_JSON: usize = 256 * 1024;
const MAX_REPLY: usize = 512 * 1024;
const WAIT_LIMIT: Duration = Duration::from_secs(20 * 60);

pub struct VideoRequest {
    port: u16,
    graph: Value,
    output_node: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoTicket {
    port: u16,
    id: String,
    output_node: String,
}

impl VideoRequest {
    pub fn new(port: u16, workflow: &str, prompt: &str) -> Result<Self, &'static str> {
        if port == 0
            || workflow.len() > MAX_JSON
            || prompt.trim().is_empty()
            || prompt.chars().count() > 2000
            || prompt.contains('\0')
        {
            return Err("VIDEO_INVALID");
        }
        let mut graph: Value = serde_json::from_str(workflow).map_err(|_| "VIDEO_WORKFLOW_INVALID")?;
        let nodes = graph.as_object_mut().ok_or("VIDEO_WORKFLOW_INVALID")?;
        if !(1..=48).contains(&nodes.len()) {
            return Err("VIDEO_WORKFLOW_INVALID");
        }
        let mut input_found = false;
        let mut output_node = None;
        for (id, node) in nodes {
            if id.is_empty() || id.len() > 8 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("VIDEO_WORKFLOW_INVALID");
            }
            let kind = node.get("class_type").and_then(Value::as_str).ok_or("VIDEO_WORKFLOW_INVALID")?;
            if !LOCAL_NODES.contains(&kind) {
                return Err("VIDEO_NODE_UNSUPPORTED");
            }
            let is_text = kind == "CLIPTextEncode";
            let is_output = kind == "SaveVideo";
            let inputs = node.get_mut("inputs").and_then(Value::as_object_mut).ok_or("VIDEO_WORKFLOW_INVALID")?;
            for (field, max) in [("width", 1024), ("height", 1024), ("length", 121), ("batch_size", 1), ("steps", 50)] {
                if inputs.get(field).is_some_and(|value| !value.as_u64().is_some_and(|size| (1..=max).contains(&size)))
                {
                    return Err("VIDEO_LIMIT_EXCEEDED");
                }
            }
            if is_text && inputs.get("text").and_then(Value::as_str) == Some("{{AIWIN11_PROMPT}}") {
                if input_found {
                    return Err("VIDEO_WORKFLOW_INVALID");
                }
                input_found = true;
                inputs.insert("text".into(), json!(prompt.trim()));
            }
            if is_output {
                if output_node.is_some() {
                    return Err("VIDEO_WORKFLOW_INVALID");
                }
                output_node = Some(id.clone());
                inputs.insert("filename_prefix".into(), json!("AiWin11/video"));
                inputs.insert("format".into(), json!("mp4"));
            }
        }
        let output_node = output_node.filter(|_| input_found).ok_or("VIDEO_WORKFLOW_INVALID")?;
        Ok(Self { port, graph, output_node })
    }

    /// The caller reserves its single job slot and obtains broker authorization before dispatch.
    pub fn submit(self) -> Result<VideoTicket, &'static str> {
        let endpoint = format!("http://127.0.0.1:{}", self.port);
        let submitted = response_json(
            video_client()?.post(format!("{endpoint}/prompt")).json(&json!({ "prompt": self.graph })).send(),
        )?;
        let id = submitted
            .get("prompt_id")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or("VIDEO_WORKFLOW_REJECTED")?;
        Ok(VideoTicket { port: self.port, id: id.to_string(), output_node: self.output_node })
    }
}

fn video_client() -> Result<Client, &'static str> {
    Client::builder()
        .no_proxy()
        .redirect(redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "VIDEO_ENGINE_UNAVAILABLE")
}

impl VideoTicket {
    pub fn follow(self) -> Result<String, &'static str> {
        let id = Uuid::parse_str(&self.id).map_err(|_| "VIDEO_UNAVAILABLE")?.to_string();
        let client = video_client()?;
        let endpoint = format!("http://127.0.0.1:{}/history/{id}", self.port);
        let started = Instant::now();
        let mut poll_failures = 0;
        while started.elapsed() < WAIT_LIMIT {
            std::thread::sleep(Duration::from_secs(2));
            let history = match response_json(client.get(&endpoint).send()) {
                Ok(history) => history,
                Err("VIDEO_ENGINE_UNAVAILABLE") if poll_failures < 2 => {
                    poll_failures += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            poll_failures = 0;
            let Some(entry) = history.get(&id) else { continue };
            if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                return Err("VIDEO_GENERATION_FAILED");
            }
            if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                return output_filename(entry, &self.output_node).ok_or("VIDEO_OUTPUT_MISSING");
            }
        }
        Err("VIDEO_TIMEOUT")
    }
}

impl Core {
    pub fn video_ticket(&self) -> Result<Option<VideoTicket>, &'static str> {
        self.setting("video.active_job")
            .map_err(|_| "VIDEO_UNAVAILABLE")?
            .map(|stored| serde_json::from_str(&stored).map_err(|_| "VIDEO_UNAVAILABLE"))
            .transpose()
    }

    pub fn set_video_ticket(&mut self, ticket: Option<&VideoTicket>) -> Result<(), &'static str> {
        if let Some(ticket) = ticket {
            let value = serde_json::to_string(ticket).map_err(|_| "VIDEO_UNAVAILABLE")?;
            self.db.execute(
                "INSERT INTO settings (key, value) VALUES ('video.active_job', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [value],
            )
        } else {
            self.db.execute("DELETE FROM settings WHERE key = 'video.active_job'", [])
        }
        .map_err(|_| "VIDEO_UNAVAILABLE")?;
        Ok(())
    }

    pub fn reserve_video_submission(&mut self) -> Result<(), &'static str> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "VIDEO_UNAVAILABLE")?;
        let count: u32 = tx
            .query_row(
                "SELECT count(*) FROM events WHERE kind = 'video.submission.reserved'
             AND timestamp >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 day')",
                [],
                |row| row.get(0),
            )
            .map_err(|_| "VIDEO_UNAVAILABLE")?;
        if count >= 4 {
            return Err("VIDEO_QUOTA_EXCEEDED");
        }
        tx.execute(
            "INSERT INTO events (id, session_id, kind, source) VALUES (?1, ?2, 'video.submission.reserved', 'core')",
            rusqlite::params![Uuid::new_v4().to_string(), self.session_id],
        )
        .map_err(|_| "VIDEO_UNAVAILABLE")?;
        tx.commit().map_err(|_| "VIDEO_UNAVAILABLE")
    }

    pub fn authorize_video(&mut self) -> Result<Option<String>, &'static str> {
        let command = self
            .desktop_command(Operation::GenerateLocalVideo(Default::default()))
            .map_err(|_| "VIDEO_NOT_AUTHORIZED")?;
        self.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command).map_err(|_| "VIDEO_NOT_AUTHORIZED")
    }

    pub fn finish_video(
        &mut self,
        task: Option<&str>,
        outcome: &Result<String, &'static str>,
    ) -> Result<(), &'static str> {
        let (kind, code) = match outcome {
            Ok(_) => ("video.completed", "success"),
            Err(code) => ("video.failed", *code),
        };
        self.audit(task, kind, code).map_err(|_| "VIDEO_AUDIT_UNAVAILABLE")
    }
}

fn response_json(response: reqwest::Result<Response>) -> Result<Value, &'static str> {
    let response = response.map_err(|_| "VIDEO_ENGINE_UNAVAILABLE")?;
    if response.status().is_client_error() {
        return Err("VIDEO_WORKFLOW_REJECTED");
    }
    let response = response.error_for_status().map_err(|_| "VIDEO_ENGINE_UNAVAILABLE")?;
    let mut bytes = Vec::new();
    response.take((MAX_REPLY + 1) as u64).read_to_end(&mut bytes).map_err(|_| "VIDEO_ENGINE_UNAVAILABLE")?;
    if bytes.len() > MAX_REPLY {
        return Err("VIDEO_REPLY_INVALID");
    }
    serde_json::from_slice(&bytes).map_err(|_| "VIDEO_REPLY_INVALID")
}

fn output_filename(entry: &Value, node: &str) -> Option<String> {
    let output = entry.get("outputs")?.get(node)?.as_object()?;
    output.values().filter_map(Value::as_array).flatten().find_map(|file| {
        let name = file.get("filename")?.as_str()?;
        let subfolder = file.get("subfolder")?.as_str()?;
        if file.get("type")?.as_str()? != "output"
            || subfolder.replace('\\', "/") != "AiWin11"
            || !name.starts_with("video_")
            || !name.ends_with(".mp4")
            || name.len() > 128
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
        {
            return None;
        }
        Some(format!("AiWin11/{name}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_ticket_and_daily_quota_are_kept_in_the_store() {
        let mut core = Core::start(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let ticket = VideoTicket { port: 8188, id: Uuid::new_v4().to_string(), output_node: "2".into() };
        core.set_video_ticket(Some(&ticket)).unwrap();
        assert_eq!(core.video_ticket().unwrap().unwrap().id, ticket.id);
        core.set_video_ticket(None).unwrap();
        assert!(core.video_ticket().unwrap().is_none());
        for _ in 0..4 {
            core.reserve_video_submission().unwrap();
        }
        assert_eq!(core.reserve_video_submission(), Err("VIDEO_QUOTA_EXCEEDED"));
    }

    #[test]
    fn a_video_workflow_replaces_only_the_prompt_and_scopes_the_output() {
        let workflow = json!({
            "1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{AIWIN11_PROMPT}}"}},
            "2": {"class_type": "SaveVideo", "inputs": {"filename_prefix": "../private"}}
        });
        let request = VideoRequest::new(8188, &workflow.to_string(), "Des nuages").unwrap();
        assert_eq!(request.graph.pointer("/1/inputs/text"), Some(&json!("Des nuages")));
        assert_eq!(request.graph.pointer("/2/inputs/filename_prefix"), Some(&json!("AiWin11/video")));
        let mut remote = workflow;
        remote["1"]["class_type"] = json!("OpenAIVideo");
        assert_eq!(VideoRequest::new(8188, &remote.to_string(), "Des nuages").err(), Some("VIDEO_NODE_UNSUPPORTED"));
    }

    #[test]
    fn output_must_be_the_expected_mp4_in_the_aiwin11_folder() {
        let file = json!({"filename": "video_00001_.mp4", "subfolder": "AiWin11", "type": "output"});
        let mut result = json!({"outputs": {"2": {"images": [file]}}});
        assert_eq!(output_filename(&result, "2").as_deref(), Some("AiWin11/video_00001_.mp4"));
        result["outputs"]["2"]["images"][0]["subfolder"] = json!("../AiWin11");
        assert_eq!(output_filename(&result, "2"), None);
    }
}
