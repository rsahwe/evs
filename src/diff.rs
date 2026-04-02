use std::{
    borrow::Cow,
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::{BufRead as _, Write as _, stdout},
    path::{Path, PathBuf},
    str::FromStr as _,
};

use ahash::{AHashMap, AHashSet};
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
    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn diff_with<F: AsRef<[PathBuf]>, I: AsRef<[Pattern]>>(
        from: Self,
        to: Self,
        store: &Store,
        files: F,
        ignores: I,
        options: &Cli,
    ) -> Result<(), EvsError> {
        debug!(
            "Diffside::diff_with({:?}, {:?}, store, {:?}, {} ignores)",
            from,
            to,
            files.as_ref(),
            ignores.as_ref().len()
        );

        Self::diff_with_(from, to, store, files.as_ref(), ignores.as_ref(), options)
    }

    fn diff_with_(
        from: Self,
        to: Self,
        store: &Store,
        files: &[PathBuf],
        ignores: &[Pattern],
        options: &Cli,
    ) -> Result<(), EvsError> {
        if from == to {
            return Ok(());
        }

        let lhs = from.read("", store, files, ignores, &AHashSet::new())?;

        trace!("Read 'from' diff source: {:?}.", lhs.0);

        let rhs = to.read("", store, files, ignores, &lhs.0)?;

        trace!("Read 'to' diff source: {:?}.", rhs.0);

        let removals = lhs.0.difference(&rhs.0);
        let insertions = rhs.0.difference(&lhs.0);
        let modifications = lhs.0.intersection(&rhs.0);

        #[allow(clippy::indexing_slicing, reason = "The keys are in the map as well.")]
        DiffFormat::print(
            removals.map(|e| (e.clone(), &lhs.1[e])),
            insertions.map(|e| (e.clone(), &rhs.1[e])),
            modifications.filter_map(|e| {
                let lhs = &lhs.1[e];
                let rhs = &rhs.1[e];

                (rhs != lhs).then(|| (e.clone(), lhs, rhs))
            }),
            options,
        );

        Ok(())
    }

    #[inline]
    #[allow(
        clippy::type_complexity,
        reason = "It's not even that complex, I might do something later."
    )]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn read<O: AsRef<Path>, F: AsRef<[PathBuf]>, I: AsRef<[Pattern]>>(
        self,
        origin: O,
        store: &Store,
        filter: F,
        ignores: I,
        overrides: &AHashSet<PathBuf>,
    ) -> Result<(AHashSet<PathBuf>, AHashMap<PathBuf, Vec<u8>>), EvsError> {
        debug!(
            "Diffside::read(self, {:?}, store, {:?}, {} ignores, {:?})",
            origin.as_ref(),
            filter.as_ref(),
            ignores.as_ref().len(),
            overrides
        );

        self.read_(
            origin.as_ref(),
            store,
            filter.as_ref(),
            ignores.as_ref(),
            overrides,
        )
    }

    #[allow(
        clippy::type_complexity,
        reason = "It's not even that complex, I might do something later."
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "It's barely over the limit and it's totally fine."
    )]
    fn read_(
        self,
        origin: &Path,
        store: &Store,
        filter: &[PathBuf],
        ignores: &[Pattern],
        overrides: &AHashSet<PathBuf>,
    ) -> Result<(AHashSet<PathBuf>, AHashMap<PathBuf, Vec<u8>>), EvsError> {
        let mut sum_set = AHashSet::new();
        let mut sum_map = AHashMap::new();

        let result = match self {
            DiffSide::Tree(tree) => {
                trace!("Reading from tree source \"{}\"...", HashDisplay(&tree));

                let (hash, tree) = store.lookup(&format!("{}", HashDisplay(&tree)))?;

                trace!("Found tree in store.");

                let Object::Tree(tree) = tree else {
                    return Err(EvsError::NotATree(hash));
                };

                for entry in tree {
                    let path = origin.join(match OsString::from_str(&entry.name) {
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

                    let path = origin.join(&entry_name);

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
    //TODO: FIX THIS GARBAGE
    #[inline]
    #[instrument(level = "debug", skip_all)]
    pub fn print<
        RB: AsRef<[u8]>,
        RP: AsRef<Path>,
        R: IntoIterator<Item = (RP, RB)>,
        IB: AsRef<[u8]>,
        IP: AsRef<Path>,
        I: IntoIterator<Item = (IP, IB)>,
        MBL: AsRef<[u8]>,
        MBR: AsRef<[u8]>,
        MP: AsRef<Path>,
        M: IntoIterator<Item = (MP, MBL, MBR)>,
    >(
        removals: R,
        insertions: I,
        modifications: M,
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

            DiffFormat::write_diff(&diff, print_color);
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

            DiffFormat::write_diff(&diff, print_color);
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

            DiffFormat::write_diff(&diff, print_color);
        }
    }

    #[inline]
    #[must_use]
    pub fn binary_to_text(binary: &[u8]) -> String {
        let mut result = String::new();

        let _ = writeln!(
            result,
            "┌────────┬─────────────────────────┬─────────────────────────┐"
        );

        for (addr, line) in binary.chunks(16).enumerate() {
            let _ = write!(
                result,
                "│{:07x}0│",
                addr & usize::try_from(u32::MAX).unwrap()
            );

            for left in line.iter().take(8) {
                let _ = write!(result, " {:02x}", left);
            }

            if let Some(left) = 8usize.checked_sub(line.len()) {
                for _ in 0..left {
                    let _ = write!(result, "   ");
                }
            }

            let _ = write!(result, " ┊");

            for right in line.iter().skip(8).take(8) {
                let _ = write!(result, " {:02x}", right);
            }

            if let Some(diff) = 16usize.checked_sub(line.len()) {
                for _ in 0..diff.min(8) {
                    let _ = write!(result, "   ");
                }
            }

            let _ = writeln!(result, " │");
        }

        let _ = writeln!(
            result,
            "└────────┴─────────────────────────┴─────────────────────────┘"
        );

        result
    }

    #[inline]
    pub fn write_diff<'a: 'b + 'c, 'b, 'c, 'd, S: DiffableStr + ?Sized>(
        diff: &UnifiedDiff<'a, 'b, 'c, 'd, S>,
        print_color: bool,
    ) {
        let mut stdout = stdout();

        if !print_color {
            let _ = diff.to_writer(&stdout);
        } else {
            let mut result = Vec::new();

            let _ = diff.to_writer(&mut result);

            for line in result.as_slice().lines().map_while(Result::ok) {
                if line.starts_with("+++") || line.starts_with("---") {
                } else if line.starts_with('@') {
                    let _ = write!(stdout, "{}", INFO_COLOR);
                } else if line.starts_with('+') {
                    let _ = write!(stdout, "{}", ADD_COLOR);
                } else if line.starts_with('-') {
                    let _ = write!(stdout, "{}", SUB_COLOR);
                } else {
                    // Doesn't matter
                }

                let _ = writeln!(stdout, "{}{}", line, NONE_COLOR);
            }
        }
    }
}
