//! Minimal MCP client (ROADMAP R6): stdio servers the user declared, JSON-RPC 2.0, one
//! process per call, then the process is stopped. Dual-era (MCP 2026-07-28, "Versioning and
//! Compatibility"): `server/discover` is sent first and a modern server gets the protocol
//! version and client capabilities in each request's `_meta`; any other answer, or none within
//! a few seconds, means a legacy server and the `initialize` handshake (2025-11-25 and earlier).
//! An MCP server's own authorization never replaces an AiWin11 permission: the agent reaches it
//! only through the policy engine, the user's approval and the broker. The executable (and any
//! file passed as argument) is fingerprinted when the user declares it: after an update, the
//! server is refused until the user accepts the new version.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command as Process, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Core;

pub const MAX_SERVERS: usize = 5;
const MAX_LINE_BYTES: u64 = 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// A legacy server stays silent or errs on `server/discover`: this long, then `initialize`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MODERN_VERSION: &str = "2026-07-28";
const LEGACY_VERSION: &str = "2025-11-25";
const LEGACY_ACCEPTED: [&str; 4] = [LEGACY_VERSION, "2025-06-18", "2025-03-26", "2024-11-05"];
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// What a test of a server found, for the Extensions page. Names are self-reported by the
/// server: shown, never trusted.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProbe {
    pub protocol_version: String,
    pub server_name: String,
    pub server_version: String,
    pub tools: Vec<String>,
}

/// Whether a declared server may run as it is on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerState {
    Ready,
    Disabled,
    /// The executable or a file argument changed since the user declared or accepted it.
    Changed,
}

/// Size and last write time of a file, `None` if it is not a readable file.
fn file_stamp(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("{}:{modified}", metadata.len()))
}

/// Fingerprint of what the server runs: its executable and every argument naming an existing
/// file (a script run by an interpreter).
pub fn fingerprint(server: &McpServer) -> Option<String> {
    let mut parts = vec![file_stamp(Path::new(&server.command))?];
    for arg in &server.args {
        let path = Path::new(arg);
        if path.is_absolute() {
            if let Some(stamp) = file_stamp(path) {
                parts.push(stamp);
            }
        }
    }
    Some(parts.join("|"))
}

/// One server declared by the user: a fixed executable and fixed arguments, no shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
}

fn enabled_by_default() -> bool {
    true
}

pub fn valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

pub fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b))
}

impl McpServer {
    pub(crate) fn permits(&self, tool: &str) -> bool {
        self.allowed_tools.as_ref().is_none_or(|tools| tools.iter().any(|allowed| allowed == tool))
    }

