//! Backups of AiWin11's own data (ROADMAP R2, "sauvegardes ciblées / points de restauration").
//!
//! A backup is a consistent copy of the SQLite store made with `VACUUM INTO`, kept in the
//! `backups` folder next to it (the user's local application data, never sent anywhere).
//! Memory entries stay DPAPI-encrypted for this Windows user inside the copy. A restore is
//! only *scheduled* while AiWin11 runs and is applied at the next launch, before the store is
//! opened, after the backup was checked; the current store is saved first.

use std::{
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::MIGRATIONS;

/// Most backups kept; the oldest ones are removed after a new backup.
pub const MAX_BACKUPS: usize = 5;
const PREFIX: &str = "aiwin11-";
const SUFFIX: &str = ".sqlite3";
const PENDING: &str = "restore.pending";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    /// File name: `aiwin11-YYYYMMDD-HHMMSS-<reason>.sqlite3`.
    pub name: String,
    pub bytes: u64,
}

/// Why a backup was made, part of its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Manual,
    BeforeMigration,
    BeforeRestore,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::BeforeMigration => "before_migration",
            Self::BeforeRestore => "before_restore",
        }
    }
}

pub fn backup_dir(store: &Path) -> PathBuf {
    store.with_file_name("backups")
}

/// Names AiWin11 itself produced: no path, only `[a-z0-9_-]` between prefix and suffix.
pub fn valid_backup_name(name: &str) -> bool {
    name.len() <= 80
        && name.strip_prefix(PREFIX).and_then(|rest| rest.strip_suffix(SUFFIX)).is_some_and(|middle| {
            !middle.is_empty()
                && middle
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
        })
}

/// Writes a consistent copy of `db` into `dir`, then keeps only the newest `MAX_BACKUPS`.
pub(crate) fn create(db: &Connection, dir: &Path, reason: Reason) -> Result<BackupInfo, &'static str> {
    fs::create_dir_all(dir).map_err(|_| "BACKUP_FAILED")?;
    let stamp: String =
        db.query_row("SELECT strftime('%Y%m%d-%H%M%f', 'now')", [], |row| row.get(0)).map_err(|_| "BACKUP_FAILED")?;
    // `%f` is "SS.SSS": keep digits only so names sort by time.
    let stamp: String = stamp.chars().filter(|c| c.is_ascii_digit() || *c == '-').collect();
    let name = format!("{PREFIX}{stamp}-{}{SUFFIX}", reason.code().replace('_', "-"));
    let path = dir.join(&name);
    if path.exists() {
        return Err("BACKUP_FAILED");
    }
    let target = path.to_str().ok_or("BACKUP_FAILED")?;
    db.execute("VACUUM INTO ?1", [target]).map_err(|_| "BACKUP_FAILED")?;
    prune(dir);
    let bytes = fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
    Ok(BackupInfo { name, bytes })
}

fn prune(dir: &Path) {
    let mut backups = list(dir);
    while backups.len() > MAX_BACKUPS {
        let oldest = backups.pop().expect("non-empty");
        let _ = fs::remove_file(dir.join(oldest.name));
    }
}

/// Backups in `dir`, newest first. Anything that is not an AiWin11 backup is ignored.
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut backups: Vec<BackupInfo> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let metadata = entry.metadata().ok()?;
            (valid_backup_name(&name) && metadata.is_file()).then_some(BackupInfo { name, bytes: metadata.len() })
        })
        .collect();
    backups.sort_by(|left, right| right.name.cmp(&left.name));
    backups
}

/// Checks that a backup is a sound AiWin11 store this version can open.
pub fn validate(path: &Path) -> Result<(), &'static str> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|_| "BACKUP_INVALID")?;
    let integrity: String =
        db.query_row("PRAGMA integrity_check", [], |row| row.get(0)).map_err(|_| "BACKUP_INVALID")?;
    let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(|_| "BACKUP_INVALID")?;
    if integrity != "ok" || version == 0 || version as usize > MIGRATIONS.len() {
        return Err("BACKUP_INVALID");
    }
    Ok(())
}

/// Asks for `name` to replace the store at the next launch.
pub fn schedule_restore(store: &Path, name: &str) -> Result<(), &'static str> {
    if !valid_backup_name(name) {
        return Err("BACKUP_INVALID");
    }
    validate(&backup_dir(store).join(name))?;
    fs::write(store.with_file_name(PENDING), name).map_err(|_| "BACKUP_FAILED")
}

pub fn cancel_restore(store: &Path) {
    let _ = fs::remove_file(store.with_file_name(PENDING));
}

pub fn pending_restore(store: &Path) -> Option<String> {
    fs::read_to_string(store.with_file_name(PENDING))
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| valid_backup_name(name))
}

