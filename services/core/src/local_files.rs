use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{ffi::OsStringExt, fs::{MetadataExt, OpenOptionsExt}, io::{AsRawHandle, FromRawHandle}},
    path::{Path, PathBuf, Prefix, Component},
};

use aiwin11_contracts::v1::{command::Operation, CopyLocalText, DeleteLocalText, MoveLocalText, PreviewLocalText};
use windows_sys::Wdk::{Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::{
    NtCreateFile, NtSetInformationFile, FileRenameInformation, FILE_CREATE,
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT, NTCREATEFILE_CREATE_DISPOSITION,
}};
use windows_sys::Win32::Foundation::{
    HANDLE, UNICODE_STRING, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, STATUS_OBJECT_NAME_COLLISION,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_FLAG_BACKUP_SEMANTICS, FILE_ATTRIBUTE_NORMAL,
    DELETE, SYNCHRONIZE, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FileDispositionInfo, FILE_DISPOSITION_INFO,
    SetFileInformationByHandle,
};

use crate::{broker::{ExecutionError, Peer, Policy}, Core};

const MAX_BYTES: usize = 64 * 1024;
const MAX_NAME: usize = 128;

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
        match fs::create_dir(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn preview(&self, core: &mut Core, name: &str) -> Result<String, &'static str> {
        let command = core.desktop_command(Operation::PreviewLocalText(PreviewLocalText {}))
            .map_err(|_| "FILE_UNAVAILABLE")?;
        let path = self.resolve(name);
        let task = core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        let result = path.and_then(|path| self.read(path, name)).map(|text| {
            text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
        });
        let (kind, code) = match &result {
            Ok(_) => ("file.preview.completed", "success"),
            Err(code) => ("file.preview.failed", *code),
        };
        core.audit(task.as_deref(), kind, code).map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        result
    }

    pub fn copy(&self, core: &mut Core, name: &str, destination: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::CopyLocalText(CopyLocalText {}), "file.copy",
            || self.copy_text(name, destination))
    }

    pub fn move_file(&self, core: &mut Core, name: &str, destination: &str) -> Result<(), &'static str> {
        self.mutate(core, Operation::MoveLocalText(MoveLocalText {}), "file.move", || {
            if !valid_name(destination) {
                return Err("INVALID_FILE_NAME");
            }
            if name.eq_ignore_ascii_case(destination) {
                return Err("FILE_SAME_NAME");
            }
            let root = self.open_root()?;
            let source = self.open_for_mutation(&root, name)?;
            rename_in_directory(&source, &root, destination, "FILE_MOVE_FAILED")
        })
    }

    pub fn delete(&self, core: &mut Core, name: &str, confirmed: bool) -> Result<(), &'static str> {
        self.mutate(core, Operation::DeleteLocalText(DeleteLocalText { confirmed }), "file.delete", || {
            let root = self.open_root()?;
            let source = self.open_for_mutation(&root, name)?;
            mark_for_deletion(&source).then_some(()).ok_or("FILE_DELETE_FAILED")
        })
    }

    fn mutate(
        &self, core: &mut Core, operation: Operation, event: &str,
        execute: impl FnOnce() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let command = core.desktop_command(operation)
            .map_err(|_| "FILE_UNAVAILABLE")?;
        let task = core.authorize_dispatch(Policy::default(), Peer::DesktopInspector, &command)
            .map_err(|error| match error {
                ExecutionError::Denied("confirmation_required") => "FILE_CONFIRMATION_REQUIRED",
                _ => "FILE_AUDIT_UNAVAILABLE",
            })?;
        let result = execute();
        let (suffix, code) = match result {
            Ok(()) => ("completed", "success"),
            Err(code) => ("failed", code),
        };
        core.audit(task.as_deref(), &format!("{event}.{suffix}"), code)
            .map_err(|_| "FILE_AUDIT_UNAVAILABLE")?;
        result
    }

    fn copy_text(&self, name: &str, destination: &str) -> Result<(), &'static str> {
        if !valid_name(destination) {
            return Err("INVALID_FILE_NAME");
        }
        let root = self.open_root()?;
        let text = self.read(self.resolve(name)?, name)?;
        let mut staging = create_staging(&root)?;
        let result = staging.write_all(text.as_bytes()).and_then(|()| staging.sync_all())
            .map_err(|_| "FILE_COPY_FAILED")
            .and_then(|()| rename_in_directory(&staging, &root, destination, "FILE_COPY_FAILED"));
        if result.is_err() && !mark_for_deletion(&staging) {
            return Err("FILE_COPY_CLEANUP_FAILED");
        }
        result
    }

    fn open_root(&self) -> Result<File, &'static str> {
        let root = OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&self.root).map_err(|_| "FILE_UNAVAILABLE")?;
        let attributes = root.metadata().map_err(|_| "FILE_UNAVAILABLE")?.file_attributes();
        if attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != FILE_ATTRIBUTE_DIRECTORY || final_path(&root)? != self.root
        {
            return Err("FILE_OUTSIDE_SCOPE");
        }
        Ok(root)
    }

    fn open_for_mutation(&self, root: &File, name: &str) -> Result<File, &'static str> {
        if !valid_name(name) {
            return Err("INVALID_FILE_NAME");
        }
        let file = open_relative(root, name, FILE_GENERIC_READ | DELETE, FILE_OPEN)
            .map_err(|_| "FILE_UNAVAILABLE")?;
        self.read_checked(&file, name)?;
        Ok(file)
    }

    fn matches_scope(&self, path: &Path, name: &str) -> bool {
        path.parent() == Some(self.root.as_path())
            && path.file_name().and_then(|leaf| leaf.to_str())
                .is_some_and(|leaf| leaf.eq_ignore_ascii_case(name))
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
        let file = OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path).map_err(|_| "FILE_UNAVAILABLE")?;
        self.read_checked(&file, name)
    }

    fn read_checked(&self, file: &File, name: &str) -> Result<String, &'static str> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err("FILE_UNAVAILABLE");
        }
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || info.nNumberOfLinks != 1
            || !self.matches_scope(&final_path(file)?, name)
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
}

