use std::{fmt::Display, ops::Deref, time::SystemTime};

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::store::{Hash, HashDisplay};

#[derive(Serialize, Deserialize, Debug)]
pub struct TreeEntry {
    pub name: Vec<u8>,
    // Maybe mode?
    pub content: Hash,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Commit {
    pub parent: Hash,
    pub name: String,
    pub email: String,
    pub tree: Hash,
    pub msg: String,
    pub date: SystemTime,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Object {
    Null,
    Blob(Vec<u8>),
    Tree(Vec<TreeEntry>),
    Commit(Commit),
}

impl Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Object::Null => write!(f, "Null object :)"),
            Object::Blob(items) => write!(f, "Blob:\n{}", items.deref().escape_ascii()),
            Object::Tree(items) => {
                if items.len() == 0 {
                    write!(f, "Empty tree :)")
                } else {
                    write!(f, "Tree:")?;

                    for item in items {
                        write!(
                            f,
                            "\n- \"{}\" {}",
                            HashDisplay(&item.content),
                            item.name.deref().escape_ascii()
                        )?;
                    }

                    Ok(())
                }
            }
            Object::Commit(commit) => write!(
                f,
                "Commit by {} <{}> at {}\n- \"{}\" state\n- \"{}\" parent\n{}",
                commit.name,
                commit.email,
                OffsetDateTime::from(commit.date)
                    .format(&Rfc3339)
                    .expect("I think this can't fail"),
                HashDisplay(&commit.tree),
                HashDisplay(&commit.parent),
                commit.msg
            ),
        }
    }
}