/// Applied before the store is opened. The request is consumed first, so a failing restore is
/// never retried in a loop. Returns `None` when nothing was scheduled.
pub fn apply_pending_restore(store: &Path) -> Option<Result<(), &'static str>> {
    let marker = store.with_file_name(PENDING);
    let requested = fs::read_to_string(&marker).ok()?;
    let _ = fs::remove_file(&marker);
    Some(restore(store, requested.trim()))
}

fn restore(store: &Path, name: &str) -> Result<(), &'static str> {
    if !valid_backup_name(name) {
        return Err("BACKUP_INVALID");
    }
    let dir = backup_dir(store);
    let source = dir.join(name);
    validate(&source)?;
    if store.exists() {
        // The current store is kept as a backup of its own before being replaced.
        let current = Connection::open(store).map_err(|_| "BACKUP_FAILED")?;
        create(&current, &dir, Reason::BeforeRestore)?;
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = fs::remove_file(PathBuf::from(format!("{}{suffix}", store.display())));
    }
    fs::copy(&source, store).map_err(|_| "BACKUP_FAILED")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Core;

    fn temp_store() -> (PathBuf, PathBuf) {
        let folder = std::env::temp_dir().join(format!("aiwin11-backup-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&folder).unwrap();
        let store = folder.join("aiwin11.sqlite3");
        (folder, store)
    }

    #[test]
    fn backups_are_valid_listed_newest_first_and_pruned() {
        let (folder, store) = temp_store();
        let core = Core::open(&store).unwrap();
        let dir = backup_dir(&store);
        let mut made = Vec::new();
        for _ in 0..=MAX_BACKUPS {
            made.push(core.backup(&dir).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let listed = list(&dir);
        assert_eq!(listed.len(), MAX_BACKUPS, "oldest removed");
        assert_eq!(listed[0], *made.last().unwrap());
        assert!(!listed.contains(&made[0]));
        validate(&dir.join(&listed[0].name)).unwrap();
        assert_eq!(core.activity().unwrap()[0].kind, "data.backup.created");
        fs::write(dir.join("autre.txt"), "x").unwrap();
        assert_eq!(list(&dir).len(), MAX_BACKUPS, "foreign files ignored");
        drop(core);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_scheduled_restore_replaces_the_store_at_next_launch_and_keeps_the_current_one() {
        let (folder, store) = temp_store();
        let dir = backup_dir(&store);
        let name = {
            let core = Core::open(&store).unwrap();
            core.backup(&dir).unwrap().name
        };
        {
            let mut core = Core::open(&store).unwrap();
            core.set_cloud_token_budget(20_000).unwrap();
        }
        schedule_restore(&store, &name).unwrap();
        assert_eq!(pending_restore(&store).as_deref(), Some(name.as_str()));
        assert_eq!(apply_pending_restore(&store), Some(Ok(())));
        assert_eq!(apply_pending_restore(&store), None, "consumed");
        let core = Core::open(&store).unwrap();
        assert_eq!(core.cloud_token_budget(), 50_000, "setting made after the backup is gone");
        assert!(list(&dir).iter().any(|backup| backup.name.ends_with("-before-restore.sqlite3")));
        drop(core);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn invalid_backups_and_names_are_refused() {
        let (folder, store) = temp_store();
        let dir = backup_dir(&store);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("aiwin11-20260101-000000-manual.sqlite3"), "not a database").unwrap();
        assert_eq!(schedule_restore(&store, "aiwin11-20260101-000000-manual.sqlite3"), Err("BACKUP_INVALID"));
        for name in
            ["../aiwin11-x.sqlite3", "aiwin11-.sqlite3", "aiwin11-X.sqlite3", "autre.sqlite3", "aiwin11-a/b.sqlite3"]
        {
            assert!(!valid_backup_name(name), "{name}");
            assert_eq!(schedule_restore(&store, name), Err("BACKUP_INVALID"));
        }
        // A marker naming a bad backup is consumed and reported, the store is untouched.
        fs::write(store.with_file_name(PENDING), "aiwin11-20260101-000000-manual.sqlite3").unwrap();
        assert_eq!(apply_pending_restore(&store), Some(Err("BACKUP_INVALID")));
        assert_eq!(pending_restore(&store), None);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_older_store_is_backed_up_before_it_is_migrated() {
        let (folder, store) = temp_store();
        {
            let db = Connection::open(&store).unwrap();
            db.execute_batch(MIGRATIONS[0]).unwrap();
        }
        let core = Core::open(&store).unwrap();
        let backups = list(&backup_dir(&store));
        assert_eq!(backups.len(), 1);
        assert!(backups[0].name.ends_with("-before-migration.sqlite3"));
        drop(core);
        // A current store is not backed up again.
        Core::open(&store).unwrap();
        assert_eq!(list(&backup_dir(&store)).len(), 1);
        fs::remove_dir_all(folder).unwrap();
    }
}