    fn validated(self) -> Result<Self, &'static str> {
        let command = Path::new(&self.command);
        let ok = valid_name(&self.name)
            && command.is_absolute()
            && command.is_file()
            && command.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            && self.args.len() <= 16
            && self.args.iter().all(|arg| arg.len() <= 512 && !arg.contains('\0'))
            && self
                .allowed_tools
                .as_ref()
                .is_none_or(|tools| tools.len() <= 64 && tools.iter().all(|tool| valid_tool_name(tool)));
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
        // Declaring a server approves what is on disk now; an unchanged declaration keeps the
        // version the user approved earlier, so saving never accepts an update silently.
        let previous = self.mcp_servers();
        let known = self.mcp_fingerprints();
        let mut fingerprints = BTreeMap::new();
        for server in &servers {
            let unchanged = previous
                .iter()
                .any(|old| old.name == server.name && old.command == server.command && old.args == server.args);
            let stamp = match known.get(&server.name) {
                Some(stamp) if unchanged => Some(stamp.clone()),
                _ => fingerprint(server),
            };
            fingerprints.insert(server.name.clone(), stamp.ok_or("MCP_SERVER_INVALID")?);
        }
        let json = serde_json::to_string(&servers).map_err(|_| "MCP_SERVER_INVALID")?;
        self.store_mcp_setting("mcp.servers", &json)?;
        self.store_mcp_fingerprints(&fingerprints)?;
        self.journal("settings.mcp.changed", None).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    fn store_mcp_setting(&self, key: &str, value: &str) -> Result<(), &'static str> {
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                [key, value],
            )
            .map(|_| ())
            .map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    fn mcp_fingerprints(&self) -> BTreeMap<String, String> {
        self.setting("mcp.fingerprints")
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    fn store_mcp_fingerprints(&self, fingerprints: &BTreeMap<String, String>) -> Result<(), &'static str> {
        let json = serde_json::to_string(fingerprints).map_err(|_| "SETTINGS_UNAVAILABLE")?;
        self.store_mcp_setting("mcp.fingerprints", &json)
    }

    /// State of every declared server, by name.
    pub fn mcp_server_states(&self) -> BTreeMap<String, ServerState> {
        let known = self.mcp_fingerprints();
        self.mcp_servers()
            .into_iter()
            .map(|server| {
                let state = if !server.enabled {
                    ServerState::Disabled
                } else if fingerprint(&server).is_some_and(|stamp| known.get(&server.name) == Some(&stamp)) {
                    ServerState::Ready
                } else {
                    ServerState::Changed
                };
                (server.name, state)
            })
            .collect()
    }

    /// The user accepts the version now on disk of a server whose files changed (an update).
    pub fn accept_mcp_update(&mut self, name: &str) -> Result<(), &'static str> {
        let server = self.mcp_servers().into_iter().find(|server| server.name == name).ok_or("MCP_SERVER_UNKNOWN")?;
        let stamp = fingerprint(&server).ok_or("MCP_SERVER_INVALID")?;
        let mut fingerprints = self.mcp_fingerprints();
        fingerprints.insert(server.name, stamp);
        self.store_mcp_fingerprints(&fingerprints)?;
        self.journal("settings.mcp.update.accepted", None).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// A declared server, if it is enabled and still the version the user approved.
    pub(crate) fn mcp_server(&self, name: &str) -> Result<McpServer, &'static str> {
        let server = self
            .mcp_servers()
            .into_iter()
            .find(|server| server.enabled && server.name == name)
            .ok_or("MCP_SERVER_UNKNOWN")?;
        match self.mcp_server_states().get(name) {
            Some(ServerState::Ready) => Ok(server),
            _ => Err("MCP_SERVER_CHANGED"),
        }
    }
}

fn client_info() -> Value {
    json!({ "name": "AiWin11", "version": env!("CARGO_PKG_VERSION") })
}

/// Per-request protocol fields of a modern request.
fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
        "io.modelcontextprotocol/clientInfo": client_info(),
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy,
}

/// One open conversation with a server process.
struct Connection {
    writer: Box<dyn Write + Send>,
    replies: mpsc::Receiver<Value>,
    next_id: u64,
    deadline: Instant,
    era: Era,
    version: String,
    server_info: Value,
}

