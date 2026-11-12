//! Minimal MCP client (ROADMAP R6): stdio servers the user declared, JSON-RPC 2.0, one
//! process per call (initialize, then `tools/list` or `tools/call`), then the process is
//! stopped. An MCP server's own authorization never replaces an AiWin11 permission: the agent
//! reaches it only through the policy engine, the user's approval and the broker.

use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command as Process, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Core;

pub const MAX_SERVERS: usize = 5;
const MAX_LINE_BYTES: u64 = 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const PROTOCOL_VERSION: &str = "2025-06-18";

/// One server declared by the user: a fixed executable and fixed arguments, no shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

pub fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b))
}

impl McpServer {
    fn validated(self) -> Result<Self, &'static str> {
        let command = Path::new(&self.command);
        let ok = valid_name(&self.name)
            && command.is_absolute()
            && command.is_file()
            && self.args.len() <= 16
            && self.args.iter().all(|arg| arg.len() <= 512 && !arg.contains('\0'));
        ok.then_some(self).ok_or("MCP_SERVER_INVALID")
    }
}

impl Core {
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        self.setting("mcp.servers")
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<Vec<McpServer>>(&json).ok())
            .unwrap_or_default()
    }

    /// Replaces the declared servers (at most 5, unique names, absolute existing executables).
    pub fn set_mcp_servers(&mut self, servers: Vec<McpServer>) -> Result<(), &'static str> {
        if servers.len() > MAX_SERVERS {
            return Err("MCP_SERVER_INVALID");
        }
        let servers: Vec<McpServer> = servers.into_iter().map(McpServer::validated).collect::<Result<_, _>>()?;
        let mut names: Vec<&str> = servers.iter().map(|server| server.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != servers.len() {
            return Err("MCP_SERVER_INVALID");
        }
        let json = serde_json::to_string(&servers).map_err(|_| "MCP_SERVER_INVALID")?;
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES ('mcp.servers', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [json],
            )
            .map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.journal("settings.mcp.changed", None).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    pub(crate) fn mcp_server(&self, name: &str) -> Option<McpServer> {
        self.mcp_servers().into_iter().find(|server| server.name == name)
    }
}

/// Starts the server, initializes the session, sends one request and returns its result.
pub(crate) fn request(server: &McpServer, method: &str, params: Value) -> Result<Value, &'static str> {
    let mut process = Process::new(&server.command);
    process.args(&server.args).env_clear();
    for key in ["SystemRoot", "PATH", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            process.env(key, value);
        }
    }
    let mut child = process
        .creation_flags(0x0800_0000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "MCP_SERVER_UNAVAILABLE")?;
    let mut stdin = child.stdin.take().ok_or("MCP_SERVER_UNAVAILABLE")?;
    let stdout = child.stdout.take().ok_or("MCP_SERVER_UNAVAILABLE")?;
    let (sender, receiver) = mpsc::channel::<Value>();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout.take(64 * MAX_LINE_BYTES));
        loop {
            let mut line = String::new();
            match (&mut reader).take(MAX_LINE_BYTES).read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(message) = serde_json::from_str::<Value>(line.trim()) {
                        if sender.send(message).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });
    let deadline = std::time::Instant::now() + CALL_TIMEOUT;
    let mut send = |message: Value| -> Result<(), &'static str> {
        let mut bytes = serde_json::to_vec(&message).map_err(|_| "MCP_PROTOCOL_ERROR")?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).and_then(|()| stdin.flush()).map_err(|_| "MCP_SERVER_UNAVAILABLE")
    };
    let answer = |id: u64| -> Result<Value, &'static str> {
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let message = receiver.recv_timeout(remaining).map_err(|_| "MCP_TIMEOUT")?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // Notifications and server requests are ignored.
            }
            if message.get("error").is_some() {
                return Err("MCP_TOOL_ERROR");
            }
            return message.get("result").cloned().ok_or("MCP_PROTOCOL_ERROR");
        }
    };
    let outcome = (|| {
        send(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "AiWin11", "version": env!("CARGO_PKG_VERSION") }
            }
        }))?;
        answer(1)?;
        send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        send(json!({ "jsonrpc": "2.0", "id": 2, "method": method, "params": params }))?;
        answer(2)
    })();
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// Tool names and descriptions of one server, bounded, for the model.
pub(crate) fn list_tools(server: &McpServer) -> Result<String, &'static str> {
    let result = request(server, "tools/list", json!({}))?;
    let tools = result.get("tools").and_then(Value::as_array).ok_or("MCP_PROTOCOL_ERROR")?;
    let lines: Vec<String> = tools
        .iter()
        .take(50)
        .filter_map(|tool| {
            let name = tool.get("name").and_then(Value::as_str).filter(|name| valid_tool_name(name))?;
            let description: String =
                tool.get("description").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
            let schema: String =
                tool.get("inputSchema").map(Value::to_string).unwrap_or_default().chars().take(400).collect();
            Some(format!("- {name} : {description} (paramètres : {schema})"))
        })
        .collect();
    Ok(lines.join("\n"))
}

/// Calls one tool and returns its text content, bounded.
pub(crate) fn call_tool(server: &McpServer, tool: &str, arguments: &Value) -> Result<String, &'static str> {
    let result = request(server, "tools/call", json!({ "name": tool, "arguments": arguments }))?;
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| content.iter().filter_map(|item| item.get("text").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let text: String = texts.join("\n").chars().take(8000).collect();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err("MCP_TOOL_ERROR");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn servers_are_validated_before_being_stored() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let system = std::env::var("SystemRoot").unwrap();
        let ok = McpServer { name: "demo".into(), command: format!(r"{system}\System32\where.exe"), args: vec![] };
        core.set_mcp_servers(vec![ok.clone()]).unwrap();
        assert_eq!(core.mcp_server("demo"), Some(ok.clone()));
        for bad in [
            McpServer { name: "bad name".into(), ..ok.clone() },
            McpServer { command: "where.exe".into(), ..ok.clone() },
            McpServer { command: r"C:\missing\server.exe".into(), ..ok.clone() },
        ] {
            assert_eq!(core.set_mcp_servers(vec![bad]), Err("MCP_SERVER_INVALID"));
        }
        assert_eq!(core.set_mcp_servers(vec![ok.clone(), ok]), Err("MCP_SERVER_INVALID"), "unique names");
        assert!(valid_tool_name("read_file") && !valid_tool_name("a b"));
    }

    #[test]
    fn a_server_that_is_not_mcp_fails_cleanly() {
        let system = std::env::var("SystemRoot").unwrap();
        let server = McpServer { name: "x".into(), command: format!(r"{system}\System32\where.exe"), args: vec![] };
        assert!(list_tools(&server).is_err());
    }
}
