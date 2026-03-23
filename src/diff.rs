use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt::Write as FmtWrite,
    fs,
    io::{BufRead, Write as IoWrite, stdout},
    path::{Path, PathBuf},
    str::FromStr,
};

use glob::Pattern;
use similar::{DiffableStr, TextDiff, udiff::UnifiedDiff};
use tracing::{debug, instrument, trace};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    objects::Object,
    store::{Hash, HashDisplay, Store},
    util::{ADD_COLOR, INFO_COLOR, NONE_COLOR, SUB_COLOR, get_color},
};

#[derive(Debug, PartialEq, Eq)]
pub enum DiffSide {
    Tree(Hash),
    Local(PathBuf),
}

impl DiffSide {
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn diff_with(
        from: Self,
        to: Self,
        store: &Store,
        files: impl AsRef<[PathBuf]>,
        ignores: impl AsRef<[Pattern]>,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!(
            "Diffside::diff_with({:?}, {:?}, store, {:?}, {} ignores)",
            from,
            to,
            files.as_ref(),
            ignores.as_ref().len()
        );

        if from == to {
            return Ok(());
        }

        let files = files.as_ref();
        let ignores = ignores.as_ref();

        let lhs = from.read("", store, files, ignores, &HashSet::new())?;

        trace!("Read 'from' diff source: {:?}.", lhs.0);

        let rhs = to.read("", store, files, ignores, &lhs.0)?;

        trace!("Read 'to' diff source: {:?}.", rhs.0);

        let removals = lhs.0.difference(&rhs.0);
        let insertions = rhs.0.difference(&lhs.0);
        let modifications = lhs.0.intersection(&rhs.0);

        DiffFormat::print(
            removals.map(|e| (e.clone(), lhs.1.get(e).unwrap())),
            insertions.map(|e| (e.clone(), rhs.1.get(e).unwrap())),
            modifications.filter_map(|e| {
                let lhs = lhs.1.get(e).unwrap();
                let rhs = rhs.1.get(e).unwrap();

                (rhs != lhs).then(|| (e.clone(), lhs, rhs))
            }),
            options,
        );

        Ok(())
    }

    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn read(
        self,
        origin: impl AsRef<Path>,
        store: &Store,
        filter: impl AsRef<[PathBuf]>,
        ignores: impl AsRef<[Pattern]>,
        overrides: &HashSet<PathBuf>,
    ) -> Result<(HashSet<PathBuf>, HashMap<PathBuf, Vec<u8>>), EvsError> {
        debug!(
            "Diffside::read(self, {:?}, store, {:?}, {} ignores, {:?})",
            origin.as_ref(),
            filter.as_ref(),
            ignores.as_ref().len(),
            overrides
        );

        let mut sum_set = HashSet::new();
        let mut sum_map = HashMap::new();

        let filter = filter.as_ref();
        let ignores = ignores.as_ref();

        let result = match self {
            DiffSide::Tree(tree) => {
                trace!("Reading from tree source \"{}\"...", HashDisplay(&tree));

                let (hash, tree) = store.lookup(&format!("{}", HashDisplay(&tree)))?;

                trace!("Found tree in store.");

                let tree = match tree {
                    Object::Tree(tree) => tree,
                    _ => return Err(EvsError::NotATree(hash)),
                };

                for entry in tree {
                    let path = origin.as_ref().join(match OsString::from_str(&entry.name) {
                        Ok(str) => str,
                    });

                    if !filter
                        .iter()
                        .any(|f| path.starts_with(f) || f.starts_with(&path))
                    {
                        trace!("Filtered path {:?}.", path);

                        continue;
                    }

                    trace!("Reading path {:?}.", path);

                    let (entry_hash, content) =
                        store.lookup(&format!("{}", HashDisplay(&entry.content)))?;

                    trace!("Found path content \"{}\".", HashDisplay(&entry_hash));

                    match content {
                        Object::Blob(content) => {
                            trace!("Inserting blob {:?}...", path);

                            sum_set.insert(path.clone());
                            sum_map.insert(path, content);
                        }
                        Object::Tree(_) => {
                            trace!("Reading tree {:?}...", path);

                            let (set, map) = DiffSide::Tree(entry_hash)
                                .read(path, store, filter, ignores, overrides)?;

                            for el in set {
                                sum_set.insert(el);
                            }

                            for (k, v) in map {
                                sum_map.insert(k, v);
                            }

                            trace!("Finished inserting tree.");
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
            DiffSide::Local(path_buf) => {
                trace!("Reading from local source {:?}...", path_buf);

                let dir = path_buf.read_dir().map_err(|e| (e, path_buf.clone()))?;

                for entry in dir {
                    let entry = entry.map_err(|e| (e, path_buf.clone()))?;

                    let entry_name = entry.file_name();

                    let path = origin.as_ref().join(&entry_name);

                    if !filter
                        .iter()
                        .any(|f| path.starts_with(f) || f.starts_with(&path))
                        || (!overrides.iter().any(|o| o.starts_with(&path))
                            && ignores
                                .iter()
                                .any(|i| path.ancestors().any(|a| i.matches_path(a))))
                    {
                        trace!("Filtered path {:?}.", path);

                        continue;
                    }

                    let entry = entry.path();

                    if entry.is_file() {
                        trace!("Inserting blob {:?}...", path);

                        sum_set.insert(path.clone());
                        sum_map.insert(path.clone(), fs::read(&entry).map_err(|e| (e, path))?);
                    } else if entry.is_dir() {
                        trace!("Reading tree {:?}...", path);

                        let (set, map) =
                            DiffSide::Local(entry).read(path, store, filter, ignores, overrides)?;

                        for el in set {
                            sum_set.insert(el);
                        }

                        for (k, v) in map {
                            sum_map.insert(k, v);
                        }

                        trace!("Finished inserting tree.");
                    } else {
                        trace!(
                            "Skipping file {:?}, because it is neither a file nor a directory.",
                            entry
                        );
                    }
                }

                (sum_set, sum_map)
            }
        };

        trace!("Finished reading.");

        Ok(result)
    }
}

pub struct DiffFormat;

impl DiffFormat {
    #[instrument(level = "debug", skip_all)]
    pub fn print(
        removals: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
        insertions: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>)>,
        modifications: impl IntoIterator<Item = (impl AsRef<Path>, impl AsRef<[u8]>, impl AsRef<[u8]>)>,
        options: &Cli,
    ) {
        debug!("DiffFormat::print(...)");

        let print_color = get_color(options);

        for removal in removals {
            let text = match removal.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => Cow::Owned(DiffFormat::binary_to_text(removal.1.as_ref())),
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

            DiffFormat::write_diff(diff, print_color);
        }

        for insertion in insertions {
            let text = match insertion.1.as_ref().as_str() {
                Some(text) => Cow::Borrowed(text),
                None => Cow::Owned(DiffFormat::binary_to_text(insertion.1.as_ref())),
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

            DiffFormat::write_diff(diff, print_color);
        }

        for modification in modifications {
            let (a_text, b_text) = match (
                modification.1.as_ref().as_str(),
                modification.2.as_ref().as_str(),
            ) {
                (Some(a_text), Some(b_text)) => (Cow::Borrowed(a_text), Cow::Borrowed(b_text)),
                (_, _) => (
                    Cow::Owned(DiffFormat::binary_to_text(modification.1.as_ref())),
                    Cow::Owned(DiffFormat::binary_to_text(modification.2.as_ref())),
                ),
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

            DiffFormat::write_diff(diff, print_color);
        }
    }

    pub fn binary_to_text(binary: impl AsRef<[u8]>) -> String {
        let mut result = String::new();

        let _ = write!(
            result,
            "┌────────┬─────────────────────────┬─────────────────────────┐\n"
        );

        for (addr, line) in binary.as_ref().chunks(16).enumerate() {
            let _ = write!(result, "│{:07x}0│", addr & u32::MAX as usize);

            for left in line.iter().take(8) {
                let _ = write!(result, " {:02x}", left);
            }

            if line.len() < 8 {
                for _ in 0..(8 - line.len()) {
                    let _ = write!(result, "   ");
                }
            }

            let _ = write!(result, " ┊");

            for right in line.iter().skip(8).take(8) {
                let _ = write!(result, " {:02x}", right);
            }

            if line.len() < 16 {
                for _ in 0..((16 - line.len()).min(8)) {
                    let _ = write!(result, "   ");
                }
            }

            let _ = write!(result, " │\n");
        }

        let _ = write!(
            result,
            "└────────┴─────────────────────────┴─────────────────────────┘\n"
        );

        result
    }

    pub fn write_diff<'a: 'b + 'c, 'b, 'c, 'd>(
        diff: UnifiedDiff<'a, 'b, 'c, 'd, impl DiffableStr + ?Sized>,
        print_color: bool,
    ) {
        let mut stdout = stdout();

        if !print_color {
            let _ = diff.to_writer(&stdout);
        } else {
            let mut result = Vec::new();

            let _ = diff.to_writer(&mut result);

            for line in result.as_slice().lines().flatten() {
                if line.starts_with("+++") || line.starts_with("---") {
                    ()
                } else if line.starts_with('@') {
                    let _ = write!(stdout, "{}", INFO_COLOR);
                } else if line.starts_with('+') {
                    let _ = write!(stdout, "{}", ADD_COLOR);
                } else if line.starts_with('-') {
                    let _ = write!(stdout, "{}", SUB_COLOR);
                }

                let _ = write!(stdout, "{}{}\n", line, NONE_COLOR);
            }
        }
    }
}
