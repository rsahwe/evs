use std::{
    io::{Write, stdout},
    path::Path,
    time::SystemTime,
};

use clap::Parser;
use evs::{
    cli::{Cli, Commands},
    error::EvsError,
    log, none,
    repo::Repository,
    store::HashDisplay,
    verbose,
};

fn main() {
    //TODO: MOVE INTO LIB
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

                let (hash, obj) = repo.lookup(r#ref, &cli)?;

                log!(&cli, "Printing object \"{}\":", HashDisplay(&hash));

                if !raw {
                    println!("{}", obj);
                } else {
                    let content = rmp_serde::to_vec(&obj).expect("msgpack failed");

                    stdout()
                        .write_all(&content)
                        .expect("write to stdout failed");
                }
            }
            Commands::Add { paths } => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let mut repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                verbose!(&cli, "Adding {} paths:", paths.len());

                for file in paths {
                    repo.add(file, &cli)?;

                    log!(&cli, "Added {:?}", file);
                }

                log!(&cli, "Finished adding.")
            }
            Commands::Sub { paths } => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let mut repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                verbose!(&cli, "Removing {} paths:", paths.len());

                for file in paths {
                    repo.sub(file, &cli)?;

                    log!(&cli, "Removed {:?}", file);
                }

                log!(&cli, "Finished removing.")
            }
            Commands::Commit {
                message,
                name,
                email,
            } => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let mut repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                let time = SystemTime::now();

                verbose!(
                    &cli,
                    "Committing by {} <{}> at {:?} with message of length {}",
                    name,
                    email,
                    time,
                    message.len()
                );

                let commit = repo.commit(
                    message.to_owned(),
                    name.to_owned(),
                    email.to_owned(),
                    time,
                    &cli,
                )?;

                log!(&cli, "Finished committing.");

                none!("HEAD is now at \"{}\".", HashDisplay(&commit));
            }
            Commands::Log { r#ref, limit } => {
                log!(
                    &cli,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let repo = Repository::find(".", &cli)?;

                log!(&cli, "Found repository at {:?}.", repo.repository);

                repo.log(r#ref, *limit, &cli)?;

                log!(&cli, "Finished printing log.");
            }
        }

        Ok(())
    }() {
        none!("{}", e);
    }
}
