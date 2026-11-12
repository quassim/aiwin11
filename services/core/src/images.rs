//! Local image generation (ROADMAP R7) through a Stable Diffusion engine the user runs on this
//! PC (AUTOMATIC1111-compatible API on 127.0.0.1). Nothing is sent anywhere else; images are
//! written to AiWin11's own `images` folder, never over an existing file.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use base64::Engine as _;
use reqwest::{blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Core;

const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageBackend {
    #[default]
    Automatic1111,
    ComfyUi,
}

impl ImageBackend {
    fn checkpoint_key(self) -> &'static str {
        match self {
            Self::Automatic1111 => "image.checkpoint",
            Self::ComfyUi => "image.comfy_checkpoint",
        }
    }

    fn valid_checkpoint(self, value: &str) -> bool {
        match self {
            Self::Automatic1111 => valid_checkpoint(value),
            Self::ComfyUi => !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control),
        }
    }
}

/// Only loopback HTTP endpoints, so prompts and images stay on this PC.
pub fn valid_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("http://127.0.0.1:").or_else(|| endpoint.strip_prefix("http://localhost:"))
    else {
        return false;
    };
    rest.trim_end_matches('/').parse::<u16>().is_ok_and(|port| port > 0)
}

impl Core {
    pub fn image_backend(&self) -> ImageBackend {
        if self.setting("image.backend").ok().flatten().as_deref() == Some("comfyUi") {
            ImageBackend::ComfyUi
        } else {
            ImageBackend::Automatic1111
        }
    }

    pub fn image_checkpoint(&self) -> Option<String> {
        let backend = self.image_backend();
        self.setting(backend.checkpoint_key()).ok().flatten().filter(|value| backend.valid_checkpoint(value))
    }

    pub fn set_image_checkpoint(&mut self, checkpoint: &str) -> Result<(), &'static str> {
        let backend = self.image_backend();
        if !checkpoint.is_empty() && !backend.valid_checkpoint(checkpoint) {
            return Err("IMAGE_MODEL_INVALID");
        }
        let session = self.session_id().to_owned();
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [backend.checkpoint_key(), checkpoint],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction
            .execute(
                "INSERT INTO events (id, session_id, kind, source, code)
             VALUES (?1, ?2, 'settings.image.model.changed', 'core', ?3)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    session,
                    if checkpoint.is_empty() { "default" } else { "selected" }
                ],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")
    }
    pub fn image_endpoint(&self) -> Option<String> {
        self.setting("image.endpoint").ok().flatten().filter(|endpoint| valid_endpoint(endpoint))
    }

    pub fn set_image_endpoint(&mut self, endpoint: &str, backend: ImageBackend) -> Result<(), &'static str> {
        let endpoint = endpoint.trim().trim_end_matches('/');
        if !endpoint.is_empty() && !valid_endpoint(endpoint) {
            return Err("IMAGE_ENDPOINT_INVALID");
        }
        let transaction = self.db.transaction().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        for (key, value) in [
            ("image.endpoint", endpoint),
            (
                "image.backend",
                match backend {
                    ImageBackend::Automatic1111 => "automatic1111",
                    ImageBackend::ComfyUi => "comfyUi",
                },
            ),
        ] {
            transaction
                .execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                    [key, value],
                )
                .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        }
        transaction.commit().map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.journal("settings.image.changed", Some(if endpoint.is_empty() { "none" } else { "local" }))
            .map_err(|_| "SETTINGS_UNAVAILABLE")
    }
}

fn client(timeout: Duration) -> Result<Client, &'static str> {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(timeout)
        .build()
        .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")
}

#[derive(Deserialize)]
struct EngineCheckpoint {
    title: String,
    model_name: String,
    sha256: Option<String>,
}

#[derive(Serialize)]
pub struct ImageCheckpoint {
    pub id: String,
    pub label: String,
}

fn valid_checkpoint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn checkpoints(endpoint: &str) -> Result<Vec<EngineCheckpoint>, &'static str> {
    if !valid_endpoint(endpoint) {
        return Err("IMAGE_ENDPOINT_INVALID");
    }
    let reply = client(Duration::from_secs(3))?
        .get(format!("{}/sdapi/v1/sd-models", endpoint.trim_end_matches('/')))
        .send()
        .and_then(|reply| reply.error_for_status())
        .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?;
    let mut bytes = Vec::new();
    reply.take(512 * 1024).read_to_end(&mut bytes).map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?;
    serde_json::from_slice(&bytes).map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")
}

