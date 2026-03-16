use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    env::var_os,
    ffi::OsString,
    fs,
    io::{IsTerminal, stdout},
    mem::ManuallyDrop,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use similar::{DiffableStr, TextDiff};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    objects::Object,
    store::{Hash, HashDisplay, Store},
    trace,
    util::DropAction,
    verbose,
};

#[derive(Debug, PartialEq, Eq)]
pub enum DiffSide {
    Tree(Hash),
    Local(PathBuf, bool),
}

impl DiffSide {
    pub fn diff_with(
        from: Self,
        to: Self,
        store: &Store,
        files: impl AsRef<[PathBuf]>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        trace!(
            options,
            "Diffside::diff_with({:?}, {:?}, store, {:?})",
            from,
            to,
            files.as_ref()
        );

        if from == to {
            return Ok(());
        }

        let files = files.as_ref();

        let drop = DropAction(|| {
            trace!(options, "Diffside::diff_with(...) error");
        });

        let lhs = from.read("", store, files, options)?;

        verbose!(options, "Read 'from' diff source.");

        let rhs = to.read("", store, files, options)?;

        verbose!(options, "Read 'to' diff source.");

        let removals = lhs.0.difference(&rhs.0);
        let insertions = rhs.0.difference(&lhs.0);
        let modifications = lhs.0.intersection(&rhs.0);

        DiffFormat::print(
            removals.map(|e| (e.clone(), lhs.1.get(e).unwrap())),
            insertions.map(|e| (e.clone(), rhs.1.get(e).unwrap())),
            modifications.map(|e| (e.clone(), lhs.1.get(e).unwrap(), rhs.1.get(e).unwrap())),
            options,
        );

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Diffside::diff_with(...) ok");

        Ok(())
    }

    pub fn read(
        self,
        origin: impl AsRef<Path>,
        store: &Store,
        filter: impl AsRef<[PathBuf]>,
        options: &Cli,
    ) -> Result<(HashSet<PathBuf>, HashMap<PathBuf, Vec<u8>>), EvsError> {
        trace!(
            options,
            "Diffside::read(self, {:?}, store, {:?})",
            origin.as_ref(),
            filter.as_ref()
        );

        let drop = DropAction(|| {
            trace!(options, "Diffside::read(self, ...) error");
        });

        let mut sum_set = HashSet::new();
        let mut sum_map = HashMap::new();

        let filter = filter.as_ref();

        let result = match self {
            DiffSide::Tree(tree) => {
                verbose!(
                    options,
                    "Reading from tree source \"{}\"...",
                    HashDisplay(&tree)
                );

                let (hash, tree) = store.lookup(&format!("{}", HashDisplay(&tree)), options)?;

                verbose!(options, "Found tree in store.");

                let tree = match tree {
                    Object::Tree(tree) => tree,
                    _ => return Err(EvsError::NotATree(hash)),
                };

                for entry in tree {
                    let path = origin.as_ref().join(OsString::from_vec(entry.name));

                    if !filter.iter().any(|f| path.starts_with(f)) {
                        verbose!(options, "Filtered path {:?}.", path);

                        continue;
                    }

                    verbose!(options, "Reading path {:?}.", path);

                    let (entry_hash, content) =
                        store.lookup(&format!("{}", HashDisplay(&entry.content)), options)?;

                    verbose!(
                        options,
                        "Found path content \"{}\".",
                        HashDisplay(&entry_hash)
                    );

                    match content {
                        Object::Blob(content) => {
                            verbose!(options, "Inserting blob...");

                            sum_set.insert(path.clone());
                            sum_map.insert(path, content);
                        }
                        Object::Tree(_) => {
                            verbose!(options, "Reading tree...");

                            let (set, map) =
                                DiffSide::Tree(entry_hash).read(path, store, filter, options)?;

                            for el in set {
                                sum_set.insert(el);
                            }

                            for (k, v) in map {
                                sum_map.insert(k, v);
                            }

                            verbose!(options, "Finished inserting tree.");
                        }
                        Object::Null => {
                            return Err(EvsError::CorruptStateDetected(
                                CorruptState::NonContentInTree(hash, entry_hash, "(null)"),
                            ));
                        }
                        Object::Commit(_) => {
                            return Err(EvsError::CorruptStateDetected(
                                CorruptState::NonContentInTree(hash, entry_hash, "commit"),
                            ));
                        }
                    }
                }

                (sum_set, sum_map)
            }
            DiffSide::Local(path_buf, root) => {
                verbose!(
                    options,
                    "Reading from local source {:?} which is {} root...",
                    path_buf,
                    if root { "the" } else { "not the" }
                );

                let dir = path_buf.read_dir().map_err(|e| (e, path_buf.clone()))?;

                for entry in dir {
                    let entry = entry.map_err(|e| (e, path_buf.clone()))?;

                    let entry_name = entry.file_name();

                    let path = origin.as_ref().join(&entry_name);

                    if !filter.iter().any(|f| path.starts_with(f)) {
                        verbose!(options, "Filtered path {:?}.", path);

                        continue;
                    }

                    let entry = entry.path();

                    if root && entry_name == ".evs" {
                        verbose!(options, "Filtered repo path {:?}.", entry);

                        continue;
                    }

                    if entry.is_file() {
                        verbose!(options, "Inserting blob...");

                        sum_set.insert(path.clone());
                        sum_map.insert(path.clone(), fs::read(&path).map_err(|e| (e, path))?);
                    } else if entry.is_dir() {
                        verbose!(options, "Reading tree...");

                        let (set, map) =
                            DiffSide::Local(entry, false).read(path, store, filter, options)?;

                        for el in set {
                            sum_set.insert(el);
                        }

                        for (k, v) in map {
                            sum_map.insert(k, v);
                        }

                        verbose!(options, "Finished inserting tree.");
                    } else {
                        verbose!(
                            options,
                            "Skipping file {:?}, because it is neither a file nor a directory.",
                            entry
                        );
                    }
                }

                (sum_set, sum_map)
            }
        };

        verbose!(options, "Finished reading.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Diffside::read(self, ...) ok");

        Ok(result)
    }
}

