use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, ErrorKind, Read as _, Seek as _, SeekFrom, Write as _, stdout},
    iter::{Peekable, once},
    path::{Components, Path, PathBuf},
    time::SystemTime,
};

use ahash::AHashSet;
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
    util::{
        ADD_COLOR, INFO_COLOR, MOD_COLOR, NONE_COLOR, SUB_COLOR, SizeDisplay, get_color,
        partial_canonicalize,
    },
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
    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn open<T: AsRef<Path>>(
        path: T,
        options: &Cli,
    ) -> Result<Repository, EvsError> {
        debug!("Repository::open({:?})", path.as_ref());

        Self::open_(path.as_ref(), options)
    }

    fn open_(
        path: &Path,
        _options: &Cli,
    ) -> Result<Repository, EvsError> {
        let _ = path.read_dir().map_err(|e| (e, path.to_path_buf()))?;

        trace!("Workspace exists and is a directory.");

        let repo = path.join(".evs");

        if !repo.exists() {
            return Err(EvsError::MissingRepository(repo));
        }

        if !repo.is_dir() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::DirectoryIsFile(repo),
            ));
        }

        trace!("Repository exists and is a directory.");

        let repo = repo.canonicalize().map_err(|e| (e, repo))?;

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

        #[allow(
            clippy::verbose_file_reads,
            reason = "fs::read is not at all equivalent here."
        )]
        lockfile
            .read_to_end(&mut repo_info)
            .map_err(|e| (e, lockfile_path.clone()))?;

        let repo_info =
            rmp_serde::from_slice(&repo_info).map_err(EvsError::RepositoryInfoCorrupt)?;

        trace!("Read repository info successfully.");

        let repository = Repository {
            workspace: path.to_path_buf(),
            repository: repo,
            lockfile,
            store: Store::new(store),
            info: repo_info,
        };

        trace!("Created repository.");

        Ok(repository)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn create<T: AsRef<Path>>(
        path: T,
        options: &Cli,
    ) -> Result<Repository, EvsError> {
        debug!("Repository::create(self, {:?})", path.as_ref());

        Self::create_(path.as_ref(), options)
    }

    fn create_(
        path: &Path,
        _options: &Cli,
    ) -> Result<Repository, EvsError> {
        let _ = path.read_dir().map_err(|e| (e, path.to_path_buf()))?;

        trace!("Workspace exists and is a directory.");

        let repo = path.join(".evs");

        DirBuilder::new()
            .create(&repo)
            .map_err(|e| (e, repo.clone()))?;

        trace!("Created repository directory.");

        let repo = repo.canonicalize().map_err(|e| (e, repo))?;

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
            .write_all(&rmp_serde::to_vec(&repo_info)?)
            .map_err(|e| (e, lockfile_path.clone()))?;

        trace!("Wrote repository info into the lockfile.");

        let repository = Repository {
            workspace: path.to_path_buf(),
            repository: repo,
            lockfile,
            store,
            info: repo_info,
        };

        trace!("Created repository.");

        Ok(repository)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn find<T: AsRef<Path>>(
        path: T,
        options: &Cli,
    ) -> Result<Repository, EvsError> {
        debug!("Repository::find({:?})", path.as_ref());

        Self::find_(path.as_ref(), options)
    }

    fn find_(
        path: &Path,
        options: &Cli,
    ) -> Result<Repository, EvsError> {
        let mut path = path.canonicalize().map_err(|e| (e, path.to_path_buf()))?;

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

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn check(
        &self,
        all: bool,
    ) -> Result<(), EvsError> {
        debug!("Repository::check(self)");

        self.store
            .check::<&[Hash]>(AHashSet::new(), &self.gc_roots(), all)
            .map(|_| ())
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn add<T: AsRef<Path>>(
        &mut self,
        path: T,
        overrides: &AHashSet<PathBuf>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!(
            "Repository::add(self, {:?}, {:?})",
            path.as_ref(),
            overrides
        );

        self.add_(path.as_ref(), overrides, options)
    }

    fn add_(
        &mut self,
        path: &Path,
        overrides: &AHashSet<PathBuf>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        let canon = path.canonicalize().map_err(|e| (e, path.to_path_buf()))?;

        trace!("Canonicalized path to {:?}", canon);

        if !canon.starts_with(&self.workspace) {
            return Err(EvsError::PathOutsideOfRepo(canon));
        }

        let relative = canon.strip_prefix(&self.workspace).unwrap();

        let ignores = self.get_ignores(options)?;

        trace!("Using ignores: {:?}.", ignores);

        let is_ignored = ignores
            .iter()
            .any(|i| relative.ancestors().any(|a| i.matches_path(a)));

        if is_ignored
            && !overrides.contains(relative)
            && (relative.starts_with(".evs")
                || !confirmation!(false, "{:?} is ignored, add anyway?", relative)?)
        {
            trace!("Filtered path {:?}.", relative);

            return Ok(());
        }

        let hash = if canon.is_dir() {
            let ignores = if is_ignored { &vec![] } else { &ignores };

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
            trace!("New stage is equal to old stage.");
        } else {
            self.info.set_stage(new_stage);
        }

        Ok(())
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn sub<T: AsRef<Path>>(
        &mut self,
        path: T,
    ) -> Result<(), EvsError> {
        debug!("Repository::sub(self, {:?})", path.as_ref());

        self.sub_(path.as_ref())
    }

    fn sub_(
        &mut self,
        path: &Path,
    ) -> Result<(), EvsError> {
        let canon = partial_canonicalize(path).map_err(|e| (e, path.to_path_buf()))?;

        trace!("Canonicalized path to {:?}", canon);

        if !canon.starts_with(&self.workspace) {
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
            trace!("New stage is equal to old stage.");
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
            obj.map_or("deleting", |_| "inserting"),
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
            let next = if let Some(next) = items.iter().find(|e| e.name.as_bytes() == next_bytes) {
                next.content
            } else {
                if obj.is_none() {
                    return Err(EvsError::PathNotInStage(path.to_path_buf()));
                }
                self.store.insert(Object::Tree(vec![]))?
            };

            self.update_stage(components, path, obj, next)?
        };

        trace!("Obtained hash or lack thereof of later component(s).");

        let hash = if let Some(obj) = hash {
            #[allow(clippy::indexing_slicing, reason = "The index comes from enumerate.")]
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name.as_bytes() == next_bytes).then_some(i))
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
                    name: String::from_utf8(next_bytes.to_owned())
                        .map_err(|e| EvsError::PathError(e.utf8_error(), e.into_bytes()))?,
                    content: obj,
                });

                trace!("Tree changed, adding tree to store...");

                Some(self.store.insert(Object::Tree(items))?)
            }
        } else {
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name.as_bytes() == next_bytes).then_some(i))
            {
                items.remove(index);

                trace!("Deleted object.");

                if items.is_empty() {
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
        overrides: &AHashSet<PathBuf>,
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
                    name: String::from_utf8(name_bytes)
                        .map_err(|e| EvsError::PathError(e.utf8_error(), e.into_bytes()))?,
                    content: hash,
                });
            }

            trace!("Inserting resulting tree...");

            self.store.insert(Object::Tree(items))?
        };

        Ok(res)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn commit(
        &mut self,
        amend_parent: Option<Hash>,
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
            parent: amend_parent.unwrap_or(self.info.head()),
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

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn lookup<T: AsRef<str>>(
        &self,
        r#ref: T,
    ) -> Result<(Hash, Object), EvsError> {
        debug!("Repository::lookup(self, \"{}\")", r#ref.as_ref());

        let resolved = self.resolve(r#ref)?;

        trace!("Resolved to \"{}\".", resolved);

        self.store.lookup(resolved.as_str())
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn log<T: AsRef<str>>(
        &self,
        r#ref: T,
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

        self.log_(r#ref.as_ref(), limit, oneline, options)
    }

    fn log_(
        &self,
        r#ref: &str,
        limit: usize,
        oneline: bool,
        options: &Cli,
    ) -> Result<(), EvsError> {
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

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn resolve<T: AsRef<str>>(
        &self,
        r#ref: T,
    ) -> Result<String, EvsError> {
        debug!("Repository::resolve(self, \"{}\")", r#ref.as_ref());

        self.resolve_(r#ref.as_ref())
    }

    fn resolve_(
        &self,
        r#ref: &str,
    ) -> Result<String, EvsError> {
        let (first, back_count) = r#ref.split_once('~').unwrap_or((r#ref, "0"));

        let back_count = back_count
            .parse::<usize>()
            .map_err(EvsError::IntegerParseError)?;

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

            trace!("Gone back to \"{}\".", resolved);
        }

        self.store.resolve_rest(resolved)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn gc(
        &self,
        _options: &Cli,
    ) -> Result<(), EvsError> {
        debug!("Repository::gc(self)");

        let (_, extra) = self
            .store
            .check::<&[Hash]>(AHashSet::new(), &self.gc_roots(), true)?;

        trace!("Checked store and obtained {} extras.", extra.len());

        if !extra.is_empty() {
            println!("This will delete {} object(s)", extra.len(),);

            if confirmation!(true, "Are you sure?")? {
                warn!("Deleting {} object(s)...", extra.len());

                for item in extra {
                    trace!("Deleting {}", HashDisplay(&item));

                    self.store.remove(item)?;
                }
            }
        }

        Ok(())
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn get_tree(
        &self,
        commit: Hash,
    ) -> Result<Hash, EvsError> {
        debug!("Repository::get_tree(self, \"{}\")", HashDisplay(&commit));

        let (hash, commit) = self.store.lookup(&format!("{}", HashDisplay(&commit)))?;

        trace!("Found referenced object.");

        Ok(match commit {
            Object::Null => self.store.insert(Object::Tree(vec![]))?,
            Object::Commit(commit) => commit.tree,
            _ => return Err(EvsError::NotACommit(hash)),
        })
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn get_ignores(
        &self,
        _options: &Cli,
    ) -> Result<Vec<Pattern>, EvsError> {
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
            .filter(|l| !l.is_empty())
            .chain(once(".evs"))
            .map(Pattern::new)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn status(
        &self,
        options: &Cli,
    ) -> Result<(), EvsError> {
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

        let empty_set = AHashSet::new();

        let cds = commit_diffside.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        let sds = stage_diffside.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        let lds = local_diffside.read("", &self.store, &global_filter, &ignores, &sds.0)?;

        trace!("Read diffsides: {:?} -> {:?} -> {:?}.", cds.0, sds.0, lds.0);

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        let (stage_added, stage_modified, stage_removed) = (
            sds.0.difference(&cds.0).collect(),
            sds.0
                .intersection(&cds.0)
                .filter(|k| cds.1[*k] != sds.1[*k])
                .collect(),
            cds.0.difference(&sds.0).collect(),
        );

        trace!("Generated stage diff.");

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        let (local_added, local_modified, local_removed) = (
            lds.0.difference(&sds.0).collect(),
            lds.0
                .intersection(&sds.0)
                .filter(|k| sds.1[*k] != lds.1[*k])
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

    #[inline]
    #[allow(clippy::too_many_arguments, reason = "This is fine.")]
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
        if !stage_added.is_empty() || !stage_modified.is_empty() || !stage_removed.is_empty() {
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
        if !local_added.is_empty() || !local_modified.is_empty() || !local_removed.is_empty() {
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

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn show<T: AsRef<str>>(
        &self,
        r#ref: T,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!("Repository::show(self, \"{}\")", r#ref.as_ref());

        self.show_(r#ref.as_ref(), options)
    }

    fn show_(
        &self,
        r#ref: &str,
        options: &Cli,
    ) -> Result<(), EvsError> {
        let (hash, commit) = self.lookup(r#ref)?;

        trace!("Found commit \"{}\".", HashDisplay(&hash));

        let commit = match commit {
            Object::Null => return Ok(()),
            Object::Commit(commit) => commit,
            _ => return Err(EvsError::NotACommit(hash)),
        };

        let rhs = DiffSide::Tree(commit.tree);

        let lhs = DiffSide::Tree(self.get_tree(commit.parent)?);

        trace!("Diffing...");

        DiffSide::diff_with(
            lhs,
            rhs,
            &self.store,
            &[AsRef::<Path>::as_ref("").to_path_buf()],
            &[],
            options,
        )
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn checkout<T: AsRef<str>>(
        &mut self,
        r#ref: T,
        force: bool,
        options: &Cli,
    ) -> Result<Hash, EvsError> {
        debug!("Repository::checkout(self, \"{}\")", r#ref.as_ref());

        self.checkout_(r#ref.as_ref(), force, options)
    }

    fn checkout_(
        &mut self,
        r#ref: &str,
        force: bool,
        options: &Cli,
    ) -> Result<Hash, EvsError> {
        let (hash, _) = self.lookup(r#ref)?;

        trace!("Found commit \"{}\".", HashDisplay(&hash));

        let dest_tree = self.get_tree(hash)?;

        trace!("Destination tree \"{}\".", HashDisplay(&dest_tree));

        let mut src_tree = self.get_tree(self.info.head())?;

        trace!("Source tree \"{}\".", HashDisplay(&src_tree));

        if src_tree != self.info.stage() {
            if force
                || confirmation!(
                    false,
                    "There are uncommitted changes, do you want to discard them?"
                )?
            {
                warn!("Discarding current stage...");

                src_tree = self.info.stage();
            } else {
                return Err(EvsError::UncommittedChanges);
            }
        }

        let dd = DiffSide::Tree(dest_tree);

        let ds = DiffSide::Tree(src_tree);

        let dl = DiffSide::Local(self.workspace.clone());

        trace!("Prepared diffsides.");

        let global_filter = [AsRef::<Path>::as_ref("").to_path_buf()];

        let empty_set = AHashSet::new();

        let ignores = self.get_ignores(options)?;

        let ds = ds.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        let dl = dl.read("", &self.store, &global_filter, &ignores, &ds.0)?;

        let dd = dd.read("", &self.store, &global_filter, &ignores, &empty_set)?;

        trace!("Read diffsides.");

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        if ds.0.difference(&dl.0).count() > 0
            || dl.0.difference(&ds.0).any(|k| dd.0.contains(k))
            || ds.0.intersection(&dl.0).any(|k| ds.1[k] != dl.1[k])
        {
            if force
                || confirmation!(
                    false,
                    "There are unstaged changes, do you want to discard them?"
                )?
            {
                warn!("Discarding local changes...");
            } else {
                return Err(EvsError::UncommittedChanges);
            }
        }

        self.info.set_head(hash);

        //TODO: SET BRANCH OR SOMETHING

        self.info.set_stage(dest_tree);

        trace!("Modified repository info.");

        for file in ds.0.difference(&dd.0) {
            let file = self.workspace.join(file);

            trace!("Deleting file {:?}...", file);

            fs::remove_file(&file).map_err(|e| (e, file.clone()))?;

            for ancestor in file.ancestors().skip(1) {
                if let Ok(dir) = ancestor.read_dir()
                    && dir.count() == 0
                {
                    trace!("Pruning empty dir {:?}...", ancestor);

                    if fs::remove_dir(ancestor).is_err() {
                        break;
                    }
                }
            }
        }

        trace!("Deleted files...");

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        for file in dd.0.difference(&ds.0) {
            let content = &dd.1[file];

            let file = self.workspace.join(file);

            // workspace is parent
            let parent = file.parent().unwrap();

            trace!("Creating dir {:?}...", parent);

            fs::create_dir_all(parent).map_err(|e| (e, file.clone()))?;

            trace!("Creating file {:?}...", file);

            fs::write(&file, content).map_err(|e| (e, file))?;
        }

        trace!("Created new files...");

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        for (file, _, content) in
            ds.0.intersection(&dd.0)
                .map(|k| (k, &ds.1[k], &dd.1[k]))
                .filter(|(_, lhs, rhs)| lhs != rhs)
        {
            let file = self.workspace.join(file);

            trace!("Modifying file {:?}...", file);

            fs::write(&file, content).map_err(|e| (e, file))?;
        }

        trace!("Modified files...");

        trace!("Checkout complete.");

        Ok(hash)
    }

    #[inline]
    #[must_use]
    pub fn gc_roots(&self) -> Vec<Hash> {
        vec![self.info.head(), self.info.stage()]
    }
}

impl Drop for Repository {
    #[inline]
    fn drop(&mut self) {
        let r = || -> Result<(), io::Error> {
            if self.info.modified {
                self.lockfile.set_len(0)?;
                self.lockfile.seek(SeekFrom::Start(0))?;
                self.lockfile
                    .write_all(&rmp_serde::to_vec(&self.info).map_err(|_e| {
                        io::Error::new(ErrorKind::InvalidData, "encoder failed")
                    })?)?;
            }

            Ok(())
        }();

        if let Err(err) = r {
            error!("Writing back Repository Info failed: {}", err);
        }
    }
}

/// All of the info about the repository.
#[derive(Serialize, Deserialize, Debug)]
pub struct RepositoryInfo {
    head: Hash,
    stage: Hash,
    #[serde(skip)]
    modified: bool,
}

impl RepositoryInfo {
    #[inline]
    #[must_use]
    pub fn head(&self) -> Hash {
        self.head
    }

    #[inline]
    pub fn set_head(
        &mut self,
        new_head: Hash,
    ) {
        self.modified = self.head != new_head;
        self.head = new_head;
    }

    #[inline]
    #[must_use]
    pub fn stage(&self) -> Hash {
        self.stage
    }

    #[inline]
    pub fn set_stage(
        &mut self,
        new_stage: Hash,
    ) {
        self.modified = self.stage != new_stage;
        self.stage = new_stage;
    }
}
