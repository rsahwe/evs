use std::{
    error::Error,
    ffi::OsString,
    fmt::{self, Debug, Display, Formatter},
    fs::TryLockError,
    io,
    num::ParseIntError,
    ops::Deref,
    path::PathBuf,
    str::Utf8Error,
};

use ahash::AHashSet;
use glob::PatternError;
use rmp_serde::{decode, encode};

use crate::store::{Hash, HashDisplay, PartialHash};

#[derive(Debug)]
pub enum EvsError {
    IOError(io::Error, PathBuf),
    MissingRepository(PathBuf),
    CorruptStateDetected(CorruptState),
    RepositoryNotFound,
    RepositoryLocked(TryLockError, PathBuf),
    ObjectNotInStore(String),
    AmbiguousObject(String, OsString),
    RepositoryInfoCorrupt(decode::Error),
    PathOutsideOfRepo(PathBuf),
    PathNotInStage(PathBuf),
    IntegerParseError(ParseIntError),
    NotACommit(Hash),
    NotATree(Hash),
    NoPreviousCommit,
    PatternError(PatternError),
    PathError(Utf8Error, Vec<u8>),
    UncommittedChanges,
    EncoderFailed(encode::Error),
}

impl Display for EvsError {
    #[inline]
    fn fmt(
        &self,
        f: &mut Formatter<'_>,
    ) -> fmt::Result {
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
            EvsError::AmbiguousObject(hash, target) => {
                write!(
                    f,
                    "Name \"{}\" matches more than one object (e.g. {:?})",
                    hash, target
                )
            }
            EvsError::RepositoryInfoCorrupt(err) => write!(f, "Repository info corrupt: {}", err),
            EvsError::PathOutsideOfRepo(err) => {
                write!(f, "Path {:?} is outside of the repository.", err)
            }
            EvsError::PathNotInStage(err) => write!(f, "Path {:?} is not in the stage.", err),
            EvsError::IntegerParseError(err) => write!(f, "Could not parse integer: {}", err),
            EvsError::NotACommit(hash) => {
                write!(f, "Object \"{}\" is not a commit", HashDisplay(hash))
            }
            EvsError::NotATree(hash) => {
                write!(f, "Object \"{}\" is not a tree", HashDisplay(hash))
            }
            EvsError::NoPreviousCommit => write!(f, "NULL object does not have a previous commit"),
            EvsError::PatternError(err) => write!(f, "{}", err),
            EvsError::PathError(e, bts) => {
                write!(f, "Path \"{}\" is not valid: {}", bts.escape_ascii(), e)
            }
            EvsError::UncommittedChanges => write!(f, "There are uncommitted changes"),
            EvsError::EncoderFailed(err) => unreachable!("The encoder failed: {}", err),
        }
    }
}

impl Error for EvsError {}

impl From<(io::Error, PathBuf)> for EvsError {
    #[inline]
    fn from(value: (io::Error, PathBuf)) -> Self {
        EvsError::IOError(value.0, value.1)
    }
}

impl From<(TryLockError, PathBuf)> for EvsError {
    #[inline]
    fn from(value: (TryLockError, PathBuf)) -> Self {
        EvsError::RepositoryLocked(value.0, value.1)
    }
}

impl From<(decode::Error, Hash)> for EvsError {
    #[inline]
    fn from(value: (decode::Error, Hash)) -> Self {
        EvsError::CorruptStateDetected(CorruptState::InvalidObjectContent(value.1, value.0))
    }
}

impl From<PatternError> for EvsError {
    #[inline]
    fn from(value: PatternError) -> Self {
        EvsError::PatternError(value)
    }
}

impl From<encode::Error> for EvsError {
    #[inline]
    fn from(value: encode::Error) -> Self {
        EvsError::EncoderFailed(value)
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
    MissingObjects(AHashSet<Hash>),
    InvalidObjectContent(Hash, decode::Error),
    NonContentInTree(Hash, Hash, &'static str),
}

impl Display for CorruptState {
    #[inline]
    fn fmt(
        &self,
        f: &mut Formatter<'_>,
    ) -> fmt::Result {
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
            CorruptState::MissingObjects(set) => {
                let mut iter = set.iter();

                write!(
                    f,
                    "Object \"{}\" (+{} more) is missing",
                    HashDisplay(iter.next().unwrap()),
                    iter.count(),
                )
            }
            CorruptState::InvalidObjectContent(hash, err) => {
                write!(f, "Object \"{}\" is not valid: {}", HashDisplay(hash), err)
            }
            CorruptState::NonContentInTree(tree, content, r#type) => {
                write!(
                    f,
                    "Object \"{}\" in tree \"{}\" is {} insted of a tree entry type",
                    HashDisplay(tree),
                    HashDisplay(content),
                    r#type
                )
            }
        }
    }
}