impl Connection {
    fn send(&mut self, message: &Value) -> Result<(), &'static str> {
        let mut bytes = serde_json::to_vec(message).map_err(|_| "MCP_PROTOCOL_ERROR")?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes).and_then(|()| self.writer.flush()).map_err(|_| "MCP_SERVER_UNAVAILABLE")
    }

    /// Sends a request and waits for its answer: the result, or the peer's error object.
    fn exchange(
        &mut self,
        method: &str,
        params: Value,
        limit: Option<Duration>,
    ) -> Result<Result<Value, Value>, &'static str> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let until = limit.map_or(self.deadline, |limit| (Instant::now() + limit).min(self.deadline));
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            let message = self.replies.recv_timeout(remaining).map_err(|_| "MCP_TIMEOUT")?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // Notifications, server requests and late answers are ignored.
            }
            if let Some(error) = message.get("error") {
                return Ok(Err(error.clone()));
            }
            return message.get("result").cloned().map(Ok).ok_or("MCP_PROTOCOL_ERROR");
        }
    }

    /// Opens the conversation: `server/discover` (modern), else the `initialize` handshake.
    fn open(
        writer: Box<dyn Write + Send>,
        replies: mpsc::Receiver<Value>,
        deadline: Instant,
    ) -> Result<Self, &'static str> {
        let mut connection = Self {
            writer,
            replies,
            next_id: 1,
            deadline,
            era: Era::Legacy,
            version: String::new(),
            server_info: Value::Null,
        };
        match connection.exchange("server/discover", json!({ "_meta": modern_meta() }), Some(PROBE_TIMEOUT)) {
            Ok(Ok(result)) => {
                let supported = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .is_some_and(|versions| versions.iter().any(|version| version == MODERN_VERSION));
                if !supported {
                    return Err("MCP_VERSION_UNSUPPORTED");
                }
                if !result.pointer("/capabilities/tools").is_some_and(Value::is_object) {
                    return Err("MCP_PROTOCOL_ERROR");
                }
                connection.era = Era::Modern;
                connection.version = MODERN_VERSION.into();
                connection.server_info =
                    result.pointer("/_meta/io.modelcontextprotocol~1serverInfo").cloned().unwrap_or_default();
                return Ok(connection);
            }
            // A modern server that does not speak our modern version: no legacy fallback.
            Ok(Err(error)) if error.get("code").and_then(Value::as_i64) == Some(UNSUPPORTED_PROTOCOL_VERSION) => {
                return Err("MCP_VERSION_UNSUPPORTED");
            }
            Ok(Err(_)) | Err("MCP_TIMEOUT") => {}
            Err(code) => return Err(code),
        }
        let params = json!({ "protocolVersion": LEGACY_VERSION, "capabilities": {}, "clientInfo": client_info() });
        let initialized = connection.exchange("initialize", params, None)?.map_err(|_| "MCP_PROTOCOL_ERROR")?;
        let version = initialized.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
        if !LEGACY_ACCEPTED.contains(&version) {
            return Err("MCP_VERSION_UNSUPPORTED");
        }
        if !initialized.pointer("/capabilities/tools").is_some_and(Value::is_object) {
            return Err("MCP_PROTOCOL_ERROR");
        }
        connection.version = version.to_owned();
        connection.server_info = initialized.get("serverInfo").cloned().unwrap_or_default();
        connection.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(connection)
    }

    /// One request in the server's era. A server error is reported as a tool error.
    fn request(&mut self, method: &str, mut params: Value) -> Result<Value, &'static str> {
        if self.era == Era::Modern {
            params["_meta"] = modern_meta();
        }
        let result = self.exchange(method, params, None)?.map_err(|_| "MCP_TOOL_ERROR")?;
        // Absent for legacy servers: complete. Anything else needs input AiWin11 never gives.
        match result.get("resultType").and_then(Value::as_str) {
            None | Some("complete") => Ok(result),
            Some("input_required") => Err("MCP_INPUT_REQUIRED"),
            Some(_) => Err("MCP_PROTOCOL_ERROR"),
        }
    }
}

/// Starts the server, opens the conversation, runs `work`, then stops the process.
fn with_server<T>(
    server: &McpServer,
    work: impl FnOnce(&mut Connection) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    server.clone().validated()?;
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
    let outcome = (|| {
        let stdin = child.stdin.take().ok_or("MCP_SERVER_UNAVAILABLE")?;
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
        let mut connection = Connection::open(Box::new(stdin), receiver, Instant::now() + CALL_TIMEOUT)?;
        work(&mut connection)
    })();
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// The server's tools, bounded: (name, description, input schema), allowed tools only.
fn tools_of(connection: &mut Connection, server: &McpServer) -> Result<Vec<(String, String, String)>, &'static str> {
    let result = connection.request("tools/list", json!({}))?;
    let tools = result.get("tools").and_then(Value::as_array).ok_or("MCP_PROTOCOL_ERROR")?;
    Ok(tools
        .iter()
        .take(50)
        .filter_map(|tool| {
            let name = tool.get("name").and_then(Value::as_str).filter(|name| valid_tool_name(name))?;
            if !server.permits(name) {
                return None;
            }
            let description: String =
                tool.get("description").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
            let schema: String =
                tool.get("inputSchema").map(Value::to_string).unwrap_or_default().chars().take(400).collect();
            Some((name.to_owned(), description, schema))
        })
        .collect())
}

