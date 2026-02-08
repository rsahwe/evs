use std::{
    collections::HashSet,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    iter::Peekable,
    mem::ManuallyDrop,
    path::{Components, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    none,
    objects::{Object, TreeEntry},
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
            serde_cbor::from_slice(&repo_info).map_err(|e| EvsError::RepositoryInfoCorrupt(e))?;

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
            .write_all(&serde_cbor::to_vec(&repo_info).expect("cbor failed"))
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
                    verbose!(options, "Found repository at {:?}.", path);

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
            let hash = self.hash_dir(&canon)?;

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

    fn hash_dir(&self, path: &PathBuf) -> Result<Hash, EvsError> {
        todo!("Hash directory {:?}", path)
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let r = || -> Result<(), io::Error> {
            if self.info.modified {
                self.lockfile.set_len(0)?;
                self.lockfile.seek(SeekFrom::Start(0))?;
                self.lockfile
                    .write_all(&serde_cbor::to_vec(&self.info).expect("cbor failed"))?;
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

    pub fn stage(&self) -> Hash {
        self.stage
    }

    pub fn set_stage(&mut self, new_stage: Hash) {
        self.stage = new_stage;
        self.modified = true;
    }
}
