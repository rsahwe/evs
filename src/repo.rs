use std::{
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    mem::ManuallyDrop,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::{Cli, VERBOSITY_ALL, VERBOSITY_TRACE},
    error::{CorruptState, EvsError},
    store::{Hash, Store},
    util::DropAction,
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
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::open({:?})", path.as_ref());
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::open(...) error");
            }
        });

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Workspace exists and is a directory.");
        }

        let repo = path.as_ref().join(".evs");

        if !repo.exists() {
            return Err(EvsError::MissingRepository(repo));
        }

        if !repo.is_dir() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::DirectoryIsFile(repo),
            ));
        }

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Repository exists and is a directory.");
        }

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

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Store exists and is a directory.");
        }

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

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Successfully obtained lock.");
        }

        let mut repo_info = vec![];

        lockfile
            .read_to_end(&mut repo_info)
            .map_err(|e| (e, lockfile_path.clone()))?;

        let repo_info = serde_cbor::from_slice(&repo_info).expect("cbor failed");

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Read repository info successfully.");
        }

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store: Store::new(store),
            info: repo_info,
        };

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created repository.");
        }

        let _ = ManuallyDrop::new(drop);

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::open(...) done");
        }

        Ok(repository)
    }

    pub fn create(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::create({:?})", path.as_ref());
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::create(...) error");
            }
        });

        let _ = path
            .as_ref()
            .read_dir()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Workspace exists and is a directory.");
        }

        let repo = path.as_ref().join(".evs");

        DirBuilder::new()
            .create(&repo)
            .map_err(|e| (e, repo.clone()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created repository directory.");
        }

        let store = repo.join("store");

        DirBuilder::new()
            .create(&store)
            .map_err(|e| (e, store.clone()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created store directory.");
        }

        let store = Store::new(store);

        // The NULL object. TODO: Replace with more accurate NULL object later
        let root = store.insert(&[], options)?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Inserted null object.");
        }

        let lockfile_path = repo.join("lock");

        let mut lockfile = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lockfile_path)
            .map_err(|e| (e, lockfile_path.clone()))?;

        lockfile.try_lock().map_err(|e| (e, repo.clone()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created and locked lockfile.");
        }

        let repo_info = RepositoryInfo {
            head: root,
            modified: false,
        };

        lockfile
            .write_all(&serde_cbor::to_vec(&repo_info).expect("cbor failed"))
            .map_err(|e| (e, lockfile_path.clone()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Wrote repository info into the lockfile.");
        }

        let repository = Repository {
            workspace: path.as_ref().to_path_buf(),
            repository: repo,
            lockfile,
            store,
            info: repo_info,
        };

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created repository.");
        }

        let _ = ManuallyDrop::new(drop);

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::create(...) done");
        }

        Ok(repository)
    }

    pub fn find(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::find({:?})", path.as_ref());
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::find(...) error");
            }
        });

        let mut path = path
            .as_ref()
            .canonicalize()
            .map_err(|e| (e, path.as_ref().to_path_buf()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Canonicalized path.");
        }

        loop {
            if options.verbose >= VERBOSITY_ALL {
                eprintln!("### Trying path {:?}:", path);
            }

            match Self::open(&path, options) {
                Ok(repo) => {
                    if options.verbose >= VERBOSITY_ALL {
                        eprintln!("### Found repository at {:?}", path);
                    }

                    let _ = ManuallyDrop::new(drop);

                    if options.verbose >= VERBOSITY_TRACE {
                        eprintln!("## Repository::find(...) done");
                    }

                    return Ok(repo);
                }
                Err(e) => match e {
                    EvsError::IOError(_, _)
                    | EvsError::CorruptStateDetected(_)
                    | EvsError::RepositoryLocked(_, _)
                    | EvsError::ObjectNotInStore(_)
                    | EvsError::AmbiguousObject(_) => return Err(e),
                    EvsError::MissingRepository(_) | EvsError::RepositoryNotFound => (),
                },
            }

            if !path.pop() {
                return Err(EvsError::RepositoryNotFound);
            }
        }
    }

    pub fn check(&self, options: &Cli) -> Result<(), EvsError> {
        self.store.check(&[self.info.head()], options)
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
            eprintln!("Writing back Repository Info failed: {}", err);
        }
    }
}

/// All of the info about the repository
#[derive(Serialize, Deserialize, Debug)]
pub struct RepositoryInfo {
    head: Hash,
    #[serde(skip)]
    modified: bool,
}

impl RepositoryInfo {
    pub fn head(&self) -> Hash {
        self.head
    }
}
