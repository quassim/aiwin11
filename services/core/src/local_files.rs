use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
};

use aiwin11_contracts::v1::{
    command::Operation, CopyLocalText, DeleteLocalText, ListLocalText, ListTrash, MoveLocalText, PreviewLocalText,
    RestoreLocalText, TrashLocalText,
};
use windows_sys::Wdk::{
    Foundation::OBJECT_ATTRIBUTES,
    Storage::FileSystem::{
        FileRenameInformation, NtCreateFile, NtSetInformationFile, FILE_CREATE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
        FILE_SYNCHRONOUS_IO_NONALERT, NTCREATEFILE_CREATE_DISPOSITION,
    },
};
use windows_sys::Win32::Foundation::{
    HANDLE, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, STATUS_OBJECT_NAME_COLLISION, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileDispositionInfo, GetFileInformationByHandle, GetFinalPathNameByHandleW, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
};

#[cfg(test)]
use crate::broker::Capability;
use crate::{
    broker::{ExecutionError, Peer, Policy},
    Core,
};

const MAX_BYTES: usize = 64 * 1024;
const MAX_NAME: usize = 128;
/// Most names returned by one listing.
pub const MAX_LISTED: usize = 100;

// Windows FILE_RENAME_INFORMATION with storage for the variable-length UTF-16 name.
#[repr(C)]
struct RenameInfo {
    replace_if_exists: u32,
    root_directory: HANDLE,
    name_length: u32,
    name: [u16; MAX_NAME],
}

pub struct LocalFiles {
    root: PathBuf,
    /// AiWin11's own trash, next to `files`: trashed files stay recoverable here.
    trash: PathBuf,
}

