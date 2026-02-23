use std::path::PathBuf;

use crate::{
    cli::Cli,
    error::EvsError,
    store::{Hash, Store},
};

#[derive(Debug, PartialEq, Eq)]
pub enum DiffSide {
    Tree(Hash),
    Local(PathBuf),
}

impl DiffSide {
    pub fn diff_with(
        from: Self,
        to: Self,
        store: &Store,
        files: impl AsRef<[PathBuf]>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        todo!(
            "from: {:?}, to: {:?}, with store: {:?}, on files: {:?}, with options: {:?}",
            from,
            to,
            store,
            files.as_ref(),
            options
        );
    }
}
