use std::{fmt::Display, fs::OpenOptions, io::Write, path::PathBuf};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

use crate::{
    cli::{Cli, VERBOSITY_ALL, VERBOSITY_TRACE},
    error::EvsError,
    util::DropAction,
};

pub type Hash = [u8; 32];
pub type PartialHash<'a> = &'a [u8];

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

pub const NULL_HASH: Hash = [
    0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
    0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
];

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

        let _drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::insert(self, ...) done");
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

        if options.verbose >= VERBOSITY_ALL {
            eprintln!("### Data hashed to {}.", HashDisplay(&hash));
        }

        let target = self.path.join(&format!("{}", HashDisplay(&hash)));

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

            Ok(hash)
        }
    }

    pub fn lookup(&self, id: PartialHash, options: &Cli) -> Result<Vec<u8>, EvsError> {
        if size_of_val(id) > size_of::<Hash>() {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::lookup(self, <overlength hash>) errored");
            }

            return Err(EvsError::ObjectNotInStore(id.to_owned()));
        }

        if options.verbose >= VERBOSITY_TRACE {
            eprintln!(
                "## Store::lookup(self, {}{})",
                HashDisplay(id),
                if id.len() < 32 { "..." } else { "" }
            );
        }

        let _drop = DropAction(|| {
            if options.verbose >= VERBOSITY_TRACE {
                eprintln!("## Store::lookup(self, ...) done");
            }
        });

        todo!("lookup")
    }
}