fn format_tools(tools: &[(String, String, String)]) -> String {
    let lines: Vec<String> = tools
        .iter()
        .map(|(name, description, schema)| format!("- {name} : {description} (paramètres : {schema})"))
        .collect();
    lines.join("\n")
}

/// Tool names and descriptions of one server, bounded, for the model.
pub(crate) fn list_tools(server: &McpServer) -> Result<String, &'static str> {
    with_server(server, |connection| tools_of(connection, server).map(|tools| format_tools(&tools)))
}

fn tool_text(result: &Value) -> Result<String, &'static str> {
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| content.iter().filter_map(|item| item.get("text").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err("MCP_TOOL_ERROR");
    }
    Ok(texts.join("\n").chars().take(8000).collect())
}

/// Calls one tool and returns its text content, bounded.
pub(crate) fn call_tool(server: &McpServer, tool: &str, arguments: &Value) -> Result<String, &'static str> {
    if !server.enabled || !server.permits(tool) {
        return Err("MCP_TOOL_DISABLED");
    }
    with_server(server, |connection| {
        tool_text(&connection.request("tools/call", json!({ "name": tool, "arguments": arguments }))?)
    })
}

/// Started by the user from the Extensions page: protocol version, identity and tools.
pub fn probe(server: &McpServer) -> Result<ServerProbe, &'static str> {
    with_server(server, |connection| {
        let tools = tools_of(connection, server)?.into_iter().map(|(name, _, _)| name).collect();
        Ok(probe_of(connection, tools))
    })
}