fn create_staging(root: &File) -> Result<File, &'static str> {
    let name = format!(".aiwin11-{}.tmp", uuid::Uuid::new_v4());
    open_relative(root, &name, FILE_GENERIC_WRITE | DELETE, FILE_CREATE)
        .map_err(|_| "FILE_COPY_FAILED")
}

fn open_relative(root: &File, name: &str, access: u32, disposition: NTCREATEFILE_CREATE_DISPOSITION) -> Result<File, i32> {
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
        NtCreateFile(&mut handle, access | SYNCHRONIZE,
            &attributes, &mut status_block, std::ptr::null(), FILE_ATTRIBUTE_NORMAL, 0,
            disposition, FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(), 0)
    };
    if status < 0 {
        return Err(status);
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn mark_for_deletion(file: &File) -> bool {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let result = unsafe {
        SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo,
            std::ptr::from_ref(&disposition).cast(), size_of::<FILE_DISPOSITION_INFO>() as u32)
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
        NtSetInformationFile(file.as_raw_handle(), &mut status_block,
            std::ptr::from_ref(&info).cast(), size_of::<RenameInfo>() as u32, FileRenameInformation)
    };
    match status {
        STATUS_OBJECT_NAME_COLLISION => Err("FILE_DESTINATION_EXISTS"),
        status if status < 0 => Err(failure),
        _ => Ok(()),
    }
}

fn final_path(file: &File) -> Result<PathBuf, &'static str> {
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), buffer.len() as u32, 0)
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("FILE_UNAVAILABLE");
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

fn valid_name(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else { return false };
    let uppercase = stem.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (uppercase.len() == 4 && (uppercase.starts_with("COM") || uppercase.starts_with("LPT"))
            && uppercase.as_bytes()[3].is_ascii_digit());
    name.len() <= MAX_NAME && extension.eq_ignore_ascii_case("txt")
        && !stem.is_empty() && stem == stem.trim() && !reserved
        && stem.bytes().all(|byte| byte.is_ascii_alphanumeric() || b" -_".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use uuid::Uuid;

    #[test]
    fn accepts_only_flat_plain_text_names() {
        assert!(valid_name("Notes 1.TXT"));
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
        let source = files.root().join("source.txt");
        let destination = files.root().join("copie.txt");
        let original = "\u{feff}Bonjour\r\n".as_bytes();
        fs::write(&source, original).unwrap();
        assert_eq!(files.copy(&mut core, "source.txt", "copie.txt"), Ok(()));
        assert_eq!(fs::read(&destination).unwrap(), original);
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::write(&destination, "À garder").unwrap();
        assert_eq!(files.copy(&mut core, "source.txt", "copie.txt"), Err("FILE_DESTINATION_EXISTS"));
        assert_eq!(fs::read_to_string(&destination).unwrap(), "À garder");
        assert_eq!(files.copy(&mut core, "source.txt", "../hors.txt"), Err("INVALID_FILE_NAME"));
        assert!(!directory.join("hors.txt").exists());
        assert_eq!(fs::read_dir(files.root()).unwrap().count(), 2);
        let events = core.activity().unwrap();
        assert_eq!(events[0].kind, "file.copy.failed");
        assert_eq!(events[4].kind, "file.copy.completed");
        for path in [source, destination] { fs::remove_file(path).unwrap(); }
        fs::remove_dir(files.root()).unwrap();
        fs::remove_dir(directory).unwrap();
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
        for path in [original, alias, outside] { fs::remove_file(path).unwrap(); }
        fs::remove_dir(files.root()).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