pub struct DiffFormat;

const INFO_COLOR: &str = "\033[36m";
const ADD_COLOR: &str = "\033[32m";
const SUB_COLOR: &str = "\033[31m";

impl DiffFormat {
    pub fn print(
        removals: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
        insertions: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
        modifications: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>, impl AsRef<[u8]>)>,
        options: &Cli,
    ) {
        trace!(options, "DiffFormat::print(...)");

        let print_color = !(options.no_color
            || var_os("NO_COLOR").is_some_and(|v| v != "")
            || !stdout().is_terminal());

        for removal in removals {
            let text = match removal.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => todo!(),
            };

            let diff = TextDiff::from_lines(text.as_ref(), "");

            let mut diff = diff.unified_diff();

            diff.header(
                AsRef::<Path>::as_ref("a")
                    .join(removal.0)
                    .display()
                    .to_string()
                    .as_str(),
                "/dev/null",
            );

            todo!()
        }

        for insertion in insertions {
            let text = match insertion.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => todo!(),
            };

            let diff = TextDiff::from_lines("", text.as_ref());

            let mut diff = diff.unified_diff();

            diff.header(
                "/dev/null",
                AsRef::<Path>::as_ref("b")
                    .join(insertion.0)
                    .display()
                    .to_string()
                    .as_str(),
            );

            todo!()
        }

        for modification in modifications {
            let a_text = match modification.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => todo!(),
            };

            let b_text = match modification.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => todo!(),
            };

            let diff = TextDiff::from_lines(a_text.as_ref(), b_text.as_ref());

            let mut diff = diff.unified_diff();

            diff.header(
                AsRef::<Path>::as_ref("a")
                    .join(&modification.0)
                    .display()
                    .to_string()
                    .as_str(),
                AsRef::<Path>::as_ref("b")
                    .join(&modification.0)
                    .display()
                    .to_string()
                    .as_str(),
            );

            todo!()
        }

        todo!("")
    }
}
