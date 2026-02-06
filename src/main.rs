use std::path::Path;

use clap::Parser;
use evs::{
    cli::{Cli, Commands},
    error::EvsError,
    log, none,
    repo::Repository,
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
        }

        Ok(())
    }() {
        none!("{}", e);
    }
}
