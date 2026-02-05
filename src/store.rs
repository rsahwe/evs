use std::{
    collections::HashSet,
    fmt::Display,
    fs::{self, OpenOptions},
    io::{Read, Write},
    mem::ManuallyDrop,
    path::PathBuf,
};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::{
    cli::{Cli, VERBOSITY_ALL, VERBOSITY_TRACE},
    error::{CorruptState, EvsError},
    util::DropAction,
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
    pub fn insert(&self, data: &[u8], options: &Cli) -> Result<Hash, EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Store::insert(self, <data of size {}>)", data.len());
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::insert(self, ...) error");
            }
        });

        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());

        encoder
            .write_all(data)
            .expect("gzip encoder failed: io error on vec");

        let compressed = encoder
            .finish()
            .expect("gzip encoder failed: io error on vec");

        if options.verbose >= VERBOSITY_ALL {
            eprintln!(
                "### Compressed data from {} to {} bytes.",
                data.len(),
                compressed.len()
            );
        }

        let hash: Hash = Sha256::digest(&compressed).into();

        let hash_display = format!("{}", HashDisplay(&hash));

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Data hashed to {}.", hash_display);
        }

        let target = self.path.join(&hash_display);

        if target.exists() {
            if options.verbose >= VERBOSITY_ALL {
                eprintln!("### Object path exists! TODO: Maybe figure out strategy for this.");
            }

            Ok(hash)
        } else {
            if options.verbose >= VERBOSITY_ALL {
                eprintln!("### Object path does not exist, inserting...");
            }

            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
                .map_err(|e| (e, target.clone()))?;

            file.write_all(&compressed)
                .map_err(|e| (e, target.clone()))?;

            if options.verbose >= VERBOSITY_ALL {
                eprintln!("### Wrote object to store.");
            }

            let _ = ManuallyDrop::new(drop);

            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::insert(self, ...) done");
            }

            Ok(hash)
        }
    }

    pub fn lookup(&self, id: &str, options: &Cli) -> Result<(Hash, Vec<u8>), EvsError> {
        if size_of_val(id) > size_of::<Hash>() * 2 {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::lookup(self, <overlength hash>) wrong args");
            }

            return Err(EvsError::ObjectNotInStore(id.to_owned()));
        }

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!(
                "## Store::lookup(self, \"{}{}\")",
                id,
                if size_of_val(id) < size_of::<Hash>() * 2 {
                    "..."
                } else {
                    ""
                }
            );
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::lookup(self, ...) error");
            }
        });

        let mut target = None;

        if size_of_val(id) < size_of::<Hash>() * 2 {
            let path = self.path.join(id);

            target = fs::exists(&path).is_ok().then_some(path);
        } else {
            for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
                let obj = obj.map_err(|e| (e, self.path.clone()))?;

                let name = obj.path();

                if let Some(hash) = name.file_name()
                    && hash.as_encoded_bytes().starts_with(id.as_bytes())
                {
                    if target.is_some() {
                        return Err(EvsError::AmbiguousObject(id.to_owned()));
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

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Found object {:?}.", target);
        }

        let content = fs::read(&target).map_err(|e| (e, target.clone()))?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Read object of compressed size {}.", content.len());
        }

        let real_hash: Hash = Sha256::digest(&content).into();

        if *target_name != *format!("{}", HashDisplay(&real_hash)) {
            return Err(EvsError::CorruptStateDetected(CorruptState::HashMismatch(
                target_name.to_owned(),
                real_hash.to_vec(),
            )));
        }

        let mut decoder = GzDecoder::new(&*content);

        let mut decompressed = vec![];

        decoder.read_to_end(&mut decompressed).map_err(|e| {
            EvsError::CorruptStateDetected(CorruptState::InvalidCompression(target.clone(), e))
        })?;

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Decompressed to size {}", decompressed.len());
        }

        let _ = ManuallyDrop::new(drop);

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Store::lookup(self, ...) done");
        }

        Ok((real_hash, decompressed))
    }

    pub fn check(&self, required: impl AsRef<[Hash]>, options: &Cli) -> Result<(), EvsError> {
        if options.verbose >= VERBOSITY_TRACE {
            eprintln!(
                "## Store::check(self, <{} hash(es)>)",
                required.as_ref().len()
            );
        }

        let drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::check(self, ...) error");
            }
        });

        let mut found = HashSet::new();

        let required = {
            let mut hm = HashSet::new();

            for r in required.as_ref() {
                hm.insert(*r);
            }

            hm
        };

        if options.verbose >= VERBOSITY_ALL {
            eprintln!(
                "### Initially required to find {} object(s).",
                required.len()
            );
        }

        for obj in self.path.read_dir().map_err(|e| (e, self.path.clone()))? {
            let obj = obj.map_err(|e| (e, self.path.clone()))?;

            let name = obj.file_name();

            let bytes = name.as_encoded_bytes();

            if size_of_val(bytes) != size_of::<Hash>() * 2 || name.to_str().is_none() {
                return Err(EvsError::CorruptStateDetected(
                    CorruptState::InvalidObjectName(name),
                ));
            }

            let (hash, _) = self.lookup(name.to_str().unwrap(), options)?;

            found.insert(hash);

            //TODO: CHECK IF OBJECT IS VALID AND ADD REFERENCED HASHES TO REQUIRED

            if options.verbose >= VERBOSITY_ALL {
                eprintln!("### Validated {:?}.", name);
            }
        }

        if options.verbose >= VERBOSITY_ALL {
            eprintln!(
                "### Finished validating {}/{} (+{}) objects.",
                required.intersection(&found).count(),
                required.len(),
                found.difference(&required).count(),
            );
        }

        let mut missing = required.difference(&found).cloned();

        if let Some(first) = missing.next() {
            return Err(EvsError::CorruptStateDetected(
                CorruptState::MissingObjects(first, missing.count()),
            ));
        }

        let _ = ManuallyDrop::new(drop);

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!("## Store::check(self, ...) done");
        }

        Ok(())
    }
}
