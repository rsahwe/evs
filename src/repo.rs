use std::{
    fs::{DirBuilder, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    cli::{Cli, VERBOSITY_ALL, VERBOSITY_TRACE},
    error::{CorruptState, EvsError},
    store::{Hash, NULL_HASH},
    util::DropAction,
};

#[derive(Debug)]
pub struct Repository {
    pub workspace: PathBuf,
    pub repository: PathBuf,
    pub lockfile: File,
    pub store: PathBuf,
    pub info: RepositoryInfo,
}

impl Repository {
    pub fn open(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::open({:?})", path.as_ref());
        }

        let _drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::open(...) done");
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
            store,
            info: repo_info,
        };

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Created repository.");
        }

        Ok(repository)
    }

    pub fn create(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::create({:?})", path.as_ref());
        }

        let _drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::create(...) done");
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

        let repo_info = RepositoryInfo { head: NULL_HASH };

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

        Ok(repository)
    }

    pub fn find(path: impl AsRef<Path>, options: &Cli) -> Result<Repository, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Repository::find({:?})", path.as_ref());
        }

        let _drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Repository::find(...) done");
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

                    return Ok(repo);
                }
                Err(e) => match e {
                    EvsError::IOError(_, _)
                    | EvsError::CorruptStateDetected(_)
                    | EvsError::RepositoryLocked(_, _) => return Err(e),
                    EvsError::MissingRepository(_) | EvsError::RepositoryNotFound => (),
                },
            }

            if !path.pop() {
                return Err(EvsError::RepositoryNotFound);
            }
        }
    }
}

/// All of the info about the repository
#[derive(Serialize, Deserialize, Debug)]
pub struct RepositoryInfo {
    head: Hash,
}
