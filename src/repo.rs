use std::{
    collections::HashSet,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    iter::Peekable,
    mem::ManuallyDrop,
    path::{Components, Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    none,
    objects::{Commit, Object, TreeEntry},
    store::{Hash, HashDisplay, Store},
    trace,
    util::DropAction,
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
        trace!(options, "Repository::check()");

        let drop = DropAction(|| {
            trace!(options, "Repository::check() error");
        });

        self.store.check(
            HashSet::new(),
            &[self.info.head(), self.info.stage()],
            options,
        )?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::check() done");

        Ok(())
    }

    pub fn add(&mut self, path: impl AsRef<Path>, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::add({:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::add(...) error");
        });

        let canon = path
            .as_ref()
            .canonicalize()
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

        let hash = if canon.is_dir() {
            let hash = self.hash_dir(&canon, options)?;

            if relative == "" {
                verbose!(options, "Hashed contents of path.");

                verbose!(options, "Recomputed stage.");

                if self.info.stage() == hash {
                    verbose!(options, "New stage is equal to old stage.");
                } else {
                    self.info.set_stage(hash);
                }

                let _ = ManuallyDrop::new(drop);

                trace!(options, "Repository::add(...) done");

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

        trace!(options, "Repository::add(...) done");

        Ok(())
    }

    pub fn sub(&mut self, path: impl AsRef<Path>, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::sub({:?})", path.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::sub(...) error");
        });

        let canon = path
            .as_ref()
            .canonicalize()
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

        trace!(options, "Repository::sub(...) done");

        Ok(())
    }

    fn update_stage(
        &mut self,
        mut path: Peekable<Components>,
        obj: Option<Hash>,
        tree: Hash,
        options: &Cli,
    ) -> Result<Option<Hash>, EvsError> {
        let next = path.next().unwrap();

        trace!(
            options,
            "Repository::update_stage({:?}..., {}, \"{}\")",
            AsRef::<Path>::as_ref(&next),
            obj.is_some(),
            HashDisplay(&tree)
        );

        let drop = DropAction(|| {
            trace!(options, "Repository::update_stage(...) error");
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

        let hash = if path.peek().is_none() {
            obj
        } else {
            let next = match items.iter().find(|e| e.name == next_bytes) {
                Some(next) => next.content,
                None => {
                    if obj.is_none() {
                        todo!("Same error as later")
                    } else {
                        self.store.insert(Object::Tree(vec![]), options)?
                    }
                }
            };

            self.update_stage(path, obj, next, options)?
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
                todo!("Error for this")
            }
        };

        verbose!(options, "Obtained hash or lack thereof of this component.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::update_stage(...) done");

        Ok(hash)
    }

    fn hash_dir(&self, path: &PathBuf, options: &Cli) -> Result<Hash, EvsError> {
        trace!(options, "Repository::hash_dir({:?})", path);

        let drop = DropAction(|| {
            trace!(options, "Repository::hash_dir(...) error");
        });

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

                if next.starts_with(&self.repository) {
                    verbose!(options, "Skipping repository...");

                    continue;
                }

                let hash = self.hash_dir(&next, options)?;

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

        trace!(options, "Repository::hash_dir(...) done");

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
            "Repository::commit(<msg of len {}>, {}, {}, {:?})",
            message.len(),
            name,
            email,
            time
        );

        let drop = DropAction(|| {
            trace!(options, "Repository::commit(...) error");
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

        trace!(options, "Repository::commit(...) done");

        Ok(commit)
    }

    pub fn lookup(
        &self,
        r#ref: impl AsRef<str>,
        options: &Cli,
    ) -> Result<(Hash, Object), EvsError> {
        trace!(options, "Repository::lookup(\"{}\")", r#ref.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::lookup(...) error");
        });

        let resolved = self.resolve(r#ref, options)?;

        verbose!(options, "Resolved to \"{}\".", resolved);

        let result = self.store.lookup(resolved.as_str(), options)?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::lookup(...) done");

        Ok(result)
    }

    pub fn log(&self, r#ref: impl AsRef<str>, limit: usize, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Repository::log(\"{}\")", r#ref.as_ref());

        let drop = DropAction(|| {
            trace!(options, "Repository::log(...) error");
        });

        let mut resolved = self.resolve(r#ref, options)?;

        verbose!(options, "Resolved to \"{}\".", resolved);

        for _ in 0..limit {
            let (hash, commit) = self.store.lookup(&resolved, options)?;

            match commit {
                Object::Null => break,
                Object::Commit(Commit { parent, .. }) => {
                    println!("{}:\n{}", HashDisplay(&hash), commit);

                    resolved = format!("{}", HashDisplay(&parent));
                }
                _ => return Err(EvsError::NotACommit(hash)),
            }

            verbose!(options, "Continuing with \"{}\"", resolved);
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::log(...) done");

        Ok(())
    }

    pub fn resolve(&self, r#ref: impl AsRef<str>, options: &Cli) -> Result<String, EvsError> {
        trace!(options, "Repository::resolve(\"{}\")", r#ref.as_ref());

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

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Repository::resolve(...) done");

        Ok(resolved)
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
