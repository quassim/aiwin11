use std::{ffi::c_void, ptr, slice, str, time::Duration};

use reqwest::{blocking::Client, redirect::Policy};
use serde_json::{json, Value};
use windows_sys::Win32::{
    Foundation::ERROR_NOT_FOUND,
    Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    },
};

use crate::model_gateway::{tool_catalog, Ollama, Proposal, AGENT_INSTRUCTIONS};

const TARGET: &str = "AiWin11/OpenAI";
const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

pub fn save_key(key: String) -> Result<(), &'static str> {
    let bytes = key.trim().as_bytes();
    if bytes.is_empty() || bytes.len() > 1024 || !bytes.iter().all(u8::is_ascii_graphic) {
        return Err("INVALID_API_KEY");
    }
    let mut target = target_name();
    let entry = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // CredWriteW reads the borrowed buffers during the call.
    if unsafe { CredWriteW(&entry, 0) } == 0 {
        return Err("CREDENTIAL_STORE_UNAVAILABLE");
    }
    Ok(())
}

pub fn delete_key() -> Result<(), &'static str> {
    let target = target_name();
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
        && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_NOT_FOUND as i32)
    {
        return Err("CREDENTIAL_STORE_UNAVAILABLE");
    }
    Ok(())
}

pub fn has_key() -> Result<bool, &'static str> {
    Ok(read_entry()?.is_some())
}

fn target_name() -> Vec<u16> {
    TARGET.encode_utf16().chain([0]).collect()
}

struct Credential(*mut CREDENTIALW);

impl Drop for Credential {
    fn drop(&mut self) {
        // The credential and its blob are one Win32 allocation.
        unsafe { CredFree(self.0.cast::<c_void>()) };
    }
}

fn read_entry() -> Result<Option<Credential>, &'static str> {
    let target = target_name();
    let mut entry = ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut entry) } == 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
            Ok(None)
        } else {
            Err("CREDENTIAL_STORE_UNAVAILABLE")
        };
    }
    Ok(Some(Credential(entry)))
}

/// Sends the user's request with the tool catalog. Callers must hold a cloud approval.
pub(crate) fn propose(model: &str, prompt: &str) -> Result<Proposal, &'static str> {
    if !model.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte)) {
        return Err("MODEL_REQUIRED");
    }
    let credential = read_entry()?.ok_or("CLOUD_KEY_REQUIRED")?;
    // CredReadW returned a live allocation held by credential until the request is built.
    let entry = unsafe { &*credential.0 };
    if entry.CredentialBlob.is_null() || !(1..=1024).contains(&entry.CredentialBlobSize) {
        return Err("INVALID_API_KEY");
    }
    let blob = unsafe { slice::from_raw_parts(entry.CredentialBlob, entry.CredentialBlobSize as usize) };
    let key = str::from_utf8(blob).map_err(|_| "INVALID_API_KEY")?;
    let client = Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(45))
        .build().map_err(|_| "CLOUD_REQUEST_FAILED")?;
    let request = client.post(ENDPOINT).bearer_auth(key).json(&json!({
        "model": model,
        "store": false,
        "stream": false,
        "max_completion_tokens": 1024,
        "parallel_tool_calls": false,
        "messages": [
            { "role": "system", "content": AGENT_INSTRUCTIONS },
            { "role": "user", "content": prompt }
        ],
        "tools": tool_catalog(true)
    }));
    drop(credential);
    let reply: Value = Ollama::receive(request).map_err(|_| "CLOUD_REQUEST_FAILED")?;
    normalize(&reply)
}

fn normalize(reply: &Value) -> Result<Proposal, &'static str> {
    let invalid = "CLOUD_PROPOSAL_INVALID";
    if reply.pointer("/choices/0/message/role").and_then(Value::as_str) != Some("assistant") {
        return Err(invalid);
    }
    match reply.pointer("/choices/0/finish_reason").and_then(Value::as_str) {
        Some("tool_calls") => {}
        Some("stop") => {
            let text = reply.pointer("/choices/0/message/content").and_then(Value::as_str);
            return Proposal::from_parts(None, text).ok_or(invalid);
        }
        _ => return Err(invalid),
    }
    let calls = reply.pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array).ok_or(invalid)?;
    let [call] = calls.as_slice() else { return Err(invalid) };
    if call.get("type").and_then(Value::as_str) != Some("function") {
        return Err(invalid);
    }
    let name = call.pointer("/function/name").and_then(Value::as_str).ok_or(invalid)?;
    let args = call.pointer("/function/arguments").and_then(Value::as_str).ok_or(invalid)?;
    let arguments: Value = serde_json::from_str(args).map_err(|_| invalid)?;
    Ok(Proposal::Calls(json!([{ "function": { "name": name, "arguments": arguments } }])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_only_one_finished_function_proposal() {
        let call = json!({"type":"function", "function":{
            "name":"launch_application","arguments":"{\"application\":\"notepad\"}"
        }});
        let mut reply = json!({"choices":[{"finish_reason":"tool_calls","message":{
            "role":"assistant","tool_calls":[call.clone()]
        }}]});
        assert_eq!(normalize(&reply).unwrap(), Proposal::Calls(
            json!([{"function":{"name":"launch_application","arguments":{"application":"notepad"}}}])));
        reply["choices"][0]["message"]["tool_calls"] = json!([call.clone(), call]);
        assert_eq!(normalize(&reply), Err("CLOUD_PROPOSAL_INVALID"));
    }

    #[test]
    fn decodes_a_finished_text_answer_and_rejects_truncation() {
        let mut reply = json!({"choices":[{"finish_reason":"stop","message":{
            "role":"assistant","content":" Bonjour "
        }}]});
        assert_eq!(normalize(&reply), Ok(Proposal::Text("Bonjour".into())));
        reply["choices"][0]["finish_reason"] = json!("length");
        assert_eq!(normalize(&reply), Err("CLOUD_PROPOSAL_INVALID"));
        reply["choices"][0]["finish_reason"] = json!("stop");
        reply["choices"][0]["message"]["content"] = json!("");
        assert_eq!(normalize(&reply), Err("CLOUD_PROPOSAL_INVALID"));
    }
}