impl LocalFiles {
    pub fn open(data_dir: &Path) -> std::io::Result<Self> {
        let canonical = data_dir.canonicalize()?;
        if !matches!(canonical.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::VerbatimDisk(_)))
        {
            return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "local drive required"));
        }
        let root = canonical.join("files");
        let trash = canonical.join("trash");
        for directory in [&root, &trash] {
            match fs::create_dir(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Self { root, trash })
    }

    /// Like `open`, but the agent works in `workspace`, a folder the user chose. It must be an
    /// existing directory on a local disk, strictly inside the user's profile, and outside
    /// AppData (where AiWin11 and other applications keep their data). The trash stays in
    /// AiWin11's data folder.
    pub fn open_with_workspace(data_dir: &Path, workspace: &Path) -> std::io::Result<Self> {
        let mut files = Self::open(data_dir)?;
        files.root = Self::validate_workspace(workspace)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::PermissionDenied, "workspace not allowed"))?;
        Ok(files)
    }

    /// Canonical form of an allowed workspace folder (see `open_with_workspace`).
    pub fn validate_workspace(workspace: &Path) -> Result<PathBuf, &'static str> {
        let canonical = workspace.canonicalize().map_err(|_| "WORKSPACE_NOT_FOUND")?;
        if !canonical.is_dir() {
            return Err("WORKSPACE_NOT_FOUND");
        }
        let profile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .and_then(|profile| profile.canonicalize().ok())
            .ok_or("WORKSPACE_NOT_ALLOWED")?;
        let inside = canonical.starts_with(&profile) && canonical != profile;
        let app_data = canonical.starts_with(profile.join("AppData"));
        let local_disk = matches!(canonical.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::VerbatimDisk(_)));
        if !inside || app_data || !local_disk {
            return Err("WORKSPACE_NOT_ALLOWED");
        }
        Ok(canonical)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn trash(&self) -> &Path {
        &self.trash
    }

    /// Agent tool: renames one text file inside the folder, never replacing an existing file.
    /// The host grants `Capability::FileMove` only when the policy engine or the user allowed it.
    pub fn agent_move(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<(), &'static str> {
        let command =
            core.agent_command(task, Operation::MoveLocalText(MoveLocalText {})).map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.move", || self.move_text(source, destination))
    }

    /// Agent tool: moves one text file into its own new folder of AiWin11's trash, keeping its
    /// name. Nothing is deleted. Returns the folder that now holds the file.
    pub fn agent_trash(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        name: &str,
    ) -> Result<PathBuf, &'static str> {
        let command =
            core.agent_command(task, Operation::TrashLocalText(TrashLocalText {})).map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.trash", || self.trash_text(name))
    }

    /// Read-only check before a trash: the file is readable and in scope.
    pub fn agent_trash_preview(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        name: &str,
    ) -> Result<usize, &'static str> {
        let command = core
            .agent_command(task, Operation::PreviewLocalText(PreviewLocalText {}))
            .map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.preview", || {
            let text = self.read(self.resolve(name)?, name)?;
            Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).chars().count())
        })
    }

    fn move_text(&self, name: &str, destination: &str) -> Result<(), &'static str> {
        if !valid_name(destination) {
            return Err("INVALID_FILE_NAME");
        }
        if name.eq_ignore_ascii_case(destination) {
            return Err("FILE_SAME_NAME");
        }
        let root = self.open_root()?;
        let source = self.open_for_mutation(&root, name)?;
        rename_in_directory(&source, &root, destination, "FILE_MOVE_FAILED")
    }

    fn trash_text(&self, name: &str) -> Result<PathBuf, &'static str> {
        let root = self.open_root()?;
        let source = self.open_for_mutation(&root, name)?;
        let _trash = open_checked_directory(&self.trash)?;
        let folder = self.trash.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&folder).map_err(|_| "FILE_TRASH_FAILED")?;
        let result = open_checked_directory(&folder)
            .and_then(|directory| rename_in_directory(&source, &directory, name, "FILE_TRASH_FAILED"));
        if result.is_err() {
            let _ = fs::remove_dir(&folder);
        }
        result.map(|()| folder)
    }

    pub fn preview(&self, core: &mut Core, name: &str) -> Result<String, &'static str> {
        let command =
            core.desktop_command(Operation::PreviewLocalText(PreviewLocalText {})).map_err(|_| "FILE_UNAVAILABLE")?;
        let path = self.resolve(name);
        let task = core
            .authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        let result = path
            .and_then(|path| self.read(path, name))
            .map(|text| text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned());
        let (kind, code) = match &result {
            Ok(_) => ("file.preview.completed", "success"),
            Err(code) => ("file.preview.failed", *code),
        };
        core.audit(task.as_deref(), kind, code).map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        result
    }

    /// Agent tool: lists the text files of the dedicated folder for `task`.
    /// The broker decides with `policy` under the agent identity, and the outcome is audited.
    pub fn agent_list(&self, core: &mut Core, task: &str, policy: Policy) -> Result<Vec<String>, &'static str> {
        let command =
            core.agent_command(task, Operation::ListLocalText(ListLocalText {})).map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.list", || self.list_names(""))
    }

    /// Agent tool: reads one text file of the dedicated folder for `task`.
    pub fn agent_read(&self, core: &mut Core, task: &str, policy: Policy, name: &str) -> Result<String, &'static str> {
        let command = core
            .agent_command(task, Operation::PreviewLocalText(PreviewLocalText {}))
            .map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.preview", || {
            let text = self.read(self.resolve(name)?, name)?;
            Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
        })
    }

    /// Agent tool, read-only dry run of a copy: reads the source and checks that the
    /// destination name is free. Returns the source length in characters for the approval.
    pub fn agent_copy_preview(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<usize, &'static str> {
        let command = core
            .agent_command(task, Operation::PreviewLocalText(PreviewLocalText {}))
            .map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.preview", || {
            if !valid_name(destination) {
                return Err("INVALID_FILE_NAME");
            }
            if source.eq_ignore_ascii_case(destination) {
                return Err("FILE_SAME_NAME");
            }
            let text = self.read(self.resolve(source)?, source)?;
            // Advisory only: the copy itself never replaces an existing file.
            if fs::symlink_metadata(self.root.join(destination)).is_ok() {
                return Err("FILE_DESTINATION_EXISTS");
            }
            Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).chars().count())
        })
    }

    /// Agent tool: copies one text file inside the folder. The host passes a policy with
    /// `Capability::FileCopy` only when the policy engine or the user allowed this exact copy.
    pub fn agent_copy(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<(), &'static str> {
        let command =
            core.agent_command(task, Operation::CopyLocalText(CopyLocalText {})).map_err(|_| "FILE_UNAVAILABLE")?;
        self.observe(core, &command, policy, "file.copy", || self.copy_text(source, destination))
    }

    fn observe<T>(
        &self,
        core: &mut Core,
        command: &aiwin11_contracts::v1::Command,
        policy: Policy,
        event: &str,
        execute: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let task = core.authorize_dispatch(policy, Peer::Agent, command).map_err(|error| match error {
            ExecutionError::Denied(_) => "FILE_ACCESS_DENIED",
            _ => "FILE_AUDIT_UNAVAILABLE",
        })?;
        let result = execute();
        let (suffix, code) = match &result {
            Ok(_) => ("completed", "success"),
            Err(code) => ("failed", *code),
        };
        core.audit(task.as_deref(), &format!("{event}.{suffix}"), code).map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        result
    }

    /// Supported text files directly in the folder, sorted, at most `MAX_LISTED`.
    fn list_names(&self, query: &str) -> Result<Vec<String>, &'static str> {
        if query.len() > MAX_NAME || !query.bytes().all(|byte| byte.is_ascii_alphanumeric() || b" -_.".contains(&byte))
        {
            return Err("INVALID_FILE_SEARCH");
        }
        let query = query.trim().to_ascii_lowercase();
        // Checks that the folder itself is the expected, non-redirected directory.
        let _root = self.open_root()?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|_| "FILE_UNAVAILABLE")? {
            let entry = entry.map_err(|_| "FILE_UNAVAILABLE")?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
            // DirEntry metadata does not follow links on Windows.
            let Ok(meta) = entry.metadata() else { continue };
            if valid_name(&name)
                && name.to_ascii_lowercase().contains(&query)
                && meta.is_file()
                && meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            {
                names.push(name);
            }
        }
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names.truncate(MAX_LISTED);
        Ok(names)
    }

    pub fn copy(&self, core: &mut Core, name: &str, destination: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::CopyLocalText(CopyLocalText {}), "file.copy", || self.copy_text(name, destination))
    }

    pub fn move_file(&self, core: &mut Core, name: &str, destination: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::MoveLocalText(MoveLocalText {}), "file.move", || self.move_text(name, destination))
    }

    pub fn trash_file(&self, core: &mut Core, name: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::TrashLocalText(TrashLocalText {}), "file.trash", || {
            self.trash_text(name).map(|_| ())
        })
    }

    pub fn list_files(&self, core: &mut Core, query: &str) -> Result<Vec<String>, &'static str> {
        self.mutate(core, Operation::ListLocalText(ListLocalText {}), "file.list", || self.list_names(query))
    }

    pub fn list_trash(&self, core: &mut Core) -> Result<Vec<(String, String)>, &'static str> {
        self.mutate(core, Operation::ListTrash(ListTrash {}), "file.trash.list", || {
            let _trash = open_checked_directory(&self.trash)?;
            let mut entries = Vec::new();
            for folder in fs::read_dir(&self.trash).map_err(|_| "FILE_UNAVAILABLE")? {
                let Ok(folder) = folder else { continue };
                let Ok(id) = folder.file_name().into_string() else { continue };
                if !valid_trash_id(&id) {
                    continue;
                }
                let path = folder.path();
                let Ok(_directory) = open_checked_directory(&path) else { continue };
                let Ok(files) = fs::read_dir(path) else { continue };
                for entry in files.flatten() {
                    let Ok(name) = entry.file_name().into_string() else { continue };
                    let Ok(metadata) = entry.metadata() else { continue };
                    if valid_name(&name)
                        && metadata.is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
                    {
                        entries.push((id.clone(), name));
                        if entries.len() == MAX_LISTED {
                            break;
                        }
                    }
                }
                if entries.len() == MAX_LISTED {
                    break;
                }
            }
            entries.sort_by_key(|(_, name)| name.to_ascii_lowercase());
            Ok(entries)
        })
    }

    pub fn restore(&self, core: &mut Core, id: &str, name: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::RestoreLocalText(RestoreLocalText {}), "file.restore", || {
            if !valid_trash_id(id) || !valid_name(name) {
                return Err("INVALID_FILE_NAME");
            }
            let root = self.open_root()?;
            let _trash = open_checked_directory(&self.trash)?;
            let path = self.trash.join(id);
            let directory = open_checked_directory(&path)?;
            let source = open_relative(&directory, name, FILE_GENERIC_READ | DELETE, FILE_OPEN)
                .map_err(|_| "FILE_UNAVAILABLE")?;
            read_checked_in(&source, &path, name)?;
            rename_in_directory(&source, &root, name, "FILE_RESTORE_FAILED")?;
            // Keep the empty entry directory rather than removing it by a reopened path.
            Ok(())
        })
    }

    pub fn delete(&self, core: &mut Core, name: &str, confirmed: bool) -> Result<(), &'static str> {
        self.mutate(core, Operation::DeleteLocalText(DeleteLocalText { confirmed }), "file.delete", || {
            let root = self.open_root()?;
            let source = self.open_for_mutation(&root, name)?;
            mark_for_deletion(&source).then_some(()).ok_or("FILE_DELETE_FAILED")
        })
    }

    fn mutate<T>(
        &self,
        core: &mut Core,
        operation: Operation,
        event: &str,
        execute: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let command = core.desktop_command(operation).map_err(|_| "FILE_UNAVAILABLE")?;
        let task =
            core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command).map_err(
                |error| match error {
                    ExecutionError::Denied("confirmation_required") => "FILE_CONFIRMATION_REQUIRED",
                    _ => "FILE_AUDIT_UNAVAILABLE",
                },
            )?;
        let result = execute();
        let (suffix, code) = match &result {
            Ok(_) => ("completed", "success"),
            Err(code) => ("failed", *code),
        };
        core.audit(task.as_deref(), &format!("{event}.{suffix}"), code).map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        result
    }

    fn copy_text(&self, name: &str, destination: &str) -> Result<(), &'static str> {
        if !valid_name(destination) {
            return Err("INVALID_FILE_NAME");
        }
        let root = self.open_root()?;
        let text = self.read(self.resolve(name)?, name)?;
        let mut staging = create_staging(&root)?;
        let result = staging
            .write_all(text.as_bytes())
            .and_then(|()| staging.sync_all())
            .map_err(|_| "FILE_COPY_FAILED")
            .and_then(|()| rename_in_directory(&staging, &root, destination, "FILE_COPY_FAILED"));
        if result.is_err() && !mark_for_deletion(&staging) {
            return Err("FILE_COPY_CLEANUP_FAILED");
        }
        result
    }

    fn open_root(&self) -> Result<File, &'static str> {
        open_checked_directory(&self.root)
    }

    fn open_for_mutation(&self, root: &File, name: &str) -> Result<File, &'static str> {
        if !valid_name(name) {
            return Err("INVALID_FILE_NAME");
        }
        let file = open_relative(root, name, FILE_GENERIC_READ | DELETE, FILE_OPEN).map_err(|_| "FILE_UNAVAILABLE")?;
        self.read_checked(&file, name)?;
        Ok(file)
    }

    fn matches_scope(&self, path: &Path, name: &str) -> bool {
        path.parent() == Some(self.root.as_path())
            && path.file_name().and_then(|leaf| leaf.to_str()).is_some_and(|leaf| leaf.eq_ignore_ascii_case(name))
    }

    fn resolve(&self, name: &str) -> Result<PathBuf, &'static str> {
        if !valid_name(name) {
            return Err("INVALID_FILE_NAME");
        }
        let path = self.root.join(name);
        for candidate in [&self.root, &path] {
            let meta = fs::symlink_metadata(candidate).map_err(|_| "FILE_UNAVAILABLE")?;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err("FILE_OUTSIDE_SCOPE");
            }
        }
        let resolved = path.canonicalize().map_err(|_| "FILE_UNAVAILABLE")?;
        if !self.matches_scope(&resolved, name) {
            return Err("FILE_OUTSIDE_SCOPE");
        }
        Ok(path)
    }

    fn read(&self, path: PathBuf, name: &str) -> Result<String, &'static str> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| "FILE_UNAVAILABLE")?;
        self.read_checked(&file, name)
    }

    fn read_checked(&self, file: &File, name: &str) -> Result<String, &'static str> {
        read_checked_in(file, &self.root, name)
    }
}