pub fn list_checkpoints(endpoint: &str, backend: ImageBackend) -> Result<Vec<ImageCheckpoint>, &'static str> {
    if backend == ImageBackend::ComfyUi {
        let catalog = comfy_catalog(endpoint, &client(Duration::from_secs(3))?)?;
        return Ok(catalog.into_iter().map(|name| ImageCheckpoint { label: name.clone(), id: name }).collect());
    }
    let mut seen = std::collections::HashSet::new();
    Ok(checkpoints(endpoint)?
        .into_iter()
        .filter_map(|checkpoint| {
            let id = checkpoint.sha256.filter(|id| valid_checkpoint(id))?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let label: String = checkpoint.model_name.chars().filter(|value| !value.is_control()).take(120).collect();
            Some(ImageCheckpoint { id, label })
        })
        .take(128)
        .collect())
}

/// Generates one 512×512 image and writes it as a new PNG in `folder`. Returns its file name.
pub(crate) fn generate(
    endpoint: &str,
    prompt: &str,
    folder: &Path,
    checkpoint: Option<&str>,
    backend: ImageBackend,
) -> Result<String, &'static str> {
    if !valid_endpoint(endpoint) {
        return Err("IMAGE_ENDPOINT_INVALID");
    }
    if backend == ImageBackend::ComfyUi {
        return write_png(folder, &generate_comfy(endpoint.trim_end_matches('/'), prompt, checkpoint)?);
    }
    let mut request = json!({ "prompt": prompt, "steps": 20, "width": 512, "height": 512, "batch_size": 1 });
    if let Some(checkpoint) = checkpoint {
        let title = checkpoints(endpoint)?
            .into_iter()
            .find(|entry| entry.sha256.as_deref() == Some(checkpoint))
            .map(|entry| entry.title)
            .filter(|title| !title.is_empty() && title.len() <= 1024)
            .ok_or("IMAGE_MODEL_UNAVAILABLE")?;
        request["override_settings"] = json!({ "sd_model_checkpoint": title });
        request["override_settings_restore_afterwards"] = json!(true);
    }
    let response = client(GENERATION_TIMEOUT)?
        .post(format!("{endpoint}/sdapi/v1/txt2img"))
        .json(&request)
        .send()
        .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?;
    if !response.status().is_success() {
        return Err("IMAGE_ENGINE_UNAVAILABLE");
    }
    let mut body = Vec::new();
    response.take((MAX_IMAGE_BYTES * 2) as u64).read_to_end(&mut body).map_err(|_| "IMAGE_INVALID")?;
    let reply: Value = serde_json::from_slice(&body).map_err(|_| "IMAGE_INVALID")?;
    let encoded = reply.pointer("/images/0").and_then(Value::as_str).ok_or("IMAGE_INVALID")?;
    let png = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "IMAGE_INVALID")?;
    write_png(folder, &png)
}

fn write_png(folder: &Path, png: &[u8]) -> Result<String, &'static str> {
    if png.len() > MAX_IMAGE_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("IMAGE_INVALID");
    }
    std::fs::create_dir_all(folder).map_err(|_| "IMAGE_WRITE_FAILED")?;
    let name = format!("image-{}.png", uuid::Uuid::new_v4().simple());
    let path: PathBuf = folder.join(&name);
    // create_new: never replaces a file.
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| "IMAGE_WRITE_FAILED")?;
    if std::io::Write::write_all(&mut file, png).is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err("IMAGE_WRITE_FAILED");
    }
    Ok(name)
}

fn bounded_json(response: reqwest::Result<reqwest::blocking::Response>) -> Result<Value, &'static str> {
    let mut body = Vec::new();
    response
        .and_then(|response| response.error_for_status())
        .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?
        .take(512 * 1024)
        .read_to_end(&mut body)
        .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?;
    serde_json::from_slice(&body).map_err(|_| "IMAGE_INVALID")
}

fn comfy_catalog(endpoint: &str, http: &Client) -> Result<Vec<String>, &'static str> {
    if !valid_endpoint(endpoint) {
        return Err("IMAGE_ENDPOINT_INVALID");
    }
    let catalog = bounded_json(
        http.get(format!("{}/object_info/CheckpointLoaderSimple", endpoint.trim_end_matches('/'))).send(),
    )?;
    let names = catalog
        .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
        .and_then(Value::as_array)
        .ok_or("IMAGE_INVALID")?;
    Ok(names
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| ImageBackend::ComfyUi.valid_checkpoint(name))
        .take(100)
        .map(str::to_owned)
        .collect())
}

