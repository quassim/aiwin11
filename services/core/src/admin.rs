//! On-demand elevated helper (ROADMAP §6, option B).
//!
//! AiWin11 runs as a standard user. A closed list of administrator operations (Windows
//! services, system temporary files) runs in `AiWin11.Admin.exe`, started through UAC only
//! after (1) the helper is enabled by the user in Security, (2) the policy engine and the
//! user approved this exact operation, (3) the broker authorized the command, and (4) the
//! user accepted the Windows UAC prompt. The helper performs one operation and exits.

use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

use aiwin11_contracts::v1::{admin_service::Action as ServiceAction, command::Operation};
use rusqlite::params;
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0},
    System::Threading::{GetExitCodeProcess, WaitForSingleObject},
    UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
};

use crate::{
    broker::{ExecutionError, Peer, Policy},
    Core,
};

/// Services the helper never touches (security, updates, services Windows needs to run).
/// The helper holds the authoritative copy; this one refuses early, before any UAC prompt.
pub const PROTECTED_SERVICES: &[&str] = &[
    "WinDefend",
    "WdNisSvc",
    "WdFilter",
    "Sense",
    "SecurityHealthService",
    "wscsvc",
    "MpsSvc",
    "BFE",
    "wuauserv",
    "UsoSvc",
    "WaaSMedicSvc",
    "TrustedInstaller",
    "RpcSs",
    "RpcEptMapper",
    "DcomLaunch",
    "LSM",
    "SamSs",
    "EventLog",
    "Winmgmt",
    "CryptSvc",
    "Power",
    "PlugPlay",
    "ProfSvc",
    "Schedule",
    "SystemEventsBroker",
    "BrokerInfrastructure",
    "CoreMessagingRegistrar",
    "gpsvc",
    "Dhcp",
    "Dnscache",
    "nsi",
    "Netlogon",
    "KeyIso",
    "VaultSvc",
    "AppIDSvc",
    "EFS",
    "BITS",
];

/// UAC prompt plus the operation itself (a service can take 30 s to stop and 30 s to start).
const HELPER_WAIT_MS: u32 = 90_000;
const SETTING: &str = "admin.helper.enabled";

pub fn valid_service_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

pub fn protected_service(name: &str) -> bool {
    PROTECTED_SERVICES.iter().any(|protected| protected.eq_ignore_ascii_case(name))
}

pub fn service_action_code(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Start => "start",
        ServiceAction::Stop => "stop",
        ServiceAction::Restart => "restart",
        ServiceAction::SetAutomatic => "automatic",
        ServiceAction::SetManual => "manual",
        ServiceAction::SetDisabled => "disabled",
        ServiceAction::Unspecified => "unspecified",
    }
}

impl Core {
    /// Disabled until the user turns it on in Security.
    pub fn admin_helper_enabled(&self) -> bool {
        self.setting(SETTING).ok().flatten().as_deref() == Some("true")
    }

    pub fn set_admin_helper_enabled(&mut self, enabled: bool) -> Result<(), &'static str> {
        let session = self.session_id.clone();
        let write = |db: &mut rusqlite::Connection| -> rusqlite::Result<()> {
            let transaction = db.transaction()?;
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                params![SETTING, if enabled { "true" } else { "false" }],
            )?;
            transaction.execute(
                "INSERT INTO events (id, session_id, kind, source, code)
                 VALUES (?1, ?2, 'settings.admin.changed', 'core', ?3)",
                params![Uuid::new_v4().to_string(), session, if enabled { "enabled" } else { "disabled" }],
            )?;
            transaction.commit()
        };
        write(&mut self.db).map_err(|_| "SETTINGS_UNAVAILABLE")
    }

    /// Authorizes one administrator operation through the broker, runs the helper through
    /// UAC and journals the outcome.
    pub(crate) fn agent_admin(&mut self, task: &str, policy: Policy, operation: Operation) -> Result<(), &'static str> {
        if !self.admin_helper_enabled() {
            return Err("ADMIN_HELPER_DISABLED");
        }
        let arguments = match &operation {
            Operation::AdminService(request) => {
                let action = ServiceAction::try_from(request.action).map_err(|_| "ADMIN_INVALID")?;
                if !valid_service_name(&request.service) || action == ServiceAction::Unspecified {
                    return Err("ADMIN_INVALID");
                }
                if protected_service(&request.service) {
                    return Err("ADMIN_REFUSED");
                }
                format!("service {} {}", service_action_code(action), request.service)
            }
            Operation::AdminCleanTemp(_) => "clean-temp".to_owned(),
            _ => return Err("ADMIN_INVALID"),
        };
        let command = self.agent_command(task, operation).map_err(|_| "ADMIN_FAILED")?;
        let task_id = self.authorize_dispatch(policy, Peer::Agent, &command).map_err(|error| match error {
            ExecutionError::Denied(_) => "ADMIN_ACCESS_DENIED",
            _ => "ADMIN_AUDIT_UNAVAILABLE",
        })?;
        let outcome = run_elevated(&arguments);
        let (kind, code) = match outcome {
            Ok(()) => ("admin.operation.completed", "done"),
            Err(code) => ("admin.operation.failed", code),
        };
        self.audit(task_id.as_deref(), kind, code).map_err(|_| "ADMIN_AUDIT_UNAVAILABLE")?;
        outcome
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// Starts the helper next to the current executable with "runas" (UAC prompt), waits for it
/// and maps its exit code.
fn run_elevated(arguments: &str) -> Result<(), &'static str> {
    let executable = std::env::current_exe().map_err(|_| "ADMIN_HELPER_MISSING")?;
    let helper = executable.parent().ok_or("ADMIN_HELPER_MISSING")?.join("AiWin11.Admin.exe");
    if !helper.is_file() {
        return Err("ADMIN_HELPER_MISSING");
    }
    // Held until the helper exits: nobody can replace or modify the file in the meantime.
    let _locked = verify_helper(&helper)?;
    let file = wide(helper.as_os_str());
    let verb = wide(OsStr::new("runas"));
    let parameters = wide(OsStr::new(arguments));
    // SAFETY: plain C struct; every pointer field stays valid until ShellExecuteExW returns.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.lpDirectory = ptr::null();
    info.nShow = 0; // SW_HIDE
                    // SAFETY: `info` is initialized as documented for ShellExecuteExW.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // ERROR_CANCELLED: the user declined the UAC prompt.
        return Err(if unsafe { GetLastError() } == 1223 { "ADMIN_UAC_DECLINED" } else { "ADMIN_FAILED" });
    }
    if info.hProcess.is_null() {
        return Err("ADMIN_FAILED");
    }
    // SAFETY: hProcess is a valid process handle owned by this function and closed below.
    let code = unsafe {
        let waited = WaitForSingleObject(info.hProcess, HELPER_WAIT_MS);
        let mut code = 0u32;
        let ok = waited == WAIT_OBJECT_0 && GetExitCodeProcess(info.hProcess, &mut code) != 0;
        CloseHandle(info.hProcess);
        ok.then_some(code)
    };
    match code {
        Some(0) => Ok(()),
        Some(2) => Err("ADMIN_INVALID"),
        Some(3) => Err("ADMIN_REFUSED"),
        Some(5) => Err("ADMIN_NOT_ELEVATED"),
        Some(_) => Err("ADMIN_FAILED"),
        None => Err("ADMIN_TIMEOUT"),
    }
}