fn valid_trash_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

fn read_checked_in(file: &File, directory: &Path, name: &str) -> Result<String, &'static str> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err("FILE_UNAVAILABLE");
    }
    let path = final_path(file)?;
    if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || info.nNumberOfLinks != 1
        || path.parent() != Some(directory)
        || !path.file_name().and_then(|leaf| leaf.to_str()).is_some_and(|leaf| leaf.eq_ignore_ascii_case(name))
    {
        return Err("FILE_OUTSIDE_SCOPE");
    }
    let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
    if size > MAX_BYTES as u64 {
        return Err("FILE_TOO_LARGE");
    }

    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes).map_err(|_| "FILE_UNAVAILABLE")?;
    if bytes.len() > MAX_BYTES {
        return Err("FILE_TOO_LARGE");
    }
    if bytes.contains(&0) {
        return Err("FILE_NOT_UTF8");
    }
    String::from_utf8(bytes).map_err(|_| "FILE_NOT_UTF8")
}

/// Opens a directory by handle and checks that it is a real directory (no junction or link)
/// whose final path is exactly `path`.
fn open_checked_directory(path: &Path) -> Result<File, &'static str> {
    let directory = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "FILE_UNAVAILABLE")?;
    let attributes = directory.metadata().map_err(|_| "FILE_UNAVAILABLE")?.file_attributes();
    if attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != FILE_ATTRIBUTE_DIRECTORY
        || final_path(&directory)? != path
    {
        return Err("FILE_OUTSIDE_SCOPE");
    }
    Ok(directory)
}

