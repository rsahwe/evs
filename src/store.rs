use std::{
    fmt::{self, Display, Formatter},
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::PathBuf,
    sync::Mutex,
    thread,
};

use ahash::{AHashSet, HashSet};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use rayon::{
    iter::{IntoParallelRefIterator as _, ParallelBridge as _, ParallelIterator as _},
    join,
};
use sha2::{Digest as _, Sha256};
use tracing::{Span, debug, instrument, trace, warn};

use crate::{
    error::{CorruptState, EvsError},
    objects::Object,
};

// This is basically as good as 9 (best), but significantly faster. Later it will be configurable.
const COMPRESSION_LEVEL: u32 = 4;

pub type Hash = [u8; 32];
pub type PartialHash<'a> = &'a [u8];

const FORMATTED_HASH_SIZE: usize = size_of::<Hash>() * 2;

/// Needs to double the length of a hash (it does).
#[derive(Debug)]
pub struct HashDisplay<'a>(pub PartialHash<'a>);

impl Display for HashDisplay<'_> {
    #[inline]
    fn fmt(
        &self,
        f: &mut Formatter<'_>,
    ) -> fmt::Result {
        for el in self.0 {
            write!(f, "{:02x}", el)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    #[inline]
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[inline]
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Assumes a valid store and might cause unintended behaviour
    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn insert(
        &self,
        parent: &Span,
        mut obj: Object,
    ) -> Result<Hash, EvsError> {
        debug!("Store::insert(self, ...)");

        if let Object::Tree(entries) = &mut obj {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
        }

        let data = rmp_serde::to_vec(&obj)?;

        trace!("Serialized object to size {}.", data.len());

        let hash: Hash = Sha256::digest(&data).into();

        let hash_display = format!("{}", HashDisplay(&hash));

        trace!("Data hashed to \"{}\".", hash_display);

        let mut encoder = GzEncoder::new(Vec::new(), Compression::new(COMPRESSION_LEVEL));

        if encoder.write_all(&data).is_err() {
            unreachable!("gzip encoder failed: io error on vec");
        }

        let Ok(compressed) = encoder.finish() else {
            unreachable!("gzip encoder failed: io error on vec");
        };

        trace!(
            "Compressed data from {} to {} bytes.",
            data.len(),
            compressed.len()
        );

        let target = self.path.join(&hash_display);

        if target.exists() {
            trace!("Object path exists, assuming it is valid.");

            Ok(hash)
        } else {
            trace!("Object path does not exist, inserting...");

            let tmp = self
                .path
                .join(format!("{}-{:?}", hash_display, thread::current().id()));

            trace!("Using temporary path {:?}.", tmp);

            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| (e, target.clone()))?;

            file.write_all(&compressed)
                .map_err(|e| (e, target.clone()))?;

            drop(file);

            fs::rename(tmp, &target).map_err(|e| (e, target.clone()))?;

            trace!("Wrote object to store.");

            Ok(hash)
        }
    }

    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn lookup(
        &self,
        parent: &Span,
        id: &str,
    ) -> Result<(Hash, Object), EvsError> {
        if size_of_val(id) > FORMATTED_HASH_SIZE {
            debug!("Store::lookup(self, <overlength hash>)");

            return Err(EvsError::ObjectNotInStore(id.to_owned()));
        }

        debug!(
            "Store::lookup(self, \"{}{}\")",
            id,
            if size_of_val(id) < FORMATTED_HASH_SIZE {
                "..."
            } else {
                ""
            }
        );

        let mut target = None;

        if size_of_val(id) == FORMATTED_HASH_SIZE {
            let path = self.path.join(id);

            target = fs::exists(&path).is_ok_and(|e| e).then_some(path);
        } else {
            for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
                let obj = obj.map_err(|e| (e, self.path.clone()))?;

                let name = obj.path();

                if let Some(hash) = name.file_name()
                    && hash.as_encoded_bytes().starts_with(id.as_bytes())
                {
                    if let Some(target) = target {
                        return Err(EvsError::AmbiguousObject(
                            id.to_owned(),
                            target.file_name().unwrap().to_os_string(),
                        ));
                    }

                    target = Some(name);
                }
            }
        }

        if target.is_none() {
            return Err(EvsError::ObjectNotInStore(id.to_owned()));
        }

        let target = target.unwrap();

        let target_name = target.file_name().unwrap();

        if size_of_val(target_name) != FORMATTED_HASH_SIZE
            || !target_name
                .as_encoded_bytes()
                .iter()
                .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::InvalidObjectName(target_name.to_owned()),
            ));
        }

        trace!("Found object {:?}.", target);

        let content = fs::read(&target).map_err(|e| (e, target.clone()))?;

        trace!("Read object of compressed size {}.", content.len());

        let mut decoder = GzDecoder::new(&*content);

        let mut decompressed = vec![];

        decoder.read_to_end(&mut decompressed).map_err(|e| {
            EvsError::CorruptStateDetected(CorruptState::InvalidCompression(target.clone(), e))
        })?;

        trace!("Decompressed to size {}.", decompressed.len());

        let real_hash: Hash = Sha256::digest(&decompressed).into();

        if *target_name != *format!("{}", HashDisplay(&real_hash)) {
            return Err(EvsError::CorruptStateDetected(CorruptState::HashMismatch(
                target_name.to_owned(),
                real_hash.to_vec(),
            )));
        }

        trace!("Validated hash.");

        let deserialized =
            rmp_serde::from_slice::<Object>(&decompressed).map_err(|e| (e, real_hash))?;

        trace!("Deserialized successfully.");

        Ok((real_hash, deserialized))
    }

    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn check<T: AsRef<[Hash]>>(
        &self,
        parent: &Span,
        found: AHashSet<Hash>,
        required: T,
        all: bool,
    ) -> Result<(AHashSet<Hash>, AHashSet<Hash>), EvsError> {
        debug!("Store::check(self, <{} hash(es)>)", required.as_ref().len());

        self.check_(found, required.as_ref(), all)
    }

    fn check_(
        &self,
        found: AHashSet<Hash>,
        required: &[Hash],
        all: bool,
    ) -> Result<(AHashSet<Hash>, AHashSet<Hash>), EvsError> {
        let current = Span::current();

        let mut extra = AHashSet::new();
        let found = Mutex::new(found);
        let missing = Mutex::new(AHashSet::new());

        let found_cache = Mutex::new(AHashSet::new());

        trace!("Initially required to find {} object(s).", required.len());

        required
            .par_iter()
            .map(|item| self.check_one(&current, *item, &found, &found_cache, &missing))
            .collect::<Result<(), EvsError>>()?;

        let mut found = found.into_inner().unwrap();
        let missing = missing.into_inner().unwrap();

        let found_cache = found_cache.into_inner().unwrap();

        if all {
            extra = self
                .path
                .read_dir()
                .map_err(|e| (e, self.path.clone()))?
                .par_bridge()
                .filter_map(|obj| {
                    let _entered = current.enter();

                    let obj = match obj.map_err(|e| (e, self.path.clone())) {
                        Ok(obj) => obj,
                        Err(e) => return Some(Err(e.into())),
                    };

                    let name = obj.file_name();

                    let bytes = name.as_encoded_bytes();

                    if size_of_val(bytes) != FORMATTED_HASH_SIZE || name.to_str().is_none() {
                        return Some(Err(EvsError::CorruptStateDetected(
                            CorruptState::InvalidObjectName(name),
                        )));
                    }

                    let name = name.to_str().unwrap();

                    if found_cache.contains(name) {
                        return None;
                    }

                    let (hash, _) = match self.lookup(&current, name) {
                        Ok(res) => res,
                        Err(e) => return Some(Err(e)),
                    };

                    trace!("Validated extra \"{}\".", name);

                    Some(Ok(hash))
                })
                .collect::<Result<HashSet<Hash>, _>>()?
                .into();

            found.extend(extra.iter());
        }

        let normal_found = found.difference(&extra).count();
        #[allow(
            clippy::arithmetic_side_effects,
            reason = "Not going to happen + impossible."
        )]
        let normal_total = normal_found + missing.len();

        trace!(
            "Finished validating {}/{} (+{}) objects.",
            normal_found,
            normal_total,
            extra.len(),
        );

        if !missing.is_empty() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::MissingObjects(missing),
            ));
        }

        Ok((found, extra))
    }

    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    fn check_one(
        &self,
        parent: &Span,
        hash: Hash,
        found: &Mutex<AHashSet<Hash>>,
        found_cache: &Mutex<AHashSet<String>>,
        missing: &Mutex<AHashSet<Hash>>,
    ) -> Result<(), EvsError> {
        debug!("Store::check_one(self, \"{}\")", HashDisplay(&hash));

        let current = Span::current();

        if found.lock().unwrap().contains(&hash) {
            return Ok(());
        }

        let name = format!("{}", HashDisplay(&hash));

        let (hash, obj) = match self.lookup(&current, &name) {
            Ok(res) => res,
            Err(EvsError::ObjectNotInStore(_)) => {
                warn!("Missing \"{}\"", name);
                missing.lock().unwrap().insert(hash);
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        found_cache.lock().unwrap().insert(name);

        trace!("Validated \"{}\".", HashDisplay(&hash));

        match obj {
            Object::Null => trace!("Found the NULL object! :)"),
            Object::Blob(data) => trace!("Found blob of size {}.", data.len()),
            Object::Tree(items) => {
                trace!("Found tree with {} child(ren).", items.len());

                items
                    .par_iter()
                    .map(|item| {
                        let _entered = current.enter();

                        trace!(
                            "Requiring \"{}\" for \"{}\".",
                            HashDisplay(&item.content),
                            HashDisplay(&hash)
                        );

                        self.check_one(&current, item.content, found, found_cache, missing)
                    })
                    .collect::<Result<(), _>>()?;
            }
            Object::Commit(commit) => {
                trace!(
                    "Found commit with state \"{}\" and parent \"{}\".",
                    HashDisplay(&commit.tree),
                    HashDisplay(&commit.parent)
                );

                trace!(
                    "Requiring \"{}\" for \"{}\".",
                    HashDisplay(&commit.tree),
                    HashDisplay(&hash)
                );

                trace!(
                    "Requiring \"{}\" for \"{}\".",
                    HashDisplay(&commit.parent),
                    HashDisplay(&hash)
                );

                let (parent, tree) = join(
                    || self.check_one(&current, commit.parent, found, found_cache, missing),
                    || self.check_one(&current, commit.tree, found, found_cache, missing),
                );

                parent?;
                tree?;
            }
        }

        found.lock().unwrap().insert(hash);

        Ok(())
    }

    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn remove(
        &self,
        parent: &Span,
        path: Hash,
    ) -> Result<(), EvsError> {
        debug!("Store::remove(self, \"{}\")", HashDisplay(&path));

        let path = self.path.join(format!("{}", HashDisplay(&path)));

        trace!("Deleting {:?}", &path);

        fs::remove_file(&path).map_err(|e| (e, path))?;

        Ok(())
    }

    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn resolve_rest(
        &self,
        parent: &Span,
        r#ref: String,
    ) -> Result<String, EvsError> {
        debug!("Store::resolve_rest(self, \"{}\")", r#ref);

        let mut target = None;

        if size_of_val(r#ref.as_str()) == FORMATTED_HASH_SIZE {
            let path = self.path.join(&r#ref);

            trace!("Fast lookup of {:?}...", path);

            target = fs::exists(&path).is_ok().then_some(path);
        } else {
            trace!("Slow lookup...");

            for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
                let obj = obj.map_err(|e| (e, self.path.clone()))?;

                let name = obj.path();

                if let Some(hash) = name.file_name()
                    && hash.as_encoded_bytes().starts_with(r#ref.as_bytes())
                {
                    trace!("Found {:?}.", hash);

                    if let Some(target) = target {
                        return Err(EvsError::AmbiguousObject(
                            r#ref,
                            target.file_name().unwrap().to_os_string(),
                        ));
                    }

                    target = Some(name);
                }
            }
        }

        if target.is_none() {
            return Err(EvsError::ObjectNotInStore(r#ref));
        }

        let target = target.unwrap();

        let target_name = target.file_name().unwrap();

        if size_of_val(target_name) != FORMATTED_HASH_SIZE
            || !target_name
                .as_encoded_bytes()
                .iter()
                .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::InvalidObjectName(target_name.to_owned()),
            ));
        }

        trace!("Validated name successfully.");

        let resolved = target_name.to_str().unwrap().to_owned();

        Ok(resolved)
    }

    #[inline]
    #[instrument(parent = parent, level = "debug", err(level = "debug"), skip_all)]
    pub fn status(
        &self,
        parent: &Span,
    ) -> Result<(usize, usize), EvsError> {
        debug!("Store::status(self)");

        self.path
            .read_dir()
            .map_err(|e| (e, self.path.clone()))?
            .try_fold((0, 0usize), |(count, size), entry| match entry {
                #[allow(clippy::arithmetic_side_effects, reason = "Never going to happen.")]
                Ok(entry) => Ok((
                    count + 1,
                    size.saturating_add(
                        usize::try_from(entry.metadata().map_err(|e| (e, entry.path()))?.len())
                            .unwrap(),
                    ),
                )),
                Err(err) => Err((err, self.path.clone()).into()),
            })
    }
}