fn probe_of(connection: &Connection, tools: Vec<String>) -> ServerProbe {
    let field = |key: &str| -> String {
        connection.server_info.get(key).and_then(Value::as_str).unwrap_or("").chars().take(64).collect()
    };
    ServerProbe {
        protocol_version: connection.version.clone(),
        server_name: field("name"),
        server_version: field("version"),
        tools,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection as Db;

    #[test]
    fn servers_are_validated_before_being_stored() {
        let mut core = Core::start(Db::open_in_memory().unwrap()).unwrap();
        let system = std::env::var("SystemRoot").unwrap();
        let ok = McpServer {
            name: "demo".into(),
            command: format!(r"{system}\System32\where.exe"),
            args: vec![],
            enabled: true,
            allowed_tools: None,
        };
        core.set_mcp_servers(vec![ok.clone()]).unwrap();
        assert_eq!(core.mcp_server("demo"), Ok(ok.clone()));
        assert_eq!(core.mcp_server_states().get("demo"), Some(&ServerState::Ready));
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

    type Responder = Box<dyn FnMut(&Value) -> Option<Value> + Send>;

    /// A fake server on the other end of a `Connection`: answers each request with `respond`.
    struct FakeServer {
        line: Vec<u8>,
        respond: Responder,
        replies: mpsc::Sender<Value>,
        seen: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
    }

    impl Write for FakeServer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.line.extend_from_slice(bytes);
            while let Some(end) = self.line.iter().position(|byte| *byte == b'\n') {
                let message: Value = serde_json::from_slice(&self.line[..end]).unwrap();
                self.line.drain(..=end);
                self.seen.lock().unwrap().push(message.clone());
                if let Some(result) = (self.respond)(&message) {
                    let _ = self.replies.send(json!({ "jsonrpc": "2.0", "id": message["id"], "result": result }));
                } else if message.get("id").is_some() && message["method"] != "server/discover" {
                    let _ = self.replies.send(json!({ "jsonrpc": "2.0", "id": message["id"], "error": { "code": -32601, "message": "unknown" } }));
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn connect(
        respond: impl FnMut(&Value) -> Option<Value> + Send + 'static,
    ) -> (Result<Connection, &'static str>, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
        let (sender, receiver) = mpsc::channel();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fake = FakeServer { line: Vec::new(), respond: Box::new(respond), replies: sender, seen: seen.clone() };
        (Connection::open(Box::new(fake), receiver, Instant::now() + Duration::from_secs(30)), seen)
    }

    fn server() -> McpServer {
        McpServer { name: "x".into(), command: "x.exe".into(), args: vec![], enabled: true, allowed_tools: None }
    }

    #[test]
    fn a_modern_server_gets_the_version_and_capabilities_in_every_request() {
        let (connection, seen) = connect(|message| match message["method"].as_str() {
            Some("server/discover") => Some(json!({
                "resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": { "tools": {} },
                "_meta": { "io.modelcontextprotocol/serverInfo": { "name": "Démo", "version": "1.2" } }
            })),
            Some("tools/list") => {
                Some(json!({ "resultType": "complete", "tools": [{ "name": "lire" }, { "name": "a b" }] }))
            }
            Some("tools/call") => Some(json!({ "resultType": "input_required" })),
            _ => None,
        });
        let mut connection = connection.unwrap();
        assert_eq!(connection.era, Era::Modern);
        let tools: Vec<String> = tools_of(&mut connection, &server()).unwrap().into_iter().map(|tool| tool.0).collect();
        assert_eq!(tools, ["lire"], "invalid tool names are dropped");
        let probe = probe_of(&connection, tools);
        assert_eq!((probe.protocol_version.as_str(), probe.server_name.as_str()), ("2026-07-28", "Démo"));
        assert_eq!(connection.request("tools/call", json!({ "name": "lire" })), Err("MCP_INPUT_REQUIRED"));
        let seen = seen.lock().unwrap();
        assert!(seen.iter().all(|message| message["method"] != "initialize"), "no handshake");
        for message in seen.iter() {
            assert_eq!(message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"], "2026-07-28");
            assert!(message["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object());
        }
    }

    #[test]
    fn a_legacy_server_falls_back_to_initialize() {
        let (connection, seen) = connect(|message| match message["method"].as_str() {
            Some("initialize") => Some(json!({ "protocolVersion": "2025-06-18", "capabilities": { "tools": {} },
                                               "serverInfo": { "name": "Ancien", "version": "0.9" } })),
            Some("tools/call") => Some(json!({ "content": [{ "type": "text", "text": "bonjour" }] })),
            _ => None,
        });
        let mut connection = connection.unwrap();
        assert_eq!((connection.era, connection.version.as_str()), (Era::Legacy, "2025-06-18"));
        let result = connection.request("tools/call", json!({ "name": "dire", "arguments": {} })).unwrap();
        assert_eq!(tool_text(&result), Ok("bonjour".into()));
        let methods: Vec<String> =
            seen.lock().unwrap().iter().map(|message| message["method"].as_str().unwrap().to_owned()).collect();
        assert_eq!(methods, ["server/discover", "initialize", "notifications/initialized", "tools/call"]);
        assert!(seen.lock().unwrap()[3]["params"].get("_meta").is_none());
    }

    #[test]
    fn unsupported_versions_and_tool_errors_are_reported() {
        let (connection, _) = connect(|message| match message["method"].as_str() {
            Some("server/discover") => {
                Some(json!({ "supportedVersions": ["2099-01-01"], "capabilities": { "tools": {} } }))
            }
            _ => None,
        });
        assert_eq!(connection.err(), Some("MCP_VERSION_UNSUPPORTED"));
        let (connection, _) = connect(|message| match message["method"].as_str() {
            Some("initialize") => Some(json!({ "protocolVersion": "2023-01-01", "capabilities": { "tools": {} } })),
            _ => None,
        });
        assert_eq!(connection.err(), Some("MCP_VERSION_UNSUPPORTED"));
        assert_eq!(tool_text(&json!({ "isError": true, "content": [] })), Err("MCP_TOOL_ERROR"));
    }

    #[test]
    fn an_updated_executable_is_refused_until_the_user_accepts_it() {
        let directory = std::env::temp_dir().join(format!("aiwin11-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let exe = directory.join("serveur.exe");
        let script = directory.join("serveur.js");
        std::fs::write(&exe, "v1").unwrap();
        std::fs::write(&script, "v1").unwrap();
        let declared = McpServer {
            name: "demo".into(),
            command: exe.to_string_lossy().into_owned(),
            args: vec![script.to_string_lossy().into_owned()],
            enabled: true,
            allowed_tools: None,
        };
        let mut core = Core::start(Db::open_in_memory().unwrap()).unwrap();
        core.set_mcp_servers(vec![declared.clone()]).unwrap();
        assert!(core.mcp_server("demo").is_ok());
        // An update of the script (or the executable) is detected, and saving does not accept it.
        std::fs::write(&script, "v2 plus longue").unwrap();
        assert_eq!(core.mcp_server("demo"), Err("MCP_SERVER_CHANGED"));
        core.set_mcp_servers(vec![declared.clone()]).unwrap();
        assert_eq!(core.mcp_server_states().get("demo"), Some(&ServerState::Changed));
        core.accept_mcp_update("demo").unwrap();
        assert!(core.mcp_server("demo").is_ok());
        core.set_mcp_servers(vec![McpServer { enabled: false, ..declared }]).unwrap();
        assert_eq!(core.mcp_server("demo"), Err("MCP_SERVER_UNKNOWN"));
        assert_eq!(core.mcp_server_states().get("demo"), Some(&ServerState::Disabled));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_server_that_is_not_mcp_fails_cleanly() {
        let system = std::env::var("SystemRoot").unwrap();
        let server = McpServer {
            name: "x".into(),
            command: format!(r"{system}\System32\where.exe"),
            args: vec![],
            enabled: true,
            allowed_tools: None,
        };
        assert!(list_tools(&server).is_err());
    }

    #[test]
    fn legacy_mcp_entries_remain_enabled_and_restricted_tools_fail_before_start() {
        let legacy = json!({"name":"demo","command":"unavailable.exe","args":[]});
        let mut server: McpServer = serde_json::from_value(legacy).unwrap();
        assert!(server.enabled && server.permits("read"));
        server.allowed_tools = Some(vec!["read".into()]);
        assert_eq!(call_tool(&server, "write", &json!({})), Err("MCP_TOOL_DISABLED"));
        server.enabled = false;
        assert_eq!(call_tool(&server, "read", &json!({})), Err("MCP_TOOL_DISABLED"));
    }
}
#[test]
#[ignore = "requires the Python runtime used by the local agent"]
fn probes_and_calls_a_read_only_mcp_process_over_stdio() {
    let python = std::env::var("AIWIN11_PYTHON").expect("set AIWIN11_PYTHON");
    let fixture = concat!(
            "import json, sys\n",
            "for line in sys.stdin:\n",
            "    req = json.loads(line)\n",
            "    if 'id' not in req: continue\n",
            "    method = req['method']\n",
            "    if method == 'server/discover':\n",
            "        result = {'resultType':'complete','supportedVersions':['2026-07-28'],'capabilities':{'tools':{}},'_meta':{'io.modelcontextprotocol/serverInfo':{'name':'AiWin11 fixture','version':'1'}}}\n",
            "    elif method == 'tools/list':\n",
            "        result = {'resultType':'complete','tools':[{'name':'read_fixture','inputSchema':{'type':'object'}}]}\n",
            "    elif method == 'tools/call' and req['params']['name'] == 'read_fixture':\n",
            "        result = {'resultType':'complete','content':[{'type':'text','text':'MCP fixture OK'}]}\n",
            "    else: raise ValueError(method)\n",
            "    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}), flush=True)\n",
        );
    let directory = std::env::temp_dir().join(format!("aiwin11-mcp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("fixture.py");
    std::fs::write(&path, fixture).unwrap();
    let server = McpServer {
        name: "fixture".into(),
        command: python,
        args: vec!["-I".into(), "-B".into(), path.to_str().unwrap().into()],
        enabled: true,
        allowed_tools: Some(vec!["read_fixture".into()]),
    };
    let probed = probe(&server);
    let called = call_tool(&server, "read_fixture", &json!({}));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
    let probed = probed.unwrap();
    assert_eq!(probed.server_name, "AiWin11 fixture");
    assert_eq!(probed.tools, ["read_fixture"]);
    assert_eq!(called.as_deref(), Ok("MCP fixture OK"));
}