fn create_staging(root: &File) -> Result<File, &'static str> {
    let name = format!(".aiwin11-{}.tmp", uuid::Uuid::new_v4());
    open_relative(root, &name, FILE_GENERIC_WRITE | DELETE, FILE_CREATE).map_err(|_| "FILE_COPY_FAILED")
}

fn open_relative(
    root: &File,
    name: &str,
    access: u32,
    disposition: NTCREATEFILE_CREATE_DISPOSITION,
) -> Result<File, i32> {
    let mut name: Vec<u16> = name.encode_utf16().collect();
    let length = (name.len() * size_of::<u16>()) as u16;
    let name = UNICODE_STRING { Length: length, MaximumLength: length, Buffer: name.as_mut_ptr() };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: root.as_raw_handle(),
        ObjectName: &name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        ..Default::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = Default::default();
    // The directory handle anchors the lookup and OBJ_DONT_REPARSE blocks redirects.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            0,
            disposition,
            FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(status);
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn mark_for_deletion(file: &File) -> bool {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    result != 0
}

fn rename_in_directory(file: &File, root: &File, destination: &str, failure: &'static str) -> Result<(), &'static str> {
    let mut info = RenameInfo {
        replace_if_exists: 0,
        root_directory: root.as_raw_handle(),
        name_length: (destination.len() * size_of::<u16>()) as u32,
        name: [0; MAX_NAME],
    };
    for (unit, byte) in info.name.iter_mut().zip(destination.bytes()) {
        *unit = u16::from(byte);
    }
    let mut status_block = Default::default();
    // Rename the opened file inside the checked directory without replacement.
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status_block,
            std::ptr::from_ref(&info).cast(),
            size_of::<RenameInfo>() as u32,
            FileRenameInformation,
        )
    };
    match status {
        STATUS_OBJECT_NAME_COLLISION => Err("FILE_DESTINATION_EXISTS"),
        status if status < 0 => Err(failure),
        _ => Ok(()),
    }
}

