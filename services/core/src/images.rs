//! Local image generation (ROADMAP R7) through a Stable Diffusion engine the user runs on this
//! PC (AUTOMATIC1111-compatible API on 127.0.0.1). Nothing is sent anywhere else; images are
//! written to AiWin11's own `images` folder, never over an existing file.

use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::Engine as _;
use reqwest::{blocking::Client, redirect::Policy};
use serde_json::{json, Value};

use crate::Core;

const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(180);

/// Only loopback HTTP endpoints, so prompts and images stay on this PC.
pub fn valid_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("http://127.0.0.1:").or_else(|| endpoint.strip_prefix("http://localhost:"))
    else {
        return false;
    };
    rest.trim_end_matches('/').parse::<u16>().is_ok_and(|port| port > 0)
}

impl Core {
    pub fn image_endpoint(&self) -> Option<String> {
        self.setting("image.endpoint").ok().flatten().filter(|endpoint| valid_endpoint(endpoint))
    }

    pub fn set_image_endpoint(&mut self, endpoint: &str) -> Result<(), &'static str> {
        let endpoint = endpoint.trim().trim_end_matches('/');
        if !endpoint.is_empty() && !valid_endpoint(endpoint) {
            return Err("IMAGE_ENDPOINT_INVALID");
        }
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('image.endpoint', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [endpoint],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
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

/// True when the engine answers its model list.
pub fn engine_available(endpoint: &str) -> bool {
    valid_endpoint(endpoint)
        && client(Duration::from_secs(3))
            .ok()
            .and_then(|client| client.get(format!("{endpoint}/sdapi/v1/sd-models")).send().ok())
            .is_some_and(|response| response.status().is_success())
}

/// Generates one 512×512 image and writes it as a new PNG in `folder`. Returns its file name.
pub(crate) fn generate(endpoint: &str, prompt: &str, folder: &Path) -> Result<String, &'static str> {
    if !valid_endpoint(endpoint) {
        return Err("IMAGE_ENDPOINT_INVALID");
    }
    let response = client(GENERATION_TIMEOUT)?
        .post(format!("{endpoint}/sdapi/v1/txt2img"))
        .json(&json!({ "prompt": prompt, "steps": 20, "width": 512, "height": 512, "batch_size": 1 }))
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
    if png.len() > MAX_IMAGE_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("IMAGE_INVALID");
    }
    std::fs::create_dir_all(folder).map_err(|_| "IMAGE_WRITE_FAILED")?;
    let name = format!("image-{}.png", uuid::Uuid::new_v4().simple());
    let path: PathBuf = folder.join(&name);
    // create_new: never replaces a file.
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_| "IMAGE_WRITE_FAILED")?;
    std::io::Write::write_all(&mut file, &png).map_err(|_| "IMAGE_WRITE_FAILED")?;
    Ok(name)
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
        assert!(!engine_available("http://127.0.0.1:1"), "nothing listens on port 1");
    }
}
