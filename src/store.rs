use std::{
    fmt::{self, Display, Formatter},
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::PathBuf,
};

use ahash::{AHashMap, AHashSet};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use sha2::{Digest as _, Sha256};
use tracing::{debug, instrument, trace};

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
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn insert(&self, mut obj: Object) -> Result<Hash, EvsError> {
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
            trace!("Object path exists! TODO: Maybe figure out strategy for this.");

            Ok(hash)
        } else {
            trace!("Object path does not exist, inserting...");

            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|e| (e, target.clone()))?;

            file.write_all(&compressed)
                .map_err(|e| (e, target.clone()))?;

            trace!("Wrote object to store.");

            Ok(hash)
        }
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn lookup(&self, id: &str) -> Result<(Hash, Object), EvsError> {
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

            target = fs::exists(&path).is_ok().then_some(path);
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
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn check<T: AsRef<[Hash]>>(
        &self,
        found: AHashSet<Hash>,
        required: T,
        dependency_info: Option<&mut Option<AHashMap<Hash, usize>>>,
    ) -> Result<AHashSet<Hash>, EvsError> {
        debug!("Store::check(self, <{} hash(es)>)", required.as_ref().len());

        self.check_(found, required.as_ref(), dependency_info)
    }

    fn check_(
        &self,
        mut found: AHashSet<Hash>,
        required: impl AsRef<[Hash]>,
        dependency_info: Option<&mut Option<AHashMap<Hash, usize>>>,
    ) -> Result<AHashSet<Hash>, EvsError> {
        let (mut required, mut dependencies) = {
            let mut hm = AHashSet::new();
            let mut dep = AHashMap::new();

            for r in required.as_ref() {
                hm.insert(*r);
                dep.insert(*r, 1);
            }

            (hm, dep)
        };

        trace!("Initially required to find {} object(s).", required.len());

        for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
            let obj = obj.map_err(|e| (e, self.path.clone()))?;

            let name = obj.file_name();

            let bytes = name.as_encoded_bytes();

            if size_of_val(bytes) != FORMATTED_HASH_SIZE || name.to_str().is_none() {
                return Err(EvsError::CorruptStateDetected(
                    CorruptState::InvalidObjectName(name),
                ));
            }

            let (hash, obj) = self.lookup(name.to_str().unwrap())?;

            trace!("Validated \"{}\".", HashDisplay(&hash));

            match obj {
                Object::Null => trace!("Found the NULL object! :)"),
                Object::Blob(data) => trace!("Found blob of size {}.", data.len()),
                Object::Tree(items) => {
                    trace!("Found tree with {} child(ren).", items.len());

                    for item in items {
                        trace!(
                            "Requiring \"{}\" for \"{}\".",
                            HashDisplay(&item.content),
                            HashDisplay(&hash)
                        );

                        required.insert(item.content);
                        #[allow(
                            clippy::arithmetic_side_effects,
                            reason = "Never going to happen."
                        )]
                        dependencies.insert(
                            item.content,
                            dependencies.get(&item.content).unwrap_or(&0) + 1,
                        );
                    }
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

                    required.insert(commit.tree);
                    #[allow(clippy::arithmetic_side_effects, reason = "Never going to happen.")]
                    dependencies.insert(
                        commit.tree,
                        dependencies.get(&commit.tree).unwrap_or(&0) + 1,
                    );

                    trace!(
                        "Requiring \"{}\" for \"{}\".",
                        HashDisplay(&commit.parent),
                        HashDisplay(&hash)
                    );

                    required.insert(commit.parent);
                    #[allow(clippy::arithmetic_side_effects, reason = "Never going to happen.")]
                    dependencies.insert(
                        commit.parent,
                        dependencies.get(&commit.parent).unwrap_or(&0) + 1,
                    );
                }
            }

            found.insert(hash);
        }

        let unnecessary_count = found
            .difference(&required)
            .map(|n| dependencies.insert(*n, 0))
            .count();

        trace!(
            "Finished validating {}/{} (+{}) objects.",
            required.intersection(&found).count(),
            required.len(),
            unnecessary_count,
        );

        let mut missing = required.difference(&found).copied();

        if let Some(first) = missing.next() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::MissingObjects(first, missing.count()),
            ));
        }

        if let Some(dependency_info) = dependency_info {
            trace!("Storing dependency info.");

            let _ = dependency_info.insert(dependencies);
        }

        Ok(found)
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn remove(&self, path: Hash) -> Result<(), EvsError> {
        debug!("Store::remove(self, \"{}\")", HashDisplay(&path));

        let path = self.path.join(format!("{}", HashDisplay(&path)));

        trace!("Deleting {:?}", &path);

        fs::remove_file(&path).map_err(|e| (e, path))?;

        Ok(())
    }

    #[inline]
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn resolve_rest(&self, r#ref: String) -> Result<String, EvsError> {
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
    #[instrument(level = "debug", err(level = "debug"), skip_all)]
    pub fn status(&self) -> Result<(usize, usize), EvsError> {
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
