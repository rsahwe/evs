use std::{
    collections::HashSet,
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    mem::ManuallyDrop,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    none,
    objects::Object,
    store::{Hash, Store},
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

        let store = repo.join("store");

        DirBuilder::new()
            .create(&store)
            .map_err(|e| (e, store.clone()))?;

        verbose!(options, "Created store directory.");

        let store = Store::new(store);

        let root = store.insert(
            &serde_cbor::to_vec(&Object::Null).expect("cbor failed"),
            options,
        )?;

        verbose!(options, "Inserted null object.");

        let empty_stage = store.insert(
            &serde_cbor::to_vec(&Object::Tree(vec![])).expect("cbor failed"),
            options,
        )?;

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
        self.store.check(
            HashSet::new(),
            &[self.info.head(), self.info.stage()],
            options,
        )?;

        Ok(())
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
}
