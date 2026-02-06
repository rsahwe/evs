use serde::{Deserialize, Serialize};

use crate::store::Hash;

#[derive(Serialize, Deserialize, Debug)]
pub struct TreeEntry {
    pub name: Vec<u8>,
    // Maybe mode?
    pub content: Hash,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Commit {
    pub name: String,
    pub email: String,
    pub tree: Hash,
    pub msg: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Object {
    Null,
    Blob(Vec<u8>),
    Tree(Vec<TreeEntry>),
    Commit(Commit),
}
