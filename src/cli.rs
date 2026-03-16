use std::{
    io::{Write, stdout},
    path::{Path, PathBuf},
    time::SystemTime,
};

use clap::{ArgAction, Parser, Subcommand};
use enable_ansi_support::enable_ansi_support;

use crate::{
    diff::DiffSide,
    error::EvsError,
    log, none,
    repo::Repository,
    store::{Hash, HashDisplay},
    verbose,
};

pub const VERBOSITY_NONE: u8 = 0;
pub const VERBOSITY_LOG: u8 = 1;
pub const VERBOSITY_TRACE: u8 = 2;
pub const VERBOSITY_ALL: u8 = 3;

/// Ev source control.
///
/// Basically a git clone.
#[derive(Parser, Debug)]
#[command(version, about = "Ev source control")]
pub struct Cli {
    /// Increases the verbosity level by one each time it appears.
    #[arg(short, action(ArgAction::Count), global(true))]
    pub verbose: u8,

    /// Disables color printing which can also be done by setting the NO_COLOR variable to something.
    #[arg(short, long, global(true))]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initializes an evs repository in the current directory.
    Init {
        /// The location of the workspace.
        path: Option<PathBuf>,
    },
    /// Checks the evs store for validity and completeness.
    Check,
    /// Prints the given object from the store.
    Cat {
        /// Prints the raw bytes of an object in msgpack format.
        #[arg(short, long, default_value_t = false)]
        raw: bool,
        r#ref: String,
    },
    /// Adds the given files and directories to the evs store and stage
    Add {
        paths: Vec<PathBuf>,
    },
    /// Removes the given files and directories from the evs stage
    Sub {
        paths: Vec<PathBuf>,
    },
    /// Commits the current stage to the commit chain.
    Commit {
        /// The commit message, currently not optional.
        #[arg(short, long)]
        message: String,
        /// The committer name, currently not optional.
        #[arg(short, long)]
        name: String,
        /// The committer email, currently not optional.
        #[arg(short, long)]
        email: String,
    },
    /// Prints the commit log of a commit.
    Log {
        /// The maximum number of commits to log.
        #[arg(short, long, default_value_t = 5)]
        limit: usize,
        /// The commit to start the log from.
        #[arg(default_value = "HEAD")]
        r#ref: String,
    },
    /// Collects all unreferenced store objects and deletes them.
    Gc,
    /// Prints the resolved store object of a given path
    Resolve {
        r#ref: String,
    },
    Diff {
        /// Switches to stage comparison. This compares the stage with the previous commit.
        #[arg(long, group("from_group"), group("to_group"))]
        staged: bool,
        /// The base reference for the diff. Defaults to the stage.
        #[arg(short, long, group("from_group"))]
        from: Option<String>,
        /// The compared reference for the diff. Defaults to the worktree.
        #[arg(short, long, group("to_group"))]
        to: Option<String>,
        /// The paths to compare. Defaults to [ "." ].
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
    },
}

impl Cli {
    pub fn run(mut self) -> Result<(), EvsError> {
        if enable_ansi_support().is_err() {
            self.no_color = true;
        }

        macro_rules! get_repo {
            () => {{
                log!(
                    &self,
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let repo = Repository::find(".", &self)?;

                log!(&self, "Found repository at {:?}.", repo.repository);

                repo
            }};
        }

        match &self.command {
            Commands::Init { path } => {
                let path = path.as_ref().map(ToOwned::to_owned).unwrap_or(".".into());

                log!(&self, "Creating repository at {:?}...", path);

                let repo = Repository::create(path, &self)?;

                log!(&self, "Created repository.");

                drop(repo);

                none!("Repository initialized successfully.");
            }
            Commands::Check => {
                let repo = get_repo!();

                repo.check(&self)?;

                drop(repo);

                none!("Repository checked successfully.");
            }
            Commands::Cat { raw, r#ref } => {
                let repo = get_repo!();

                let (hash, obj) = repo.lookup(r#ref, &self)?;

                log!(&self, "Printing object \"{}\":", HashDisplay(&hash));

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
                let mut repo = get_repo!();

                verbose!(&self, "Adding {} paths:", paths.len());

                for file in paths {
                    repo.add(file, &self)?;

                    log!(&self, "Added {:?}", file);
                }

                log!(&self, "Finished adding.")
            }
            Commands::Sub { paths } => {
                let mut repo = get_repo!();

                verbose!(&self, "Removing {} paths:", paths.len());

                for file in paths {
                    repo.sub(file, &self)?;

                    log!(&self, "Removed {:?}", file);
                }

                log!(&self, "Finished removing.")
            }
            Commands::Commit {
                message,
                name,
                email,
            } => {
                let mut repo = get_repo!();

                let time = SystemTime::now();

                verbose!(
                    &self,
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
                    &self,
                )?;

                log!(&self, "Finished committing.");

                none!("HEAD is now at \"{}\".", HashDisplay(&commit));
            }
            Commands::Log { r#ref, limit } => {
                let repo = get_repo!();

                repo.log(r#ref, *limit, &self)?;

                log!(&self, "Finished printing log.");
            }
            Commands::Gc => {
                let repo = get_repo!();

                repo.gc(&self)?;

                log!(&self, "Finished collecting garbage.");
            }
            Commands::Resolve { r#ref } => {
                let repo = get_repo!();

                let hash = repo.resolve(&r#ref, &self)?;

                verbose!(&self, "\"{}\" resolved to \"{}\".", r#ref, hash);

                none!("{}", hash);
            }
            Commands::Diff {
                staged,
                from,
                to,
                paths,
            } => {
                let repo = get_repo!();

                let (from, to) = if *staged {
                    let to = DiffSide::Tree(repo.info.stage());

                    let from = DiffSide::Tree(repo.get_tree(repo.info.head(), &self)?);

                    (from, to)
                } else {
                    (
                        DiffSide::Tree(
                            from.as_ref()
                                .map(|f| {
                                    Ok::<Hash, EvsError>(
                                        repo.get_tree(repo.lookup(f, &self)?.0, &self)?,
                                    )
                                })
                                .transpose()?
                                .unwrap_or(repo.info.stage()),
                        ),
                        to.as_ref()
                            .map(|t| {
                                Ok::<DiffSide, EvsError>(DiffSide::Tree(
                                    repo.get_tree(repo.lookup(t, &self)?.0, &self)?,
                                ))
                            })
                            .transpose()?
                            .unwrap_or(DiffSide::Local(repo.workspace.clone(), true)),
                    )
                };

                verbose!(&self, "Diffing from {:?} to {:?}:", from, to);

                DiffSide::diff_with(
                    from,
                    to,
                    &repo.store,
                    paths
                        .iter()
                        .map(|p| {
                            p.canonicalize()
                                .map_err(|e| (e, p.clone()))?
                                .strip_prefix(&repo.workspace)
                                .map(|p| p.to_path_buf())
                                .map_err(|_| EvsError::PathOutsideOfRepo(p.clone()))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    &self,
                )?;

                log!(&self, "Finished diff.");
            }
        }

        Ok(())
    }
}