fn generate_comfy(endpoint: &str, prompt: &str, checkpoint: Option<&str>) -> Result<Vec<u8>, &'static str> {
    let http = client(Duration::from_secs(8))?;
    let checkpoint = checkpoint.ok_or("IMAGE_MODEL_UNAVAILABLE")?;
    if !comfy_catalog(endpoint, &http)?.iter().any(|name| name == checkpoint) {
        return Err("IMAGE_MODEL_UNAVAILABLE");
    }
    let prefix = format!("image-{}", uuid::Uuid::new_v4().simple());
    let graph = json!({
        "1": {"class_type":"CheckpointLoaderSimple","inputs":{"ckpt_name":checkpoint}},
        "2": {"class_type":"CLIPTextEncode","inputs":{"text":prompt,"clip":["1",1]}},
        "3": {"class_type":"CLIPTextEncode","inputs":{"text":"","clip":["1",1]}},
        "4": {"class_type":"EmptyLatentImage","inputs":{"width":512,"height":512,"batch_size":1}},
        "5": {"class_type":"KSampler","inputs":{
            "model":["1",0],"positive":["2",0],"negative":["3",0],"latent_image":["4",0],
            "seed":0,"steps":20,"cfg":7,"sampler_name":"euler","scheduler":"normal","denoise":1}},
        "6": {"class_type":"VAEDecode","inputs":{"samples":["5",0],"vae":["1",2]}},
        "7": {"class_type":"SaveImage","inputs":{"images":["6",0],"filename_prefix":format!("AiWin11/{prefix}")}}
    });
    let queued = bounded_json(http.post(format!("{endpoint}/prompt")).json(&json!({"prompt": graph})).send())?;
    let id = queued
        .get("prompt_id")
        .and_then(Value::as_str)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .ok_or("IMAGE_INVALID")?
        .to_string();
    let started = Instant::now();
    while started.elapsed() < GENERATION_TIMEOUT {
        std::thread::sleep(Duration::from_secs(2));
        let history = bounded_json(http.get(format!("{endpoint}/history/{id}")).send())?;
        let Some(entry) = history.get(&id) else { continue };
        if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
            return Err("IMAGE_GENERATION_FAILED");
        }
        if entry.pointer("/status/completed").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let artifact = entry.pointer("/outputs/7/images/0").ok_or("IMAGE_INVALID")?;
        let filename = artifact
            .get("filename")
            .and_then(Value::as_str)
            .filter(|name| {
                name.starts_with(&prefix)
                    && name.ends_with(".png")
                    && name.len() <= 128
                    && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            })
            .ok_or("IMAGE_INVALID")?;
        if artifact.get("subfolder").and_then(Value::as_str) != Some("AiWin11")
            || artifact.get("type").and_then(Value::as_str) != Some("output")
        {
            return Err("IMAGE_INVALID");
        }
        let mut png = Vec::new();
        http.get(format!("{endpoint}/view"))
            .query(&[("filename", filename), ("subfolder", "AiWin11"), ("type", "output")])
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|_| "IMAGE_ENGINE_UNAVAILABLE")?
            .take(MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut png)
            .map_err(|_| "IMAGE_INVALID")?;
        return Ok(png);
    }
    Err("IMAGE_TIMEOUT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_endpoints_are_accepted() {
        assert!(valid_endpoint("http://127.0.0.1:7860") && valid_endpoint("http://localhost:7860/"));
        for bad in [
            "https://example.com",
            "http://192.168.1.2:7860",
            "http://127.0.0.1",
            "http://127.0.0.1:x",
            "ftp://127.0.0.1:1",
        ] {
            assert!(!valid_endpoint(bad), "{bad}");
        }
        assert!(
            list_checkpoints("http://127.0.0.1:1", ImageBackend::Automatic1111).is_err(),
            "nothing listens on port 1"
        );
    }

    #[test]
    fn checkpoint_ids_are_full_hashes() {
        assert!(valid_checkpoint(&"a1".repeat(32)));
        assert!(!valid_checkpoint("model.safetensors"));
        assert!(!valid_checkpoint(&"g".repeat(64)));
    }
}
