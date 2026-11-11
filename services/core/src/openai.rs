use std::{ffi::c_void, io::BufRead, ptr, slice, str, time::Duration};

use reqwest::{blocking::Client, redirect::Policy};
use serde_json::{json, Value};
use windows_sys::Win32::{
    Foundation::ERROR_NOT_FOUND,
    Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    },
};

use crate::model_gateway::{open_stream, tool_catalog, Proposal, StreamSink, StreamedText};

const INVALID: &str = "CLOUD_PROPOSAL_INVALID";

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

/// Streams the conversation (instructions, request, tool calls and results, in the Chat
/// Completions format) with the tool catalog. Callers must hold a cloud approval.
pub(crate) fn propose(
    model: &str, messages: &[Value], sink: &mut StreamSink<'_>,
) -> Result<Proposal, &'static str> {
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
        "stream": true,
        "max_completion_tokens": 1024,
        "parallel_tool_calls": false,
        "messages": messages,
        // No file or window tools: file contents and screen data never go to the cloud.
        "tools": tool_catalog(true, false, false)
    }));
    drop(credential);
    let stream = open_stream(request).map_err(|_| "CLOUD_REQUEST_FAILED")?;
    fold_stream(stream, sink)
}

/// One streamed tool call, assembled from its fragments.
#[derive(Default)]
struct PartialCall {
    id: String,
    kind: String,
    name: String,
    arguments: String,
}

/// Folds an OpenAI Chat Completions SSE stream into one proposal. Only text deltas reach
/// the sink; at most one tool call (index 0) is accepted, as `parallel_tool_calls` is off.
fn fold_stream(reader: impl BufRead, sink: &mut StreamSink<'_>) -> Result<Proposal, &'static str> {
    let mut text = StreamedText::default();
    let mut call: Option<PartialCall> = None;
    let mut finish: Option<String> = None;
    for line in reader.lines() {
        let line = line.map_err(|_| "CLOUD_REQUEST_FAILED")?;
        // SSE comments, event names and blank separators carry no data.
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
        if data == "[DONE]" {
            return finish_proposal(finish.as_deref(), call, text.as_str());
        }
        let chunk: Value = serde_json::from_str(data).map_err(|_| INVALID)?;
        let mut delta_text = None;
        if let Some(choice) = chunk.pointer("/choices/0") {
            let delta = choice.get("delta");
            if let Some(role) = delta.and_then(|delta| delta.get("role")).and_then(Value::as_str) {
                if role != "assistant" {
                    return Err(INVALID);
                }
            }
            for fragment in delta.and_then(|delta| delta.get("tool_calls")).and_then(Value::as_array).into_iter().flatten() {
                if fragment.get("index").and_then(Value::as_u64) != Some(0) {
                    return Err(INVALID);
                }
                let call = call.get_or_insert_with(PartialCall::default);
                let piece = |pointer| fragment.pointer(pointer).and_then(Value::as_str).unwrap_or("");
                call.id.push_str(piece("/id"));
                call.kind.push_str(piece("/type"));
                call.name.push_str(piece("/function/name"));
                call.arguments.push_str(piece("/function/arguments"));
                if call.arguments.len() > 4096 {
                    return Err(INVALID);
                }
            }
            delta_text = delta.and_then(|delta| delta.get("content")).and_then(Value::as_str)
                .filter(|content| !content.is_empty());
            if let Some(content) = delta_text {
                text.push(content).map_err(|()| INVALID)?;
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                finish = Some(reason.to_owned());
            }
        }
        if !sink(delta_text) {
            return Err("TASK_CANCELLED");
        }
    }
    Err(INVALID)
}

fn finish_proposal(finish: Option<&str>, call: Option<PartialCall>, text: &str) -> Result<Proposal, &'static str> {
    match (finish, call) {
        (Some("tool_calls"), Some(call)) if call.kind == "function" && !call.name.is_empty() => {
            let arguments: Value = serde_json::from_str(&call.arguments).map_err(|_| INVALID)?;
            Ok(Proposal::Calls(json!([{ "id": call.id, "function": { "name": call.name, "arguments": arguments } }])))
        }
        (Some("stop"), None) => Proposal::from_parts(None, Some(text)).ok_or(INVALID),
        _ => Err(INVALID),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(chunks: &[Value]) -> std::io::Cursor<String> {
        let mut body: String = chunks.iter().map(|chunk| format!("data: {chunk}\n\n")).collect();
        body.push_str(": keep-alive\n\ndata: [DONE]\n\n");
        std::io::Cursor::new(body)
    }

    fn delta(delta: Value, finish: Value) -> Value {
        json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    }

    fn fold(chunks: &[Value]) -> (Result<Proposal, &'static str>, Vec<String>) {
        let mut deltas = Vec::new();
        let result = fold_stream(sse(chunks), &mut |text: Option<&str>| {
            deltas.extend(text.map(str::to_owned));
            true
        });
        (result, deltas)
    }

    fn tool_fragment(fields: Value) -> Value {
        let mut fragment = json!({"index":0});
        fragment.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        delta(json!({"tool_calls":[fragment]}), Value::Null)
    }

    #[test]
    fn assembles_one_fragmented_function_call_without_forwarding_it() {
        let (result, deltas) = fold(&[
            delta(json!({"role":"assistant"}), Value::Null),
            tool_fragment(json!({"type":"function","id":"call_1","function":{"name":"launch_application","arguments":""}})),
            tool_fragment(json!({"function":{"arguments":"{\"applica"}})),
            tool_fragment(json!({"function":{"arguments":"tion\":\"notepad\"}"}})),
            delta(json!({}), json!("tool_calls")),
        ]);
        // The call ID is kept: the tool result sent back later must refer to it.
        assert_eq!(result, Ok(Proposal::Calls(
            json!([{"id":"call_1","function":{"name":"launch_application","arguments":{"application":"notepad"}}}]))));
        assert!(deltas.is_empty());
    }

    #[test]
    fn streams_and_folds_a_finished_text_answer() {
        let (result, deltas) = fold(&[
            delta(json!({"role":"assistant","content":""}), Value::Null),
            delta(json!({"content":" Bon"}), Value::Null),
            delta(json!({"content":"jour "}), Value::Null),
            delta(json!({}), json!("stop")),
        ]);
        assert_eq!(result, Ok(Proposal::Text("Bonjour".into())));
        assert_eq!(deltas, [" Bon", "jour "]);
    }

    #[test]
    fn rejects_truncated_parallel_or_unterminated_streams() {
        let second_call = delta(json!({"tool_calls":[{"index":1,"type":"function",
            "function":{"name":"launch_application","arguments":"{}"}}]}), Value::Null);
        for chunks in [
            vec![delta(json!({"content":"Bonjour"}), json!("length"))],
            vec![delta(json!({"content":"   "}), json!("stop"))],
            vec![second_call, delta(json!({}), json!("tool_calls"))],
            vec![delta(json!({"role":"user","content":"x"}), json!("stop"))],
        ] {
            assert_eq!(fold(&chunks).0, Err(INVALID));
        }
        let unterminated = std::io::Cursor::new(format!("data: {}\n\n", delta(json!({"content":"x"}), json!("stop"))));
        assert_eq!(fold_stream(unterminated, &mut |_| true), Err(INVALID));
    }

    #[test]
    fn stops_reading_when_the_sink_cancels() {
        let result = fold_stream(sse(&[
            delta(json!({"content":"a"}), Value::Null),
            delta(json!({}), json!("stop")),
        ]), &mut |_| false);
        assert_eq!(result, Err("TASK_CANCELLED"));
    }
}
