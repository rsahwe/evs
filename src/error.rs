use std::{
    error::Error,
    fmt::{Debug, Display},
    fs::TryLockError,
    io,
    ops::Deref,
    path::PathBuf,
};

use crate::store::{HashDisplay, PartialHash};

#[derive(Debug)]
pub enum EvsError {
    IOError(io::Error, PathBuf),
    MissingRepository(PathBuf),
    CorruptStateDetected(CorruptState),
    RepositoryNotFound,
    RepositoryLocked(TryLockError, PathBuf),
    ObjectNotInStore(<<PartialHash<'static> as Deref>::Target as ToOwned>::Owned),
}

impl Display for EvsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvsError::IOError(err, pb) => write!(f, "IO Error on {:?}: {}", pb, err),
            EvsError::MissingRepository(pb) => write!(f, "No repository found at`{:?}", pb),
            EvsError::CorruptStateDetected(cs) => write!(f, "Corrupt state: {}", cs),
            EvsError::RepositoryNotFound => write!(f, "No repository was found"),
            EvsError::RepositoryLocked(err, pb) => {
                write!(f, "The repository at {:?} could not be locked: {}", pb, err)
            }
            EvsError::ObjectNotInStore(hash) => write!(
                f,
                "Could not find object \"{}\" in store",
                HashDisplay(hash)
            ),
        }
    }
}

impl Error for EvsError {}

impl From<(io::Error, PathBuf)> for EvsError {
    fn from(value: (io::Error, PathBuf)) -> Self {
        EvsError::IOError(value.0, value.1)
    }
}

impl From<(TryLockError, PathBuf)> for EvsError {
    fn from(value: (TryLockError, PathBuf)) -> Self {
        EvsError::RepositoryLocked(value.0, value.1)
    }
}

#[derive(Debug)]
pub enum CorruptState {
    MissingPath(PathBuf),
    DirectoryIsFile(PathBuf),
    FileIsDirectory(PathBuf),
}

impl Display for CorruptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CorruptState::MissingPath(pb) => write!(f, "Path {:?} is missing", pb),
            CorruptState::DirectoryIsFile(pb) => write!(f, "Path {:?} should be a directory", pb),
            CorruptState::FileIsDirectory(pb) => write!(f, "Path {:?} should be a file", pb),
        }
    }
}
