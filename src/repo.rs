use std::{
    collections::HashSet,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write, stdout},
    iter::{Peekable, once},
    path::{Components, Path, PathBuf},
    time::SystemTime,
};

use glob::Pattern;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, instrument, trace, warn};

use crate::{
    cli::Cli,
    confirmation,
    diff::DiffSide,
    error::{CorruptState, EvsError},
    objects::{Commit, Object, TreeEntry},
    store::{Hash, HashDisplay, Store},
    util::{ADD_COLOR, INFO_COLOR, MOD_COLOR, NONE_COLOR, SUB_COLOR, SizeDisplay, get_color},
};

#[derive(Debug)]
pub struct Repository {
    pub workspace: PathBuf,
    pub repository: PathBuf,
    pub lockfile: File,
    pub store: Store,
    pub info: RepositoryInfo,
}

impl Repository {
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn open(path: impl AsRef<Path>, _options: &Cli) -> Result<Repository, EvsError> {
        debug!("Repository::open({:?})", path.as_ref());

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        trace!("Workspace exists and is a directory.");

        let repo = path.as_ref().join(".evs");

        if !repo.exists() {
            return Err(EvsError::MissingRepository(repo));
        }

        if !repo.is_dir() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::DirectoryIsFile(repo),
            ));
        }

        trace!("Repository exists and is a directory.");

        let repo = repo.canonicalize().expect("repo exists and is a directory");

        trace!("Repository directory was canonicalized.");

        let store = repo.join("store");

        if !store.exists() {
            return Err(EvsError::CorruptStateDetected(CorruptState::MissingPath(
                store,
            )));
        }

        if !store.is_dir() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::DirectoryIsFile(store),
            ));
        }

        trace!("Store exists and is a directory.");

        let lockfile_path = repo.join("lock");

        if !lockfile_path.exists() {
            return Err(EvsError::CorruptStateDetected(CorruptState::MissingPath(
                lockfile_path,
            )));
        }

        if !lockfile_path.is_file() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::FileIsDirectory(lockfile_path),
            ));
        }

        let mut lockfile = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lockfile_path)
            .map_err(|e| (e, lockfile_path.clone()))?;

        lockfile.try_lock().map_err(|e| (e, repo.clone()))?;

        trace!("Successfully obtained lock.");

        let mut repo_info = vec![];

        lockfile
            .read_to_end(&mut repo_info)
            .map_err(|e| (e, lockfile_path.clone()))?;

        let repo_info =
            rmp_serde::from_slice(&repo_info).map_err(|e| EvsError::RepositoryInfoCorrupt(e))?;

        trace!("Read repository info successfully.");

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store: Store::new(store),
            info: repo_info,
        };

        trace!("Created repository.");

        Ok(repository)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn create(path: impl AsRef<Path>, _options: &Cli) -> Result<Repository, EvsError> {
        debug!("Repository::create(self, {:?})", path.as_ref());

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        trace!("Workspace exists and is a directory.");

        let repo = path.as_ref().join(".evs");

        DirBuilder::new()
            .create(&repo)
            .map_err(|e| (e, repo.clone()))?;

        trace!("Created repository directory.");

        let repo = repo.canonicalize().expect("repo dir was just created");

        trace!("Repository directory was canonicalized.");

        let store = repo.join("store");

        DirBuilder::new()
            .create(&store)
            .map_err(|e| (e, store.clone()))?;

        trace!("Created store directory.");

        let store = Store::new(store);

        let root = store.insert(Object::Null)?;

        trace!("Inserted null object.");

        let empty_stage = store.insert(Object::Tree(vec![]))?;

        trace!("Inserted empty tree.");

        let lockfile_path = repo.join("lock");

        let mut lockfile = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lockfile_path)
            .map_err(|e| (e, lockfile_path.clone()))?;

        lockfile.try_lock().map_err(|e| (e, repo.clone()))?;

        trace!("Created and locked lockfile.");

        let repo_info = RepositoryInfo {
            head: root,
            stage: empty_stage,
            modified: false,
        };

        lockfile
            .write_all(&rmp_serde::to_vec(&repo_info).expect("msgpack failed"))
            .map_err(|e| (e, lockfile_path.clone()))?;

        trace!("Wrote repository info into the lockfile.");

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store,
            info: repo_info,
        };

        trace!("Created repository.");

        Ok(repository)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn find(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        debug!("Repository::find({:?})", path.as_ref());

        let mut path = path
            .as_ref()
            .canonicalize()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        trace!("Canonicalized path.");

        loop {
            trace!("Trying path {:?}:", path);

            match Self::open(&path, options) {
                Ok(repo) => {
                    trace!("Found repository in {:?}.", path);

                    return Ok(repo);
                }
                Err(e) => match e {
                    EvsError::MissingRepository(_) => (),
                    _ => return Err(e),
                },
            }

            if !path.pop() {
                return Err(EvsError::RepositoryNotFound);
            }
        }
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn check(&self) -> Result<(), EvsError> {
        debug!("Repository::check(self)");

        self.store
            .check(HashSet::new(), &[self.info.head(), self.info.stage()], None)
            .map(|_| ())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn add(
        &mut self,
        path: impl AsRef<Path>,
        overrides: &HashSet<PathBuf>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!(
            "Repository::add(self, {:?}, {:?})",
            path.as_ref(),
            overrides
        );

        let canon = path
            .as_ref()
            .canonicalize()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        trace!("Canonicalized path to {:?}", canon);

        if !canon.starts_with(&self.workspace) {
            return Err(EvsError::PathOutsideOfRepo(canon));
        }

        let relative = canon.strip_prefix(&self.workspace).unwrap();

        let ignores = self.get_ignores(options)?;

        trace!("Using ignores: {:?}.", ignores);

        if ignores
            .iter()
            .any(|i| relative.ancestors().any(|a| i.matches_path(a)))
            && !overrides.contains(relative)
        {
            if relative.starts_with(".evs")
                || !confirmation!(false, "{:?} is ignored, add anyway?", relative)?
            {
                trace!("Filtered path {:?}.", relative);

                return Ok(());
            }
        }

        let hash = if canon.is_dir() {
            let hash = self.hash_dir(&canon, ignores, overrides)?;

            if relative == "" {
                trace!("Hashed contents of path.");

                trace!("Recomputed stage.");

                if self.info.stage() == hash {
                    trace!("New stage is equal to old stage.");
                } else {
                    self.info.set_stage(hash);
                }

                return Ok(());
            }

            hash
        } else {
            self.store.insert(Object::Blob(
                fs::read(&canon).map_err(|e| (e, canon.clone()))?,
            ))?
        };

        trace!("Hashed contents of path.");

        let new_stage = match self.update_stage(
            relative.components().peekable(),
            relative,
            Some(hash),
            self.info.stage(),
        )? {
            Some(stage) => stage,
            None => self.store.insert(Object::Tree(vec![]))?,
        };

        trace!("Recomputed stage.");

        if self.info.stage() == new_stage {
            trace!("New stage is equal to old stage.")
        } else {
            self.info.set_stage(new_stage);
        }

        Ok(())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn sub(&mut self, path: impl AsRef<Path>) -> Result<(), EvsError> {
        debug!("Repository::sub(self, {:?})", path.as_ref());

        let canon = path
            .as_ref()
            .canonicalize() //TODO: ALTERNATIVE WITH PARTIAL CANONICALIZE (CUSTOM?)
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        trace!("Canonicalized path to {:?}", canon);

        if !canon.starts_with(
            self.repository
                .parent()
                .expect("repository should have parent"),
        ) {
            return Err(EvsError::PathOutsideOfRepo(canon));
        }

        if canon.starts_with(&self.repository) {
            return Err(EvsError::PathOutsideOfRepo(canon));
        }

        let relative = canon
            .strip_prefix(self.repository.parent().unwrap())
            .unwrap();

        let new_stage = if relative == "" {
            self.store.insert(Object::Tree(vec![]))?
        } else {
            match self.update_stage(
                relative.components().peekable(),
                relative,
                None,
                self.info.stage(),
            )? {
                Some(stage) => stage,
                None => self.store.insert(Object::Tree(vec![]))?,
            }
        };

        trace!("Recomputed stage.");

        if self.info.stage() == new_stage {
            trace!("New stage is equal to old stage.")
        } else {
            self.info.set_stage(new_stage);
        }

        Ok(())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    fn update_stage(
        &mut self,
        mut components: Peekable<Components>,
        path: impl AsRef<Path>,
        obj: Option<Hash>,
        tree: Hash,
    ) -> Result<Option<Hash>, EvsError> {
        let next = components.next().unwrap();

        debug!(
            "Repository::update_stage(self, {:?}, {:?}..., {}, {})",
            path.as_ref(),
            next,
            obj.map(|_| "inserting").unwrap_or("deleting"),
            HashDisplay(&tree)
        );

        let path = path.as_ref();

        let next_bytes = AsRef::<Path>::as_ref(&next).as_os_str().as_encoded_bytes();

        let mut items = match self.store.lookup(&format!("{}", HashDisplay(&tree))) {
            Ok((_, Object::Tree(items))) => items,
            Ok((hash, _)) => {
                trace!("Replacing object \"{}\" with new tree.", HashDisplay(&hash));

                vec![]
            }
            Err(e) => return Err(e),
        };

        trace!("Obtained {} tree item(s).", items.len());

        let hash = if components.peek().is_none() {
            obj
        } else {
            let next = match items.iter().find(|e| e.name == next_bytes) {
                Some(next) => next.content,
                None => {
                    if obj.is_none() {
                        return Err(EvsError::PathNotInStage(path.to_path_buf()));
                    } else {
                        self.store.insert(Object::Tree(vec![]))?
                    }
                }
            };

            self.update_stage(components, path, obj, next)?
        };

        trace!("Obtained hash or lack thereof of later component(s).");

        let hash = if let Some(obj) = hash {
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name == next_bytes).then_some(i))
            {
                if items[index].content == obj {
                    trace!("Object unchanged.");

                    Some(tree)
                } else {
                    items[index].content = obj;

                    trace!("Object changed, adding new tree to store...");

                    Some(self.store.insert(Object::Tree(items))?)
                }
            } else {
                items.push(TreeEntry {
                    name: next_bytes.to_owned(),
                    content: obj,
                });

                trace!("Tree changed, adding tree to store...");

                Some(self.store.insert(Object::Tree(items))?)
            }
        } else {
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name == next_bytes).then_some(i))
            {
                items.remove(index);

                trace!("Deleted object.");

                if items.len() == 0 {
                    trace!("Empty tree pruned.");

                    None
                } else {
                    trace!("Tree changed, adding tree to store...");

                    Some(self.store.insert(Object::Tree(items))?)
                }
            } else {
                return Err(EvsError::PathNotInStage(path.to_path_buf()));
            }
        };

        trace!("Obtained hash or lack thereof of this component.");

        Ok(hash)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    fn hash_dir(
        &self,
        path: &PathBuf,
        ignores: impl AsRef<[Pattern]>,
        overrides: &HashSet<PathBuf>,
    ) -> Result<Hash, EvsError> {
        debug!(
            "Repository::hash_dir(self, {:?}, {} ignores, {:?})",
            path,
            ignores.as_ref().len(),
            overrides
        );

        let ignores = ignores.as_ref();

        let res = if !path.is_dir() {
            let content = fs::read(path).map_err(|e| (e, path.to_owned()))?;

            trace!("Read blob, inserting...");

            self.store.insert(Object::Blob(content))?
        } else {
            let mut items = vec![];

            for child in path.read_dir().map_err(|e| (e, path.to_owned()))? {
                let child = child.map_err(|e| (e, path.to_owned()))?;

                let name = child.file_name();

                let name_bytes = name.as_encoded_bytes().to_owned();

                let next = path.join(&name);

                let relative = next.strip_prefix(&self.workspace).unwrap();

                if ignores
                    .iter()
                    .any(|i| relative.ancestors().any(|a| i.matches_path(a)))
                    && !overrides.iter().any(|o| o.starts_with(relative))
                {
                    trace!("Filtered child {:?}.", name);

                    continue;
                }

                let hash = self.hash_dir(&next, ignores, overrides)?;

                trace!("Hashed child {:?}.", name);

                items.push(TreeEntry {
                    name: name_bytes,
                    content: hash,
                });
            }

            trace!("Inserting resulting tree...");

            self.store.insert(Object::Tree(items))?
        };

        Ok(res)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn commit(
        &mut self,
        message: String,
        name: String,
        email: String,
        time: SystemTime,
        _options: &Cli,
    ) -> Result<Hash, EvsError> {
        debug!(
            "Repository::commit(self, \"{}\", {}, {}, {:?})",
            message.as_bytes().escape_ascii(),
            name,
            email,
            time
        );

        let commit = self.store.insert(Object::Commit(Commit {
            parent: self.info.head(),
            name,
            email,
            tree: self.info.stage(),
            msg: message,
            date: time,
        }))?;

        trace!("Created and inserted commit object.");

        self.info.set_head(commit);

        trace!("Moved head to \"{}\".", HashDisplay(&commit));

        Ok(commit)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn lookup(&self, r#ref: impl AsRef<str>) -> Result<(Hash, Object), EvsError> {
        debug!("Repository::lookup(self, \"{}\")", r#ref.as_ref());

        let resolved = self.resolve(r#ref)?;

        trace!("Resolved to \"{}\".", resolved);

        self.store.lookup(resolved.as_str())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn log(
        &self,
        r#ref: impl AsRef<str>,
        limit: usize,
        oneline: bool,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!(
            "Repository::log(self, \"{}\", {}, {})",
            r#ref.as_ref(),
            limit,
            oneline
        );

        let print_color = get_color(options);

        let mod_color = if print_color { MOD_COLOR } else { "" };
        let info_color = if print_color { INFO_COLOR } else { "" };
        let none_color = if print_color { NONE_COLOR } else { "" };

        let mut resolved = self.resolve(r#ref)?;

        trace!("Resolved to \"{}\".", resolved);

        for _ in 0..limit {
            let (hash, commit) = self.store.lookup(&resolved)?;

            match commit {
                Object::Null => return Ok(()),
                Object::Commit(Commit {
                    parent, ref msg, ..
                }) => {
                    if oneline {
                        println!(
                            "{}{}{}: {}{}{}",
                            info_color,
                            HashDisplay(&hash),
                            none_color,
                            mod_color,
                            msg.lines().next().unwrap_or(""),
                            none_color
                        );
                    } else {
                        println!(
                            "{}{}{}:\n{}{}{}",
                            info_color,
                            HashDisplay(&hash),
                            none_color,
                            mod_color,
                            commit,
                            none_color
                        );
                    }

                    resolved = format!("{}", HashDisplay(&parent));
                }
                _ => return Err(EvsError::NotACommit(hash)),
            }

            trace!("Continuing with \"{}\"", resolved);
        }

        println!("{}...{}", info_color, none_color);

        Ok(())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn resolve(&self, r#ref: impl AsRef<str>) -> Result<String, EvsError> {
        debug!("Repository::resolve(self, \"{}\")", r#ref.as_ref());

        let (first, back_count) = r#ref
            .as_ref()
            .split_once('~')
            .unwrap_or((r#ref.as_ref(), "0"));

        let back_count = back_count
            .parse::<usize>()
            .map_err(|e| EvsError::IntegerParseError(e))?;

        let first = match first {
            "HEAD" => format!("{}", HashDisplay(&self.info.head())),
            first => first.to_owned(),
        };

        trace!("Starting at \"{}\".", first);

        let mut resolved = first;

        for _ in 0..back_count {
            let (hash, commit) = self.store.lookup(resolved.as_str())?;

            resolved = match commit {
                Object::Commit(Commit { parent, .. }) => format!("{}", HashDisplay(&parent)),
                Object::Null => return Err(EvsError::NoPreviousCommit),
                _ => return Err(EvsError::NotACommit(hash)),
            };

            trace!("Gone back to \"{}\".", resolved)
        }

        self.store.resolve_rest(resolved)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn gc(&self, _options: &Cli) -> Result<(), EvsError> {
        debug!("Repository::gc(self)");

        let mut dependencies = None;

        self.store.check(
            HashSet::new(),
            &[self.info.head(), self.info.stage()],
            Some(&mut dependencies),
        )?;

        let dependencies = dependencies.as_mut().unwrap();

        trace!(
            "Checked store and obtained {} dependencies.",
            dependencies.len()
        );

        let mut deletion_list = vec![];

        let mut tree_count = 0;
        let mut commit_count = 0;
        let mut blob_count = 0;

        while let Some((k, _)) = dependencies.iter().find(|(_, v)| **v == 0) {
            let k = *k;

            deletion_list.push(k);

            dependencies.remove(&k);

            let hash_display = format!("{}", HashDisplay(&k));

            trace!("Adding \"{}\" to the deletion list.", hash_display);

            match self.store.lookup(&hash_display)?.1 {
                Object::Null => (),
                Object::Blob(_) => {
                    blob_count += 1;
                }
                Object::Tree(items) => {
                    for item in items {
                        trace!("Decrementing rc for \"{}\".", HashDisplay(&item.content));

                        dependencies.insert(
                            item.content,
                            dependencies.get(&item.content).unwrap_or(&1) - 1,
                        );
                    }

                    tree_count += 1;
                }
                Object::Commit(commit) => {
                    trace!("Decrementing rc for \"{}\".", HashDisplay(&commit.tree));

                    dependencies.insert(
                        commit.tree,
                        dependencies.get(&commit.tree).unwrap_or(&1) - 1,
                    );

                    trace!("Decrementing rc for \"{}\".", HashDisplay(&commit.parent));

                    dependencies.insert(
                        commit.parent,
                        dependencies.get(&commit.parent).unwrap_or(&1) - 1,
                    );

                    commit_count += 1;
                }
            }
        }

        if deletion_list.len() != 0 {
            println!(
                "This will delete {} object(s): ({} commit(s), {} tree(s), {} blob(s))",
                deletion_list.len(),
                commit_count,
                tree_count,
                blob_count
            );

            if confirmation!(true, "Are you sure?")? {
                warn!("Deleting {} object(s)...", deletion_list.len());

                for item in deletion_list {
                    trace!("Deleting {}", HashDisplay(&item));

                    self.store.remove(item)?;
                }
            }
        }

        Ok(())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn get_tree(&self, commit: Hash) -> Result<Hash, EvsError> {
        debug!("Repository::get_tree(self, \"{}\")", HashDisplay(&commit));

        let (hash, commit) = self.store.lookup(&format!("{}", HashDisplay(&commit)))?;

        trace!("Found referenced object.");

        let commit = match commit {
            Object::Commit(commit) => commit,
            _ => return Err(EvsError::NotACommit(hash)),
        };

        Ok(commit.tree)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn get_ignores(&self, _options: &Cli) -> Result<Vec<Pattern>, EvsError> {
        debug!("Repository::get_ignores(self)");

        let ignores_file = self.workspace.join(".evsignore");

        let content = if ignores_file.exists() {
            trace!("Ignores file exists, trying to read...");

            fs::read_to_string(&ignores_file).map_err(|e| (e, ignores_file))?
        } else {
            trace!("Missing ignores file substituted with \"\".");

            String::new()
        };

        trace!("Read ignores file successfully.");

        content
            .lines()
            .map(str::trim)
            .filter(|l| l.len() != 0)
            .chain(once(".evs"))
            .map(Pattern::new)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn status(&self, options: &Cli) -> Result<(), EvsError> {
        debug!("Repository::status(self)");

        let (store_count, store_size) = self.store.status()?;

        trace!(
            "Store reported {} objects with a collective {} bytes.",
            store_count, store_size
        );

        let (repo_head, repo_stage) = (self.info.head(), self.info.stage());

        let commit_diffside = DiffSide::Tree(self.get_tree(repo_head)?);

        let stage_diffside = DiffSide::Tree(repo_stage);

        let local_diffside = DiffSide::Local(self.workspace.clone());

        trace!("Prepared diffsides.");

        let ignores = self.get_ignores(options)?;

        let global_filter = [AsRef::<Path>::as_ref("").to_path_buf()];

        let empty_set = HashSet::new();

        let cds = commit_diffside.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        let sds = stage_diffside.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        let lds = local_diffside.read("", &self.store, &global_filter, &ignores, &sds.0)?;

        trace!("Read diffsides: {:?} -> {:?} -> {:?}.", cds.0, sds.0, lds.0);

        let (stage_added, stage_modified, stage_removed) = (
            sds.0.difference(&cds.0).collect(),
            sds.0
                .intersection(&cds.0)
                .filter(|k| cds.1.get(*k).unwrap() != sds.1.get(*k).unwrap())
                .collect(),
            cds.0.difference(&sds.0).collect(),
        );

        trace!("Generated stage diff.");

        let (local_added, local_modified, local_removed) = (
            lds.0.difference(&sds.0).collect(),
            lds.0
                .intersection(&sds.0)
                .filter(|k| sds.1.get(*k).unwrap() != lds.1.get(*k).unwrap())
                .collect(),
            sds.0.difference(&lds.0).collect(),
        );

        trace!("Generated local diff.");

        self.print_info(
            store_count,
            store_size,
            repo_head,
            repo_stage,
            stage_added,
            stage_modified,
            stage_removed,
            local_added,
            local_modified,
            local_removed,
            get_color(options),
        );

        Ok(())
    }

    pub fn print_info(
        &self,
        store_count: usize,
        store_size: usize,
        repo_head: Hash,
        repo_stage: Hash,
        stage_added: Vec<&PathBuf>,
        stage_modified: Vec<&PathBuf>,
        stage_removed: Vec<&PathBuf>,
        local_added: Vec<&PathBuf>,
        local_modified: Vec<&PathBuf>,
        local_removed: Vec<&PathBuf>,
        print_color: bool,
    ) {
        let add_color = if print_color { ADD_COLOR } else { "" };
        let sub_color = if print_color { SUB_COLOR } else { "" };
        let mod_color = if print_color { MOD_COLOR } else { "" };
        let none_color = if print_color { NONE_COLOR } else { "" };

        println!("  Head is at \"{}\"", HashDisplay(&repo_head));
        println!("  and stage is \"{}\"", HashDisplay(&repo_stage));
        println!(
            "  Store has {} objects with size {}",
            store_count,
            SizeDisplay(store_size, print_color)
        );
        if stage_added.len() + stage_modified.len() + stage_removed.len() > 0 {
            println!();
            println!("  Staged changes:");
            print!("{}", add_color);
            for addition in stage_added {
                println!("    added {:?}", addition);
            }
            print!("{}", mod_color);
            for modification in stage_modified {
                println!("    modified {:?}", modification);
            }
            print!("{}", sub_color);
            for deletion in stage_removed {
                println!("    removed {:?}", deletion);
            }
        }
        if local_added.len() + local_modified.len() + local_removed.len() > 0 {
            println!("{}", none_color);
            println!("  Unstaged changes:");
            print!("{}", add_color);
            for addition in local_added {
                println!("    added {:?}", addition);
            }
            print!("{}", mod_color);
            for modification in local_modified {
                println!("    modified {:?}", modification);
            }
            print!("{}", sub_color);
            for deletion in local_removed {
                println!("    removed {:?}", deletion);
            }
        }
        print!("{}", none_color);

        let _ = stdout().flush();
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let r = || -> Result<(), io::Error> {
            if self.info.modified {
                self.lockfile.set_len(0)?;
                self.lockfile.seek(SeekFrom::Start(0))?;
                self.lockfile
                    .write_all(&rmp_serde::to_vec(&self.info).expect("msgpack failed"))?;
            }

            Ok(())
        }();

        if let Err(err) = r {
            error!("Writing back Repository Info failed: {}", err);
        }
    }
}

/// All of the info about the repository
#[derive(Serialize, Deserialize, Debug)]
pub struct RepositoryInfo {
    head: Hash,
    stage: Hash,
    #[serde(skip)]
    modified: bool,
}

impl RepositoryInfo {
    pub fn head(&self) -> Hash {
        self.head
    }

    pub fn set_head(&mut self, new_head: Hash) {
        self.head = new_head;
        self.modified = true;
    }

    pub fn stage(&self) -> Hash {
        self.stage
    }

    pub fn set_stage(&mut self, new_stage: Hash) {
        self.stage = new_stage;
        self.modified = true;
    }
}
