use std::{io::stdout, path::Path};

use clap::Parser;
use evs::{
    cli::{Cli, Commands},
    error::EvsError,
    log, none,
    repo::Repository,
    store::HashDisplay,
};

fn main() {
    if let Err(e) = || -> Result<(), EvsError> {
        let cli = Cli::parse();

        match &cli.command {
            Commands::Init { path } => {
                let path = path.as_ref().map(ToOwned::to_owned).unwrap_or(".".into());

                log!(&cli, "Creating repository at {:?}...", path);

                let repo = Repository::create(path, &cli)?;

                log!(&cli, "Created repository.");

                drop(repo);

                none!("Repository initialized successfully.");
            }
            Commands::Check => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                repo.check(&cli)?;

                drop(repo);

                none!("Repository checked successfully.");
            }
            Commands::Cat { raw, r#ref } => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                let (hash, obj) = repo.store.lookup(&r#ref, &cli)?;

                log!(&cli, "Printing object \"{}\":", HashDisplay(&hash));

                if !raw {
                    println!("{}", obj);
                } else {
                    serde_cbor::to_writer(stdout(), &obj).expect("cbor failed");
                }
            }
        }

        Ok(())
    }() {
        none!("{}", e);
    }
}
