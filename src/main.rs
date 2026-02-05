use std::path::Path;

use clap::Parser;
use evs::{
    cli::{Cli, Commands, VERBOSITY_LOG},
    error::EvsError,
    repo::Repository,
};

fn main() {
    if let Err(e) = || -> Result<(), EvsError> {
        let cli = Cli::parse();

        match &cli.command {
            Commands::Init { path } => {
                let path = path.as_ref().map(ToOwned::to_owned).unwrap_or(".".into());

                if cli.verbose >= VERBOSITY_LOG {
                    eprintln!("# Creating repository at {:?}...", path);
                }

                let repo = Repository::create(path, &cli)?;

                if cli.verbose >= VERBOSITY_LOG {
                    eprintln!("# Created repository.");
                }

                drop(repo);

                eprintln!("Repository initialized successfully.");
            }
            Commands::Check => {
                if cli.verbose >= VERBOSITY_LOG {
                    eprintln!(
                        "# Searching for repository starting from {:?}:",
                        AsRef::<Path>::as_ref(".")
                    );
                }

                let repo = Repository::find(".", &cli)?;

                if cli.verbose >= VERBOSITY_LOG {
                    eprintln!("# Found repository at {:?}.", repo.repository);
                }

                repo.check(&cli)?;

                drop(repo);

                eprintln!("Repository checked successfully.");
            }
        }

        Ok(())
    }() {
        eprintln!("{}", e);
    }
}
