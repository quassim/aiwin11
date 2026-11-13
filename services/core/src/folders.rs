//! Several folders the agent may work in (ROADMAP R2, "plusieurs dossiers").
//!
//! The first folder is the primary one (AiWin11's dedicated folder or the folder chosen in the
//! profile): its files are named as before, `notes.txt`. Up to `MAX_EXTRA_FOLDERS` more folders
//! chosen by the user are reached with a short alias, `alias/notes.txt`. Each folder is a
//! `LocalFiles` store with the same checks (canonical paths, no links, supported formats); a
//! copy or rename never crosses folders. Every operation keeps its broker authorization and
//! audit from `LocalFiles`.

use std::path::{Path, PathBuf};

use crate::{
    broker::Policy,
    local_files::{LocalFiles, SearchHit, MAX_LISTED},
    Core,
};

/// Folders the user may add besides the primary one.
pub const MAX_EXTRA_FOLDERS: usize = 4;
const MAX_ALIAS: usize = 24;

pub struct FolderSet {
    /// `(alias, store)`; the primary folder has an empty alias.
    folders: Vec<(String, LocalFiles)>,
}

/// Alias of a folder: its name, lowercased, letters and digits kept, others turned into `-`.
pub fn alias_for(path: &Path) -> String {
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut folded = String::new();
    for c in name.chars() {
        // French accents folded; anything else that is not a letter or digit becomes one dash.
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if c != '-' || !folded.ends_with('-') {
            folded.push(c);
        }
    }
    let alias: String = folded.trim_matches('-').chars().take(MAX_ALIAS).collect();
    let alias = alias.trim_end_matches('-').to_owned();
    if alias.is_empty() {
        "dossier".into()
    } else {
        alias
    }
}

