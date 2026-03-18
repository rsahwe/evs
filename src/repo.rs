use std::{
    collections::HashSet,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write, stdout},
    iter::{Peekable, once},
    mem::ManuallyDrop,
    path::{Components, Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::Cli,
    diff::DiffSide,
    error::{CorruptState, EvsError},
    log, none,
    objects::{Commit, Object, TreeEntry},
    store::{Hash, HashDisplay, Store},
    trace,
    util::{
        ADD_COLOR, DropAction, INFO_COLOR, MOD_COLOR, NONE_COLOR, SUB_COLOR, SizeDisplay,
        confirmation, get_color,
    },
    verbose,
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
    pub fn open(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        trace!(options, "Repository::open({:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::open(...) error");
        });

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        verbose!(options, "Workspace exists and is a directory.");

        let repo = path.as_ref().join(".evs");

        if !repo.exists() {
            return Err(EvsError::MissingRepository(repo));
        }

        if !repo.is_dir() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::DirectoryIsFile(repo),
            ));
        }

        verbose!(options, "Repository exists and is a directory.");

        let repo = repo.canonicalize().expect("repo exists and is a directory");

        verbose!(options, "Repository directory was canonicalized.");

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

        verbose!(options, "Store exists and is a directory.");

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

        verbose!(options, "Successfully obtained lock.");

        let mut repo_info = vec![];

        lockfile
            .read_to_end(&mut repo_info)
            .map_err(|e| (e, lockfile_path.clone()))?;

        let repo_info =
            rmp_serde::from_slice(&repo_info).map_err(|e| EvsError::RepositoryInfoCorrupt(e))?;

        verbose!(options, "Read repository info successfully.");

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store: Store::new(store),
            info: repo_info,
        };

        verbose!(options, "Created repository.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::open(...) done");

        Ok(repository)
    }

    pub fn create(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        trace!(options, "Repository::create({:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::create(...) error");
        });

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        verbose!(options, "Workspace exists and is a directory.");

        let repo = path.as_ref().join(".evs");

        DirBuilder::new()
            .create(&repo)
            .map_err(|e| (e, repo.clone()))?;

        verbose!(options, "Created repository directory.");

        let repo = repo.canonicalize().expect("repo dir was just created");

        verbose!(options, "Repository directory was canonicalized.");

        let store = repo.join("store");

        DirBuilder::new()
            .create(&store)
            .map_err(|e| (e, store.clone()))?;

        verbose!(options, "Created store directory.");

        let store = Store::new(store);

        let root = store.insert(Object::Null, options)?;

        verbose!(options, "Inserted null object.");

        let empty_stage = store.insert(Object::Tree(vec![]), options)?;

        verbose!(options, "Inserted empty tree.");

        let lockfile_path = repo.join("lock");

        let mut lockfile = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lockfile_path)
            .map_err(|e| (e, lockfile_path.clone()))?;

        lockfile.try_lock().map_err(|e| (e, repo.clone()))?;

        verbose!(options, "Created and locked lockfile.");

        let repo_info = RepositoryInfo {
            head: root,
            stage: empty_stage,
            modified: false,
        };

        lockfile
            .write_all(&rmp_serde::to_vec(&repo_info).expect("msgpack failed"))
            .map_err(|e| (e, lockfile_path.clone()))?;

        verbose!(options, "Wrote repository info into the lockfile.");

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store,
            info: repo_info,
        };

        verbose!(options, "Created repository.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::create(...) done");

        Ok(repository)
    }

    pub fn find(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        trace!(options, "Repository::find({:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::find(...) error");
        });

        let mut path = path
            .as_ref()
            .canonicalize()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        verbose!(options, "Canonicalized path.");

        loop {
            verbose!(options, "Trying path {:?}:", path);

            match Self::open(&path, options) {
                Ok(repo) => {
                    verbose!(options, "Found repository in {:?}.", path);

                    let _ = ManuallyDrop::new(drop);

                    trace!(options, "Repository::find(...) done");

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

    pub fn check(&self, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::check(self)");

        let drop = DropAction(|| {
            trace!(options, "Repository::check(self) error");
        });

        self.store.check(
            HashSet::new(),
            &[self.info.head(), self.info.stage()],
            None,
            options,
        )?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::check(self) done");

        Ok(())
    }

    pub fn add(
        &mut self,
        path: impl AsRef<Path>,
        overrides: &HashSet<PathBuf>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        trace!(options, "Repository::add(self, {:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::add(self, ...) error");
        });

        let canon = path
            .as_ref()
            .canonicalize()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        verbose!(options, "Canonicalized path to {:?}", canon);

        if !canon.starts_with(&self.workspace) {
            return Err(EvsError::PathOutsideOfRepo(canon));
        }

        let relative = canon.strip_prefix(&self.workspace).unwrap();

        let ignores = self.get_ignores(options)?;

        verbose!(options, "Using ignores: {:?}.", ignores);

        if ignores.iter().any(|i| relative.starts_with(i)) && !overrides.contains(relative) {
            if relative.starts_with(".evs")
                || !confirmation(
                    &format!("{:?} is ignored, add anyway?", relative),
                    false,
                    options,
                )?
            {
                verbose!(options, "Filtered path {:?}.", relative);

                let _ = ManuallyDrop::new(drop);

                trace!(options, "Repository::add(self, ...) done");

                return Ok(());
            }
        }

        let hash = if canon.is_dir() {
            let hash = self.hash_dir(&canon, ignores, overrides, options)?;

            if relative == "" {
                verbose!(options, "Hashed contents of path.");

                verbose!(options, "Recomputed stage.");

                if self.info.stage() == hash {
                    verbose!(options, "New stage is equal to old stage.");
                } else {
                    self.info.set_stage(hash);
                }

                let _ = ManuallyDrop::new(drop);

                trace!(options, "Repository::add(self, ...) done");

                return Ok(());
            }

            hash
        } else {
            self.store.insert(
                Object::Blob(fs::read(&canon).map_err(|e| (e, canon.clone()))?),
                options,
            )?
        };

        verbose!(options, "Hashed contents of path.");

        let new_stage = match self.update_stage(
            relative.components().peekable(),
            relative,
            Some(hash),
            self.info.stage(),
            options,
        )? {
            Some(stage) => stage,
            None => self.store.insert(Object::Tree(vec![]), options)?,
        };

        verbose!(options, "Recomputed stage.");

        if self.info.stage() == new_stage {
            verbose!(options, "New stage is equal to old stage.")
        } else {
            self.info.set_stage(new_stage);
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::add(self, ...) done");

        Ok(())
    }

    pub fn sub(&mut self, path: impl AsRef<Path>, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::sub(self, {:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::sub(self, ...) error");
        });

        let canon = path
            .as_ref()
            .canonicalize() //TODO: ALTERNATIVE WITH PARTIAL CANONICALIZE (CUSTOM?)
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        verbose!(options, "Canonicalized path to {:?}", canon);

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
            self.store.insert(Object::Tree(vec![]), options)?
        } else {
            match self.update_stage(
                relative.components().peekable(),
                relative,
                None,
                self.info.stage(),
                options,
            )? {
                Some(stage) => stage,
                None => self.store.insert(Object::Tree(vec![]), options)?,
            }
        };

        verbose!(options, "Recomputed stage.");

        if self.info.stage() == new_stage {
            verbose!(options, "New stage is equal to old stage.")
        } else {
            self.info.set_stage(new_stage);
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::sub(self, ...) done");

        Ok(())
    }

    fn update_stage(
        &mut self,
        mut components: Peekable<Components>,
        path: impl AsRef<Path>,
        obj: Option<Hash>,
        tree: Hash,
        options: &Cli,
    ) -> Result<Option<Hash>, EvsError> {
        let next = components.next().unwrap();

        let path = path.as_ref();

        trace!(
            options,
            "Repository::update_stage(self, {:?}..., {}, \"{}\")",
            AsRef::<Path>::as_ref(&next),
            obj.is_some(),
            HashDisplay(&tree)
        );

        let drop = DropAction(|| {
            trace!(options, "Repository::update_stage(self, ...) error");
        });

        let next_bytes = AsRef::<Path>::as_ref(&next).as_os_str().as_encoded_bytes();

        let mut items = match self
            .store
            .lookup(&format!("{}", HashDisplay(&tree)), options)
        {
            Ok((_, Object::Tree(items))) => items,
            Ok((hash, _)) => {
                verbose!(
                    options,
                    "Replacing object \"{}\" with new tree!",
                    HashDisplay(&hash)
                );

                vec![]
            }
            Err(e) => return Err(e),
        };

        verbose!(options, "Obtained {} tree item(s).", items.len());

        let hash = if components.peek().is_none() {
            obj
        } else {
            let next = match items.iter().find(|e| e.name == next_bytes) {
                Some(next) => next.content,
                None => {
                    if obj.is_none() {
                        return Err(EvsError::PathNotInStage(path.to_path_buf()));
                    } else {
                        self.store.insert(Object::Tree(vec![]), options)?
                    }
                }
            };

            self.update_stage(components, path, obj, next, options)?
        };

        verbose!(
            options,
            "Obtained hash or lack thereof of later component(s)."
        );

        let hash = if let Some(obj) = hash {
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name == next_bytes).then_some(i))
            {
                if items[index].content == obj {
                    verbose!(options, "Object unchanged.");

                    Some(tree)
                } else {
                    items[index].content = obj;

                    verbose!(options, "Object changed, adding new tree to store...");

                    Some(self.store.insert(Object::Tree(items), options)?)
                }
            } else {
                items.push(TreeEntry {
                    name: next_bytes.to_owned(),
                    content: obj,
                });

                verbose!(options, "Tree changed, adding tree to store...");

                Some(self.store.insert(Object::Tree(items), options)?)
            }
        } else {
            if let Some(index) = items
                .iter()
                .enumerate()
                .find_map(|(i, e)| (e.name == next_bytes).then_some(i))
            {
                items.remove(index);

                verbose!(options, "Deleted object.");

                if items.len() == 0 {
                    verbose!(options, "Empty tree pruned.");

                    None
                } else {
                    verbose!(options, "Tree changed, adding tree to store...");

                    Some(self.store.insert(Object::Tree(items), options)?)
                }
            } else {
                return Err(EvsError::PathNotInStage(path.to_path_buf()));
            }
        };

        verbose!(options, "Obtained hash or lack thereof of this component.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::update_stage(self, ...) done");

        Ok(hash)
    }

    fn hash_dir(
        &self,
        path: &PathBuf,
        ignores: impl AsRef<[PathBuf]>,
        overrides: &HashSet<PathBuf>,
        options: &Cli,
    ) -> Result<Hash, EvsError> {
        trace!(
            options,
            "Repository::hash_dir(self, {:?}, {:?})",
            path,
            ignores.as_ref()
        );

        verbose!(options, "CHANGE");

        let drop = DropAction(|| {
            trace!(options, "Repository::hash_dir(self, ...) error");
        });

        let ignores = ignores.as_ref();

        let res = if !path.is_dir() {
            let content = fs::read(path).map_err(|e| (e, path.to_owned()))?;

            verbose!(options, "Read blob, inserting...");

            self.store.insert(Object::Blob(content), options)?
        } else {
            let mut items = vec![];

            for child in path.read_dir().map_err(|e| (e, path.to_owned()))? {
                let child = child.map_err(|e| (e, path.to_owned()))?;

                let name = child.file_name();

                let name_bytes = name.as_encoded_bytes().to_owned();

                let next = path.join(&name);

                let relative = next.strip_prefix(&self.workspace).unwrap();

                if ignores.iter().any(|i| i == relative)
                    && !overrides.iter().any(|o| o.starts_with(relative))
                {
                    verbose!(options, "Filtered child {:?}.", name);

                    continue;
                }

                let hash = self.hash_dir(&next, ignores, overrides, options)?;

                verbose!(options, "Hashed child {:?}.", name);

                items.push(TreeEntry {
                    name: name_bytes,
                    content: hash,
                });
            }

            verbose!(options, "Inserting resulting tree...");

            self.store.insert(Object::Tree(items), options)?
        };

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::hash_dir(self, ...) done");

        Ok(res)
    }

    pub fn commit(
        &mut self,
        message: String,
        name: String,
        email: String,
        time: SystemTime,
        options: &Cli,
    ) -> Result<Hash, EvsError> {
        trace!(
            options,
            "Repository::commit(self, <msg of len {}>, {}, {}, {:?})",
            message.len(),
            name,
            email,
            time
        );

        let drop = DropAction(|| {
            trace!(options, "Repository::commit(self, ...) error");
        });

        let commit = self.store.insert(
            Object::Commit(Commit {
                parent: self.info.head(),
                name,
                email,
                tree: self.info.stage(),
                msg: message,
                date: time,
            }),
            options,
        )?;

        verbose!(options, "Created and inserted commit object.");

        self.info.set_head(commit);

        verbose!(options, "Moved head to \"{}\".", HashDisplay(&commit));

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::commit(self, ...) done");

        Ok(commit)
    }

    pub fn lookup(
        &self,
        r#ref: impl AsRef<str>,
        options: &Cli,
    ) -> Result<(Hash, Object), EvsError> {
        trace!(options, "Repository::lookup(self, \"{}\")", r#ref.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::lookup(self, ...) error");
        });

        let resolved = self.resolve(r#ref, options)?;

        verbose!(options, "Resolved to \"{}\".", resolved);

        let result = self.store.lookup(resolved.as_str(), options)?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::lookup(self, ...) done");

        Ok(result)
    }

    pub fn log(
        &self,
        r#ref: impl AsRef<str>,
        limit: usize,
        oneline: bool,
        options: &Cli,
    ) -> Result<(), EvsError> {
        trace!(options, "Repository::log(self, \"{}\")", r#ref.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::log(self, ...) error");
        });

        let print_color = get_color(options);

        let mod_color = if print_color { MOD_COLOR } else { "" };
        let info_color = if print_color { INFO_COLOR } else { "" };
        let none_color = if print_color { NONE_COLOR } else { "" };

        let mut resolved = self.resolve(r#ref, options)?;

        verbose!(options, "Resolved to \"{}\".", resolved);

        'broken: {
            for _ in 0..limit {
                let (hash, commit) = self.store.lookup(&resolved, options)?;

                match commit {
                    Object::Null => break 'broken,
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

                verbose!(options, "Continuing with \"{}\"", resolved);
            }

            println!("{}...{}", info_color, none_color);
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::log(self, ...) done");

        Ok(())
    }

    pub fn resolve(&self, r#ref: impl AsRef<str>, options: &Cli) -> Result<String, EvsError> {
        trace!(options, "Repository::resolve(self, \"{}\")", r#ref.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::resolve(...) error");
        });

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

        verbose!(options, "Starting at \"{}\".", first);

        let mut resolved = first;

        for _ in 0..back_count {
            let (hash, commit) = self.store.lookup(resolved.as_str(), options)?;

            resolved = match commit {
                Object::Commit(Commit { parent, .. }) => format!("{}", HashDisplay(&parent)),
                Object::Null => return Err(EvsError::NoPreviousCommit),
                _ => return Err(EvsError::NotACommit(hash)),
            };

            verbose!(options, "Gone back to \"{}\".", resolved)
        }

        let resolved = self.store.resolve_rest(resolved, options)?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::resolve(self, ...) done");

        Ok(resolved)
    }

    pub fn gc(&self, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::gc(self)");

        let drop = DropAction(|| {
            trace!(options, "Repository::gc(self) error");
        });

        let mut dependencies = None;

        self.store.check(
            HashSet::new(),
            &[self.info.head(), self.info.stage()],
            Some(&mut dependencies),
            options,
        )?;

        let dependencies = dependencies.as_mut().unwrap();

        verbose!(
            options,
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

            verbose!(options, "Adding \"{}\" to the deletion list.", hash_display);

            match self.store.lookup(&hash_display, options)?.1 {
                Object::Null => (),
                Object::Blob(_) => {
                    blob_count += 1;
                }
                Object::Tree(items) => {
                    for item in items {
                        verbose!(
                            options,
                            "Decrementing rc for \"{}\".",
                            HashDisplay(&item.content)
                        );

                        dependencies.insert(
                            item.content,
                            dependencies.get(&item.content).unwrap_or(&1) - 1,
                        );
                    }

                    tree_count += 1;
                }
                Object::Commit(commit) => {
                    verbose!(
                        options,
                        "Decrementing rc for \"{}\".",
                        HashDisplay(&commit.tree)
                    );

                    dependencies.insert(
                        commit.tree,
                        dependencies.get(&commit.tree).unwrap_or(&1) - 1,
                    );

                    verbose!(
                        options,
                        "Decrementing rc for \"{}\".",
                        HashDisplay(&commit.parent)
                    );

                    dependencies.insert(
                        commit.parent,
                        dependencies.get(&commit.parent).unwrap_or(&1) - 1,
                    );

                    commit_count += 1;
                }
            }
        }

        if deletion_list.len() != 0 {
            none!(
                "This will delete {} objects: ({} commits, {} trees, {} blobs)",
                deletion_list.len(),
                commit_count,
                tree_count,
                blob_count
            );

            if confirmation("Are you sure?", true, options)? {
                log!(options, "Deleting {} objects...", deletion_list.len());

                for item in deletion_list {
                    verbose!(options, "Deleting {}", HashDisplay(&item));

                    self.store.remove(item, options)?;
                }
            }
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::gc(self) done");

        Ok(())
    }

    pub fn get_tree(&self, commit: Hash, options: &Cli) -> Result<Hash, EvsError> {
        trace!(
            options,
            "Repository::get_tree(self, \"{}\")",
            HashDisplay(&commit)
        );

        let drop = DropAction(|| {
            trace!(options, "Repository::get_tree(self, ...) error");
        });

        let (hash, commit) = self
            .store
            .lookup(&format!("{}", HashDisplay(&commit)), options)?;

        verbose!(options, "Found referenced object.");

        let commit = match commit {
            Object::Commit(commit) => commit,
            _ => return Err(EvsError::NotACommit(hash)),
        };

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::get_tree(self, ...) done");

        Ok(commit.tree)
    }

    pub fn get_ignores(&self, options: &Cli) -> Result<Vec<PathBuf>, EvsError> {
        trace!(options, "Repository::get_ignores(self)");

        let drop = DropAction(|| {
            trace!(options, "Repository::get_ignores(self) error");
        });

        let ignores_file = self.workspace.join(".evsignore");

        let content = if ignores_file.exists() {
            verbose!(options, "Ignores file exists, trying to read...");

            fs::read_to_string(&ignores_file).map_err(|e| (e, ignores_file))?
        } else {
            verbose!(options, "Missing ignores file substituted with \"\".");

            String::new()
        };

        verbose!(options, "Read ignores file successfully.");

        let result = content
            .lines()
            .map(str::trim)
            .filter(|l| l.len() != 0)
            .chain(once(".evs"))
            .map(PathBuf::from)
            .collect();

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::get_ignores(self) ok");

        Ok(result)
    }

    pub fn status(&self, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::status(self)");

        let drop = DropAction(|| {
            trace!(options, "Repository::status(self) error");
        });

        let (store_count, store_size) = self.store.status()?;

        verbose!(
            options,
            "Store reported {} objects with a collective {} bytes.",
            store_count,
            store_size
        );

        let (repo_head, repo_stage) = (self.info.head(), self.info.stage());

        let commit_diffside = DiffSide::Tree(self.get_tree(repo_head, options)?);

        let stage_diffside = DiffSide::Tree(repo_stage);

        let local_diffside = DiffSide::Local(self.workspace.clone());

        verbose!(options, "Prepared diffsides.");

        let ignores = self.get_ignores(options)?;

        let global_filter = [AsRef::<Path>::as_ref("").to_path_buf()];

        let empty_set = HashSet::new();

        let cds = commit_diffside.read(
            "",
            &self.store,
            &global_filter,
            &ignores,
            &empty_set,
            options,
        )?;

        let sds = stage_diffside.read(
            "",
            &self.store,
            &global_filter,
            &ignores,
            &empty_set,
            options,
        )?;

        let lds =
            local_diffside.read("", &self.store, &global_filter, &ignores, &sds.0, options)?;

        verbose!(
            options,
            "Read diffsides: {:?} -> {:?} -> {:?}.",
            cds.0,
            sds.0,
            lds.0
        );

        let (stage_added, stage_modified, stage_removed) = (
            sds.0.difference(&cds.0).collect(),
            sds.0
                .intersection(&cds.0)
                .filter(|k| cds.1.get(*k).unwrap() != sds.1.get(*k).unwrap())
                .collect(),
            cds.0.difference(&sds.0).collect(),
        );

        verbose!(options, "Generated stage diff.");

        let (local_added, local_modified, local_removed) = (
            lds.0.difference(&sds.0).collect(),
            lds.0
                .intersection(&sds.0)
                .filter(|k| sds.1.get(*k).unwrap() != lds.1.get(*k).unwrap())
                .collect(),
            sds.0.difference(&lds.0).collect(),
        );

        verbose!(options, "Generated local diff.");

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

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::status(self) ok");

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
            none!("Writing back Repository Info failed: {}", err);
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