/// SHA-256 of the helper expected by this build (set by `scripts/package.ps1`; absent in
/// development builds, where only the lock applies).
const EXPECTED_SHA256: Option<&str> = option_env!("AIWIN11_ADMIN_SHA256");

/// Opens the helper with read sharing only (no writer, no deletion, no rename while open),
/// then checks its SHA-256 against the value pinned at build time.
fn verify_helper(helper: &std::path::Path) -> Result<std::fs::File, &'static str> {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    const FILE_SHARE_READ: u32 = 0x1;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(helper)
        .map_err(|_| "ADMIN_HELPER_TAMPERED")?;
    if let Some(expected) = EXPECTED_SHA256 {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| "ADMIN_HELPER_TAMPERED")?;
        if !sha256_hex(&bytes).is_some_and(|digest| digest.eq_ignore_ascii_case(expected.trim())) {
            return Err("ADMIN_HELPER_TAMPERED");
        }
    }
    Ok(file)
}

fn sha256_hex(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut digest = [0u8; 32];
    // SAFETY: pseudo-handle algorithm, borrowed input, 32-byte output buffer.
    let status = unsafe {
        BCryptHash(
            BCRYPT_SHA256_ALG_HANDLE,
            ptr::null(),
            0,
            bytes.as_ptr(),
            bytes.len() as u32,
            digest.as_mut_ptr(),
            32,
        )
    };
    (status == 0).then(|| digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiwin11_contracts::v1::{AdminCleanTemp, AdminService};
    use rusqlite::Connection;

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(sha256_hex(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn service_names_are_validated_and_protected_services_refused() {
        assert!(valid_service_name("Spooler") && valid_service_name("my-svc_1.2"));
        for bad in ["", "a b", "x;y", "..\\x", &"a".repeat(65)] {
            assert!(!valid_service_name(bad), "{bad}");
        }
        assert!(protected_service("windefend") && protected_service("WUAUSERV") && !protected_service("Spooler"));
        let helper = include_str!("../../admin/Program.cs");
        for name in PROTECTED_SERVICES {
            assert!(helper.contains(&format!("\"{name}\"")), "{name} missing from the helper's list");
        }
    }

    #[test]
    fn the_helper_is_disabled_by_default_and_checks_requests_before_any_prompt() {
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let grant = Policy::granting(&[crate::broker::Capability::AdminOperation]);
        let clean = Operation::AdminCleanTemp(AdminCleanTemp {});
        assert!(!core.admin_helper_enabled());
        assert_eq!(core.agent_admin(&task, grant, clean.clone()), Err("ADMIN_HELPER_DISABLED"));
        core.set_admin_helper_enabled(true).unwrap();
        assert!(core.admin_helper_enabled());
        let service = |name: &str, action: ServiceAction| {
            Operation::AdminService(AdminService { service: name.into(), action: action as i32 })
        };
        assert_eq!(core.agent_admin(&task, grant, service("WinDefend", ServiceAction::Stop)), Err("ADMIN_REFUSED"));
        assert_eq!(core.agent_admin(&task, grant, service("a b", ServiceAction::Stop)), Err("ADMIN_INVALID"));
        assert_eq!(
            core.agent_admin(&task, grant, service("Spooler", ServiceAction::Unspecified)),
            Err("ADMIN_INVALID")
        );
        // Without the capability the broker refuses before the helper is ever started.
        assert_eq!(core.agent_admin(&task, Policy::default(), clean), Err("ADMIN_ACCESS_DENIED"));
    }
}
