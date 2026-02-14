use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    fs::{self, OpenOptions},
    io::{Read, Write},
    mem::ManuallyDrop,
    path::PathBuf,
};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::{
    cli::Cli,
    error::{CorruptState, EvsError},
    objects::Object,
    trace,
    util::DropAction,
    verbose,
};

pub type Hash = [u8; 32];
pub type PartialHash<'a> = &'a [u8];

/// Needs to double the length of a hash (it does)
#[derive(Debug)]
pub struct HashDisplay<'a>(pub PartialHash<'a>);

impl<'a> Display for HashDisplay<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Assumes a valid store and might cause unintended behaviour
    pub fn insert(&self, mut obj: Object, options: &Cli) -> Result<Hash, EvsError> {
        trace!(options, "Store::insert(self, ...)");

        let drop = DropAction(|| {
            trace!(options, "Store::insert(self, ...) error");
        });

        match &mut obj {
            Object::Tree(entries) => {
                entries.sort_by(|a, b| a.name.cmp(&b.name));
            }
            _ => (),
        }

        let data = rmp_serde::to_vec(&obj).expect("msgpack failed");

        verbose!(options, "Serialized object to size {}", data.len());

        let hash: Hash = Sha256::digest(&data).into();

        let hash_display = format!("{}", HashDisplay(&hash));

        verbose!(options, "Data hashed to \"{}\".", hash_display);

        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());

        encoder
            .write_all(&data)
            .expect("gzip encoder failed: io error on vec");

        let compressed = encoder
            .finish()
            .expect("gzip encoder failed: io error on vec");

        verbose!(
            options,
            "Compressed data from {} to {} bytes.",
            data.len(),
            compressed.len()
        );

        let target = self.path.join(&hash_display);

        let hash = if target.exists() {
            verbose!(
                options,
                "Object path exists! TODO: Maybe figure out strategy for this."
            );

            hash
        } else {
            verbose!(options, "Object path does not exist, inserting...");

            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|e| (e, target.clone()))?;

            file.write_all(&compressed)
                .map_err(|e| (e, target.clone()))?;

            verbose!(options, "Wrote object to store.");

            hash
        };

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Store::insert(self, ...) done");

        Ok(hash)
    }

    pub fn lookup(&self, id: &str, options: &Cli) -> Result<(Hash, Object), EvsError> {
        if size_of_val(id) > size_of::<Hash>() * 2 {
            trace!(options, "Store::lookup(self, <overlength hash>) wrong args");

            return Err(EvsError::ObjectNotInStore(id.to_owned()));
        }

        trace!(
            options,
            "Store::lookup(self, \"{}{}\")",
            id,
            if size_of_val(id) < size_of::<Hash>() * 2 {
                "..."
            } else {
                ""
            }
        );

        let drop = DropAction(|| {
            trace!(options, "Store::lookup(self, ...) error");
        });

        let mut target = None;

        if size_of_val(id) == size_of::<Hash>() * 2 {
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

        if size_of_val(target_name) != size_of::<Hash>() * 2
            || !target_name
                .as_encoded_bytes()
                .iter()
                .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::InvalidObjectName(target_name.to_owned()),
            ));
        }

        verbose!(options, "Found object {:?}.", target);

        let content = fs::read(&target).map_err(|e| (e, target.clone()))?;

        verbose!(options, "Read object of compressed size {}.", content.len());

        let mut decoder = GzDecoder::new(&*content);

        let mut decompressed = vec![];

        decoder.read_to_end(&mut decompressed).map_err(|e| {
            EvsError::CorruptStateDetected(CorruptState::InvalidCompression(target.clone(), e))
        })?;

        verbose!(options, "Decompressed to size {}.", decompressed.len());

        let real_hash: Hash = Sha256::digest(&decompressed).into();

        if *target_name != *format!("{}", HashDisplay(&real_hash)) {
            return Err(EvsError::CorruptStateDetected(CorruptState::HashMismatch(
                target_name.to_owned(),
                real_hash.to_vec(),
            )));
        }

        verbose!(options, "Validated hash.");

        let deserialized =
            rmp_serde::from_slice::<Object>(&decompressed).map_err(|e| (e, real_hash))?;

        verbose!(options, "Deserialized successfully.");

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Store::lookup(self, ...) done");

        Ok((real_hash, deserialized))
    }

    pub fn check(
        &self,
        mut found: HashSet<Hash>,
        required: impl AsRef<[Hash]>,
        dependency_info: Option<&mut Option<HashMap<Hash, usize>>>,
        options: &Cli,
    ) -> Result<HashSet<Hash>, EvsError> {
        trace!(
            options,
            "Store::check(self, <{} hash(es)>)",
            required.as_ref().len()
        );

        let drop = DropAction(|| {
            trace!(options, "Store::check(self, ...) error");
        });

        let (mut required, mut dependencies) = {
            let mut hm = HashSet::new();
            let mut dep = HashMap::new();

            for r in required.as_ref() {
                hm.insert(*r);
                dep.insert(*r, 1);
            }

            (hm, dep)
        };

        verbose!(
            options,
            "Initially required to find {} object(s).",
            required.len()
        );

        for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
            let obj = obj.map_err(|e| (e, self.path.clone()))?;

            let name = obj.file_name();

            let bytes = name.as_encoded_bytes();

            if size_of_val(bytes) != size_of::<Hash>() * 2 || name.to_str().is_none() {
                return Err(EvsError::CorruptStateDetected(
                    CorruptState::InvalidObjectName(name),
                ));
            }

            let (hash, obj) = self.lookup(name.to_str().unwrap(), options)?;

            verbose!(options, "Validated \"{}\".", HashDisplay(&hash));

            match obj {
                Object::Null => verbose!(options, "Found the NULL object! :)"),
                Object::Blob(data) => verbose!(options, "Found blob of size {}.", data.len()),
                Object::Tree(items) => {
                    verbose!(options, "Found tree with {} child(ren).", items.len());

                    for item in items {
                        verbose!(
                            options,
                            "Requiring \"{}\" for \"{}\".",
                            HashDisplay(&item.content),
                            HashDisplay(&hash)
                        );

                        required.insert(item.content);
                        dependencies.insert(
                            item.content,
                            dependencies.get(&item.content).unwrap_or(&0) + 1,
                        );
                    }
                }
                Object::Commit(commit) => {
                    verbose!(
                        options,
                        "Found commit with state \"{}\" and parent \"{}\".",
                        HashDisplay(&commit.tree),
                        HashDisplay(&commit.parent)
                    );

                    verbose!(
                        options,
                        "Requiring \"{}\" for \"{}\".",
                        HashDisplay(&commit.tree),
                        HashDisplay(&hash)
                    );

                    required.insert(commit.tree);
                    dependencies.insert(
                        commit.tree,
                        dependencies.get(&commit.tree).unwrap_or(&0) + 1,
                    );

                    verbose!(
                        options,
                        "Requiring \"{}\" for \"{}\".",
                        HashDisplay(&commit.parent),
                        HashDisplay(&hash)
                    );

                    required.insert(commit.parent);
                    dependencies.insert(
                        commit.parent,
                        dependencies.get(&commit.parent).unwrap_or(&0) + 1,
                    );
                }
            }

            found.insert(hash);
        }

        let unnecessary_count = found.difference(&required).fold(0, |acc, n| {
            dependencies.insert(*n, 0);
            acc + 1
        });

        verbose!(
            options,
            "Finished validating {}/{} (+{}) objects.",
            required.intersection(&found).count(),
            required.len(),
            unnecessary_count,
        );

        let mut missing = required.difference(&found).cloned();

        if let Some(first) = missing.next() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::MissingObjects(first, missing.count()),
            ));
        }

        if let Some(dependency_info) = dependency_info {
            verbose!(options, "Storing dependency info.");

            let _ = dependency_info.insert(dependencies);
        }

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Store::check(self, ...) done");

        Ok(found)
    }

    pub fn remove(&self, path: Hash, options: &Cli) -> Result<(), EvsError> {
        trace!(options, "Store::remove(self, \"{}\")", HashDisplay(&path));

        let drop = DropAction(|| {
            trace!(options, "Store::remove(self, ...) error");
        });

        let path = self.path.join(&format!("{}", HashDisplay(&path)));

        verbose!(options, "Deleting {:?}", &path);

        fs::remove_file(&path).map_err(|e| (e, path))?;

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Store::remove(self, ...) done");

        Ok(())
    }

    pub fn resolve_rest(&self, r#ref: String, options: &Cli) -> Result<String, EvsError> {
        trace!(options, "Store::resolve_rest(self, \"{}\")", r#ref);

        let drop = DropAction(|| {
            trace!(options, "Store::resolve_rest(self, ...) error");
        });

        let mut target = None;

        if size_of_val(r#ref.as_str()) == size_of::<Hash>() * 2 {
            let path = self.path.join(&r#ref);

            verbose!(options, "Fast lookup of {:?}...", path);

            target = fs::exists(&path).is_ok().then_some(path);
        } else {
            verbose!(options, "Slow lookup...");

            for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
                let obj = obj.map_err(|e| (e, self.path.clone()))?;

                let name = obj.path();

                if let Some(hash) = name.file_name()
                    && hash.as_encoded_bytes().starts_with(r#ref.as_bytes())
                {
                    verbose!(options, "Found {:?}.", hash);

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

        if size_of_val(target_name) != size_of::<Hash>() * 2
            || !target_name
                .as_encoded_bytes()
                .iter()
                .all(|b| matches!(*b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::InvalidObjectName(target_name.to_owned()),
            ));
        }

        verbose!(options, "Validated name successfully.");

        let resolved = target_name.to_str().unwrap().to_owned();

        let _ = ManuallyDrop::new(drop);

        trace!(options, "Store::resolve_rest(self, ...) done");

        Ok(resolved)
    }
}