fn final_path(file: &File) -> Result<PathBuf, &'static str> {
    let mut buffer = vec![0u16; 32768];
    let length = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), buffer.len() as u32, 0) }
        as usize;
    if length == 0 || length >= buffer.len() {
        return Err("FILE_UNAVAILABLE");
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

fn valid_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else { return false };
    let uppercase = stem.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (uppercase.len() == 4
            && (uppercase.starts_with("COM") || uppercase.starts_with("LPT"))
            && uppercase.as_bytes()[3].is_ascii_digit());
    name.len() <= MAX_NAME
        && ["txt", "md", "csv"].iter().any(|supported| extension.eq_ignore_ascii_case(supported))
        && !stem.is_empty()
        && stem == stem.trim()
        && !reserved
        && stem.bytes().all(|byte| byte.is_ascii_alphanumeric() || b" -_".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_must_be_inside_the_profile_and_outside_app_data() {
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        let windows = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        assert_eq!(LocalFiles::validate_workspace(&windows), Err("WORKSPACE_NOT_ALLOWED"));
        assert_eq!(LocalFiles::validate_workspace(&profile), Err("WORKSPACE_NOT_ALLOWED"), "the profile root itself");
        assert_eq!(LocalFiles::validate_workspace(&std::env::temp_dir()), Err("WORKSPACE_NOT_ALLOWED"), "AppData");
        assert_eq!(LocalFiles::validate_workspace(&profile.join("aiwin11-missing-folder")), Err("WORKSPACE_NOT_FOUND"));
        let folder = profile.join(format!("aiwin11-workspace-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let accepted = LocalFiles::validate_workspace(&folder);
        let _ = std::fs::remove_dir(&folder);
        assert!(accepted.is_ok(), "{accepted:?}");
    }
    use rusqlite::Connection;
    use uuid::Uuid;

    #[test]
    fn accepts_only_flat_plain_text_names() {
        assert!(valid_name("Notes 1.TXT"));
        assert!(valid_name("Notes 1.MD"));
        assert!(valid_name("Tableau.CSV"));
        for name in ["../x.txt", "C:\\x.txt", "x.txt:stream", "dir/x.txt", "NUL.txt", "COM1.txt", "x.txt ", "x..txt"] {
            assert!(!valid_name(name), "{name}");
        }
    }

    #[test]
    fn copy_publishes_exact_bytes_without_replacing_or_leaving_staging_files() {
        let directory = std::env::temp_dir().join(format!("aiwin11-file-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let files = LocalFiles::open(&directory).unwrap();
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let source = files.root().join("source.md");
        let destination = files.root().join("copie.md");
        let original = "\u{feff}Bonjour\r\n".as_bytes();
        fs::write(&source, original).unwrap();
        assert_eq!(files.copy(&mut core, "source.md", "copie.md"), Ok(()));
        assert_eq!(fs::read(&destination).unwrap(), original);
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::write(&destination, "À garder").unwrap();
        assert_eq!(files.copy(&mut core, "source.md", "copie.md"), Err("FILE_DESTINATION_EXISTS"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "À garder");
        assert_eq!(files.copy(&mut core, "source.md", "../hors.md"), Err("INVALID_FILE_NAME"));
        assert!(!directory.join("hors.txt").exists());
        assert_eq!(fs::read_dir(files.root()).unwrap().count(), 2);
        let events = core.activity().unwrap();
        assert_eq!(events[0].kind, "file.copy.failed");
        assert_eq!(events[4].kind, "file.copy.completed");
        for path in [source, destination] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn agent_reads_need_the_grant_and_list_only_plain_text_files() {
        let directory = std::env::temp_dir().join(format!("aiwin11-file-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let files = LocalFiles::open(&directory).unwrap();
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        fs::write(files.root().join("b note.txt"), "\u{feff}Deuxième").unwrap();
        fs::write(files.root().join("A.txt"), "Premier").unwrap();
        fs::write(files.root().join("budget.csv"), "poste,montant\r\ntrain,25").unwrap();
        fs::write(files.root().join("image.png"), "x").unwrap();
        fs::write(files.root().join(".aiwin11-staging.tmp"), "x").unwrap();
        fs::create_dir(files.root().join("dossier.txt")).unwrap();
        fs::write(directory.join("outside.txt"), "outside").unwrap();
        fs::hard_link(directory.join("outside.txt"), files.root().join("lien.txt")).unwrap();

        let denied = Policy::default();
        assert_eq!(files.agent_list(&mut core, &task, denied), Err("FILE_ACCESS_DENIED"));
        assert_eq!(files.agent_read(&mut core, &task, denied, "A.txt"), Err("FILE_ACCESS_DENIED"));

        let reads = Policy::granting(&[Capability::FileRead]);
        assert_eq!(files.list_files(&mut core, "NOTE"), Ok(vec!["b note.txt".into()]));
        assert_eq!(files.list_files(&mut core, "../outside"), Err("INVALID_FILE_SEARCH"));
        // Hard links are listed by name but refused when read, like in the preview.
        assert_eq!(
            files.agent_list(&mut core, &task, reads),
            Ok(vec!["A.txt".into(), "b note.txt".into(), "budget.csv".into(), "lien.txt".into()])
        );
        assert_eq!(files.agent_read(&mut core, &task, reads, "b note.txt"), Ok("Deuxième".into()));
        assert_eq!(files.preview(&mut core, "budget.csv"), Ok("poste,montant\r\ntrain,25".into()));
        assert_eq!(files.agent_read(&mut core, &task, reads, "lien.txt"), Err("FILE_OUTSIDE_SCOPE"));
        assert_eq!(files.agent_read(&mut core, &task, reads, "../outside.txt"), Err("INVALID_FILE_NAME"));
        let kinds: Vec<_> = core.activity().unwrap().into_iter().map(|event| event.kind).collect();
        assert_eq!(&kinds[..4], ["file.preview.failed", "policy.allowed", "file.preview.failed", "policy.allowed"]);
        assert!(kinds.contains(&"file.list.completed".to_owned()));
        assert_eq!(kinds.iter().filter(|kind| *kind == "policy.denied").count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn agent_copy_is_previewed_read_only_and_needs_its_own_grant() {
        let directory = std::env::temp_dir().join(format!("aiwin11-file-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let files = LocalFiles::open(&directory).unwrap();
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let original = "\u{feff}Données\r\n".as_bytes();
        fs::write(files.root().join("source.txt"), original).unwrap();
        fs::write(files.root().join("pris.txt"), "déjà là").unwrap();
        let reads = Policy::granting(&[Capability::FileRead]);
        let copy = Policy::granting(&[Capability::FileCopy]);

        assert_eq!(files.agent_copy_preview(&mut core, &task, reads, "source.txt", "copie.txt"), Ok(9));
        assert_eq!(
            files.agent_copy_preview(&mut core, &task, reads, "source.txt", "PRIS.txt"),
            Err("FILE_DESTINATION_EXISTS")
        );
        assert_eq!(
            files.agent_copy_preview(&mut core, &task, reads, "source.txt", "SOURCE.TXT"),
            Err("FILE_SAME_NAME")
        );
        assert_eq!(
            files.agent_copy_preview(&mut core, &task, reads, "absent.txt", "copie.txt"),
            Err("FILE_UNAVAILABLE")
        );
        assert_eq!(
            files.agent_copy_preview(&mut core, &task, Policy::default(), "source.txt", "copie.txt"),
            Err("FILE_ACCESS_DENIED")
        );
        assert!(!files.root().join("copie.txt").exists(), "the preview writes nothing");

        assert_eq!(files.agent_copy(&mut core, &task, reads, "source.txt", "copie.txt"), Err("FILE_ACCESS_DENIED"));
        assert!(!files.root().join("copie.txt").exists());
        assert_eq!(files.agent_copy(&mut core, &task, copy, "source.txt", "copie.txt"), Ok(()));
        assert_eq!(fs::read(files.root().join("copie.txt")).unwrap(), original);
        assert_eq!(files.agent_copy(&mut core, &task, copy, "source.txt", "pris.txt"), Err("FILE_DESTINATION_EXISTS"));
        assert_eq!(fs::read_to_string(files.root().join("pris.txt")).unwrap(), "déjà là");
        let kinds: Vec<_> = core.activity().unwrap().into_iter().map(|event| event.kind).collect();
        assert_eq!(&kinds[..4], ["file.copy.failed", "policy.allowed", "file.copy.completed", "policy.allowed"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn agent_move_and_trash_need_their_own_grants_and_never_replace_or_delete() {
        let directory = std::env::temp_dir().join(format!("aiwin11-file-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let files = LocalFiles::open(&directory).unwrap();
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        fs::write(files.root().join("brouillon.txt"), "Texte").unwrap();
        fs::write(files.root().join("pris.txt"), "garder").unwrap();
        let reads = Policy::granting(&[Capability::FileRead]);
        let moves = Policy::granting(&[Capability::FileMove]);
        let trashes = Policy::granting(&[Capability::FileTrash]);

        assert_eq!(files.agent_move(&mut core, &task, reads, "brouillon.txt", "final.txt"), Err("FILE_ACCESS_DENIED"));
        assert_eq!(
            files.agent_move(&mut core, &task, trashes, "brouillon.txt", "final.txt"),
            Err("FILE_ACCESS_DENIED")
        );
        assert_eq!(
            files.agent_move(&mut core, &task, moves, "brouillon.txt", "pris.txt"),
            Err("FILE_DESTINATION_EXISTS")
        );
        assert_eq!(fs::read_to_string(files.root().join("pris.txt")).unwrap(), "garder");
        assert_eq!(
            files.agent_move(&mut core, &task, moves, "brouillon.txt", "../final.txt"),
            Err("INVALID_FILE_NAME")
        );
        assert_eq!(files.agent_move(&mut core, &task, moves, "brouillon.txt", "final.txt"), Ok(()));
        assert!(!files.root().join("brouillon.txt").exists());
        assert_eq!(fs::read_to_string(files.root().join("final.txt")).unwrap(), "Texte");

        assert_eq!(files.agent_trash_preview(&mut core, &task, reads, "final.txt"), Ok(5));
        assert_eq!(files.agent_trash(&mut core, &task, moves, "final.txt"), Err("FILE_ACCESS_DENIED"));
        let folder = files.agent_trash(&mut core, &task, trashes, "final.txt").unwrap();
        assert!(!files.root().join("final.txt").exists());
        assert_eq!(folder.parent(), Some(files.trash()));
        assert_eq!(fs::read_to_string(folder.join("final.txt")).unwrap(), "Texte", "trashed, not deleted");
        assert_eq!(files.agent_trash(&mut core, &task, trashes, "absent.txt"), Err("FILE_UNAVAILABLE"));
        assert_eq!(fs::read_dir(files.trash()).unwrap().count(), 1, "a failed trash leaves no empty folder");
        let id = folder.file_name().unwrap().to_str().unwrap();
        assert_eq!(files.list_trash(&mut core).unwrap(), vec![(id.to_owned(), "final.txt".to_owned())]);
        let destination = files.root().join("final.txt");
        fs::write(&destination, "garder").unwrap();
        assert_eq!(files.restore(&mut core, id, "final.txt"), Err("FILE_DESTINATION_EXISTS"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "garder");
        assert_eq!(fs::read_to_string(folder.join("final.txt")).unwrap(), "Texte");
        fs::remove_file(&destination).unwrap();
        assert_eq!(files.restore(&mut core, id, "final.txt"), Ok(()));
        assert_eq!(fs::read_to_string(destination).unwrap(), "Texte");
        assert!(files.list_trash(&mut core).unwrap().is_empty());
        let kinds: Vec<_> = core.activity().unwrap().into_iter().map(|event| event.kind).collect();
        assert!(
            kinds.contains(&"file.trash.completed".to_owned()) && kinds.contains(&"file.move.completed".to_owned())
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_hard_links_and_preserves_normal_preview() {
        let directory = std::env::temp_dir().join(format!("aiwin11-file-test-{}", Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let files = LocalFiles::open(&directory).unwrap();
        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let original = files.root().join("note.txt");
        fs::write(&original, "\u{feff}Bonjour").unwrap();
        assert_eq!(files.preview(&mut core, "note.txt"), Ok("Bonjour".into()));
        let outside = directory.join("outside.txt");
        let alias = files.root().join("alias.txt");
        fs::write(&outside, "outside").unwrap();
        fs::hard_link(&outside, &alias).unwrap();
        assert_eq!(files.preview(&mut core, "alias.txt"), Err("FILE_OUTSIDE_SCOPE"));
        let events = core.activity().unwrap();
        assert_eq!(events[0].kind, "file.preview.failed");
        assert_eq!(events[2].kind, "file.preview.completed");
        for path in [original, alias, outside] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
