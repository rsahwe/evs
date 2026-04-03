use std::{
    fmt::{self, Display, Formatter},
    ops::Deref as _,
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::store::{Hash, HashDisplay};

#[derive(Serialize, Deserialize, Debug)]
pub struct TreeEntry {
    pub name: String,
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
    #[inline]
    fn fmt(
        &self,
        f: &mut Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Object::Null => write!(f, "Null object :)"),
            Object::Blob(items) => write!(f, "Blob:\n{}", items.deref().escape_ascii()),
            Object::Tree(items) => {
                if items.is_empty() {
                    write!(f, "Empty tree :)")
                } else {
                    write!(f, "Tree:")?;

                    for item in items {
                        write!(f, "\n- \"{}\" {}", HashDisplay(&item.content), item.name)?;
                    }

                    Ok(())
                }
            }
            Object::Commit(commit) => write!(
                f,
                "  Commit by {} <{}> at {}\n  - \"{}\" state\n  - \"{}\" parent\n\n{}",
                commit.name,
                commit.email,
                OffsetDateTime::from(commit.date).format(&Rfc3339).unwrap(), // This can't fail I think
                HashDisplay(&commit.tree),
                HashDisplay(&commit.parent),
                commit.msg.lines().fold(String::new(), |mut acc, l| {
                    acc += "    ";
                    acc += l;
                    acc += "\n";
                    acc
                }),
            ),
        }
    }
}
