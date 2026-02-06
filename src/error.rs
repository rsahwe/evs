use std::{
    error::Error,
    ffi::OsString,
    fmt::{Debug, Display},
    fs::TryLockError,
    io,
    ops::Deref,
    path::PathBuf,
};

use crate::store::{Hash, HashDisplay, PartialHash};

#[derive(Debug)]
pub enum EvsError {
    IOError(io::Error, PathBuf),
    MissingRepository(PathBuf),
    CorruptStateDetected(CorruptState),
    RepositoryNotFound,
    RepositoryLocked(TryLockError, PathBuf),
    ObjectNotInStore(String),
    AmbiguousObject(String),
    RepositoryInfoCorrupt(serde_cbor::Error),
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
            EvsError::ObjectNotInStore(hash) => {
                write!(f, "Could not find object \"{}\" in store", hash)
            }
            EvsError::AmbiguousObject(hash) => {
                write!(f, "Name \"{}\" matches more than one object", hash)
            }
            EvsError::RepositoryInfoCorrupt(err) => write!(f, "Repository info corrupt: {}", err),
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

impl From<(serde_cbor::Error, Hash)> for EvsError {
    fn from(value: (serde_cbor::Error, Hash)) -> Self {
        EvsError::CorruptStateDetected(CorruptState::InvalidObjectContent(value.1, value.0))
    }
}

#[derive(Debug)]
pub enum CorruptState {
    MissingPath(PathBuf),
    DirectoryIsFile(PathBuf),
    FileIsDirectory(PathBuf),
    InvalidObjectName(OsString),
    HashMismatch(
        OsString,
        <<PartialHash<'static> as Deref>::Target as ToOwned>::Owned,
    ),
    InvalidCompression(PathBuf, io::Error),
    MissingObjects(Hash, usize),
    InvalidObjectContent(Hash, serde_cbor::Error),
}

impl Display for CorruptState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CorruptState::MissingPath(pb) => write!(f, "Path {:?} is missing", pb),
            CorruptState::DirectoryIsFile(pb) => write!(f, "Path {:?} should be a directory", pb),
            CorruptState::FileIsDirectory(pb) => write!(f, "Path {:?} should be a file", pb),
            CorruptState::InvalidObjectName(name) => {
                write!(f, "Found invalid object name {:?} in store", name)
            }
            CorruptState::HashMismatch(found, real) => write!(
                f,
                "Object {:?} seemingly contains object \"{}\"",
                found,
                HashDisplay(real)
            ),
            CorruptState::InvalidCompression(pb, err) => {
                write!(f, "Path {:?} is compressed incorrectly: {}", pb, err)
            }
            CorruptState::MissingObjects(first, rest) => {
                write!(
                    f,
                    "Object \"{}\" (+{} more) is missing",
                    HashDisplay(first),
                    rest
                )
            }
            CorruptState::InvalidObjectContent(hash, err) => {
                write!(f, "Object {} is not valid: {}", HashDisplay(hash), err)
            }
        }
    }
}