fn valid_alias(alias: &str) -> bool {
    (1..=MAX_ALIAS).contains(&alias.len())
        && alias.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl FolderSet {
    pub fn single(primary: LocalFiles) -> Self {
        Self { folders: vec![(String::new(), primary)] }
    }

    /// Adds the extra folders; their aliases are made unique (`docs`, `docs-2`).
    pub fn with_extra(primary: LocalFiles, data_dir: &Path, extra: &[PathBuf]) -> Self {
        let mut set = Self::single(primary);
        for path in extra.iter().take(MAX_EXTRA_FOLDERS) {
            let Ok(store) = LocalFiles::open_with_workspace(data_dir, path) else { continue };
            if set.folders.iter().any(|(_, known)| known.root() == store.root()) {
                continue;
            }
            let base = alias_for(store.root());
            let mut alias = base.clone();
            let mut index = 2;
            while set.folders.iter().any(|(known, _)| *known == alias) {
                alias = format!("{}-{index}", base.chars().take(MAX_ALIAS - 3).collect::<String>());
                index += 1;
            }
            set.folders.push((alias, store));
        }
        set
    }

    pub fn primary(&self) -> &LocalFiles {
        &self.folders[0].1
    }

    pub fn root(&self) -> &Path {
        self.primary().root()
    }

    pub fn trash(&self) -> &Path {
        self.primary().trash()
    }

    /// Aliases of the extra folders, in order.
    pub fn aliases(&self) -> Vec<&str> {
        self.folders.iter().skip(1).map(|(alias, _)| alias.as_str()).collect()
    }

    /// The store and the bare file name for `name` (`alias/file` or `file`).
    fn resolve<'a>(&self, name: &'a str) -> Result<(&str, &LocalFiles, &'a str), &'static str> {
        match name.split_once('/') {
            None => Ok(("", self.primary(), name)),
            Some((alias, file)) if valid_alias(alias) => self
                .folders
                .iter()
                .find(|(known, _)| known == alias)
                .map(|(known, store)| (known.as_str(), store, file))
                .ok_or("FOLDER_UNKNOWN"),
            Some(_) => Err("INVALID_FILE_NAME"),
        }
    }

    /// Source and destination in the same folder; a destination without alias stays in the
    /// source's folder.
    fn resolve_pair<'a>(
        &self,
        source: &'a str,
        destination: &'a str,
    ) -> Result<(&LocalFiles, &'a str, &'a str), &'static str> {
        let (alias, store, source) = self.resolve(source)?;
        let destination = match destination.split_once('/') {
            None => destination,
            Some((other, file)) if other == alias => file,
            Some(_) => return Err("FILE_FOLDER_MISMATCH"),
        };
        Ok((store, source, destination))
    }

    fn prefixed(alias: &str, name: String) -> String {
        if alias.is_empty() {
            name
        } else {
            format!("{alias}/{name}")
        }
    }

    pub fn agent_list(&self, core: &mut Core, task: &str, policy: Policy) -> Result<Vec<String>, &'static str> {
        let mut names = Vec::new();
        for (alias, store) in &self.folders {
            let listed = store.agent_list(core, task, policy)?;
            names.extend(listed.into_iter().map(|name| Self::prefixed(alias, name)));
            if names.len() >= MAX_LISTED {
                names.truncate(MAX_LISTED);
                break;
            }
        }
        Ok(names)
    }

    pub fn agent_search(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        query: &str,
    ) -> Result<Vec<SearchHit>, &'static str> {
        let mut hits = Vec::new();
        for (alias, store) in &self.folders {
            for hit in store.agent_search(core, task, policy, query)? {
                hits.push(SearchHit { name: Self::prefixed(alias, hit.name), lines: hit.lines });
            }
            if hits.len() >= 20 {
                hits.truncate(20);
                break;
            }
        }
        Ok(hits)
    }

    pub fn agent_read(&self, core: &mut Core, task: &str, policy: Policy, name: &str) -> Result<String, &'static str> {
        let (_, store, file) = self.resolve(name)?;
        store.agent_read(core, task, policy, file)
    }

    pub fn agent_resolve_document(
        &self,
        core: &Core,
        name: &str,
    ) -> Result<(PathBuf, std::fs::File, std::fs::File), &'static str> {
        let (_, store, file) = self.resolve(name)?;
        store.agent_resolve_document(core, file)
    }

    pub fn agent_trash_preview(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        name: &str,
    ) -> Result<usize, &'static str> {
        let (_, store, file) = self.resolve(name)?;
        store.agent_trash_preview(core, task, policy, file)
    }

    pub fn agent_trash(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        name: &str,
    ) -> Result<PathBuf, &'static str> {
        let (_, store, file) = self.resolve(name)?;
        store.agent_trash(core, task, policy, file)
    }

    pub fn agent_copy_preview(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<usize, &'static str> {
        let (store, source, destination) = self.resolve_pair(source, destination)?;
        store.agent_copy_preview(core, task, policy, source, destination)
    }

    pub fn agent_copy(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<(), &'static str> {
        let (store, source, destination) = self.resolve_pair(source, destination)?;
        store.agent_copy(core, task, policy, source, destination)
    }

    pub fn agent_write(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        name: &str,
        content: &str,
    ) -> Result<(), &'static str> {
        let (_, store, file) = self.resolve(name)?;
        store.agent_write(core, task, policy, file, content)
    }

    pub fn agent_move(
        &self,
        core: &mut Core,
        task: &str,
        policy: Policy,
        source: &str,
        destination: &str,
    ) -> Result<(), &'static str> {
        let (store, source, destination) = self.resolve_pair(source, destination)?;
        store.agent_move(core, task, policy, source, destination)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use uuid::Uuid;

    use super::*;
    use crate::broker::Capability;

    #[test]
    fn aliases_are_short_lowercase_and_unique() {
        assert_eq!(alias_for(Path::new(r"C:\Users\x\Mes Documents")), "mes-documents");
        assert_eq!(alias_for(Path::new(r"C:\Users\x\Été  2026")), "ete-2026");
        assert_eq!(alias_for(Path::new(r"C:\")), "dossier");
        assert!(valid_alias("docs-2") && !valid_alias("Docs") && !valid_alias("a/b") && !valid_alias(""));
    }

    #[test]
    fn names_route_to_their_folder_and_never_cross_folders() {
        let data = std::env::temp_dir().join(format!("aiwin11-folders-test-{}", Uuid::new_v4()));
        fs::create_dir(&data).unwrap();
        let primary = LocalFiles::open(&data).unwrap();
        // Extra folders must be inside the user profile: none is added from a temp folder.
        let set = FolderSet::with_extra(primary, &data, &[data.join("absent")]);
        assert!(set.aliases().is_empty());
        let mut set = set;
        let extra = data.join("extra");
        fs::create_dir(&extra).unwrap();
        // Tests build the second store directly; the profile check is LocalFiles' own.
        set.folders.push(("docs".into(), LocalFiles::open(&extra).unwrap()));
        fs::write(set.root().join("a.txt"), "alpha").unwrap();
        fs::write(set.folders[1].1.root().join("b.txt"), "beta").unwrap();

        let mut core = Core::start(Connection::open_in_memory().unwrap()).unwrap();
        let task = Uuid::new_v4().to_string();
        let all = Policy::granting(&[Capability::FileRead, Capability::FileCopy, Capability::FileMove]);
        assert_eq!(set.agent_list(&mut core, &task, all).unwrap(), ["a.txt", "docs/b.txt"]);
        assert_eq!(set.agent_read(&mut core, &task, all, "docs/b.txt").unwrap(), "beta");
        assert_eq!(set.agent_read(&mut core, &task, all, "a.txt").unwrap(), "alpha");
        assert_eq!(set.agent_read(&mut core, &task, all, "autre/b.txt"), Err("FOLDER_UNKNOWN"));
        assert_eq!(set.agent_read(&mut core, &task, all, "Docs/b.txt"), Err("INVALID_FILE_NAME"));
        assert_eq!(set.agent_copy(&mut core, &task, all, "docs/b.txt", "a-copie.txt"), Ok(()));
        assert!(set.folders[1].1.root().join("a-copie.txt").exists(), "a bare destination stays in the source folder");
        assert_eq!(set.agent_move(&mut core, &task, all, "docs/b.txt", "docs/c.txt"), Ok(()));
        assert_eq!(set.agent_move(&mut core, &task, all, "a.txt", "docs/a.txt"), Err("FILE_FOLDER_MISMATCH"));
        let hits = set.agent_search(&mut core, &task, all, "ta").unwrap();
        assert_eq!(hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(), ["docs/a-copie.txt", "docs/c.txt"]);
        fs::remove_dir_all(data).unwrap();
    }
}
