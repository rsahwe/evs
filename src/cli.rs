use std::{
    borrow::Cow,
    io::{Write as _, stdout},
    path::{Path, PathBuf},
    time::SystemTime,
};

use ahash::AHashSet;
use clap::{ArgAction, CommandFactory as _, Parser, Subcommand, ValueHint};
use clap_complete::ArgValueCompleter;
use tracing::{Span, info, trace};

use crate::{
    diff::DiffSide,
    error::{CorruptState, EvsError},
    objects::Object,
    repo::Repository,
    store::HashDisplay,
    util::{partial_canonicalize, repo_ref_completer},
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

    /// Disables color printing which can also be done by setting the `NO_COLOR` variable to something.
    #[arg(long, global(true))]
    pub no_color: bool,

    /// Forces color printing even if automatically or directly disabled.
    #[arg(long, global(true))]
    pub force_color: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initializes an evs repository in the current directory.
    Init {
        /// The location of the workspace.
        #[arg(value_hint(ValueHint::DirPath))]
        path: Option<PathBuf>,
    },
    /// Checks the evs store for validity and completeness.
    Check {
        /// Whether to check all objects in the store or only the required ones.
        #[arg(short, long, default_value_t = false)]
        all: bool,
    },
    /// Prints the given object from the store.
    Cat {
        /// Prints the raw bytes of an object in msgpack format.
        #[arg(short, long, default_value_t = false)]
        raw: bool,
        /// The object to print.
        #[arg(add(ArgValueCompleter::new(repo_ref_completer)))]
        r#ref: String,
    },
    /// Adds the given files and directories to the evs store and stage.
    Add {
        /// The list of files and directories to add.
        #[arg(value_hint(ValueHint::AnyPath))]
        paths: Vec<PathBuf>,
    },
    /// Removes the given files and directories from the evs stage.
    Sub {
        /// The list of files and directories to remove.
        #[arg(value_hint(ValueHint::AnyPath))]
        paths: Vec<PathBuf>,
    },
    /// Commits the current stage to the commit chain.
    Commit {
        /// Whether to modify the previous commit instead of creating a new one or not.
        #[arg(long)]
        amend: bool,
        /// The commit message, currently not optional.
        #[arg(short, long, value_hint(ValueHint::Other))]
        message: Option<String>,
        /// The committer name, currently not optional.
        #[arg(short, long, value_hint(ValueHint::Username))]
        name: Option<String>,
        /// The committer email, currently not optional.
        #[arg(short, long, value_hint(ValueHint::Other))]
        email: Option<String>,
    },
    /// Prints the commit log of a commit.
    Log {
        /// The maximum number of commits to log.
        #[arg(short, long, default_value_t = 5, value_hint(ValueHint::Other))]
        limit: usize,
        /// Prints every commit on only one line.
        #[arg(short, long)]
        oneline: bool,
        /// The commit to start the log from.
        #[arg(
            default_value = "HEAD",
            add(ArgValueCompleter::new(repo_ref_completer))
        )]
        r#ref: String,
    },
    /// Collects all unreferenced store objects and deletes them.
    Gc,
    /// Prints the resolved store object of a given path.
    Resolve {
        /// The store expression to resolve.
        #[arg(add(ArgValueCompleter::new(repo_ref_completer)))]
        r#ref: String,
    },
    /// Prints the difference between two repository states in the unified diff format.
    Diff {
        /// Switches to stage comparison. This compares the stage with the previous commit.
        #[arg(long, group("from_group"), group("to_group"))]
        staged: bool,
        /// The base reference for the diff. Defaults to the stage.
        #[arg(
            short,
            long,
            group("from_group"),
            add(ArgValueCompleter::new(repo_ref_completer))
        )]
        from: Option<String>,
        /// The compared reference for the diff. Defaults to the worktree.
        #[arg(
            short,
            long,
            group("to_group"),
            add(ArgValueCompleter::new(repo_ref_completer))
        )]
        to: Option<String>,
        /// The paths to compare. Defaults to the current directory.
        // TODO: FIX?
        #[arg(default_value = ".", value_hint(ValueHint::AnyPath))]
        paths: Vec<PathBuf>,
    },
    /// Prints the repository status including commit status, changes, staged changes and object count.
    Status,
    /// Shows the diff generated by the commit of the given path.
    Show {
        /// The commit to show the diff of.
        #[arg(add(ArgValueCompleter::new(repo_ref_completer)))]
        r#ref: String,
    },
    /// Switches the state of the worktree to a different commit.
    Checkout {
        /// Whether or not to discard staged changes.
        #[arg(short, long)]
        force: bool,
        /// The commit to checkout.
        #[arg(add(ArgValueCompleter::new(repo_ref_completer)))]
        r#ref: String,
    },
    #[doc(hidden)]
    #[clap(hide(true))]
    Mangen {
        #[arg(value_hint(ValueHint::DirPath))]
        dir: PathBuf,
    },
    #[doc(hidden)]
    #[clap(hide(true))]
    Completion,
}

impl Commands {
    #[allow(
        clippy::too_many_lines,
        reason = "This is just because of the number of subcommands + this is totally fine due to the separation in the match."
    )]
    #[inline]
    pub fn run(
        &self,
        options: &Cli,
    ) -> Result<(), EvsError> {
        let current = Span::current();

        macro_rules! get_repo {
            () => {{
                info!(
                    "Searching for repository starting from {:?}:",
                    AsRef::<Path>::as_ref(".")
                );

                let repo = Repository::find(&current, ".", options)?;

                info!("Found repository at {:?}.", repo.repository);

                repo
            }};
        }

        match self {
            Commands::Init { path } => {
                let path = path.as_ref().map_or(".".into(), ToOwned::to_owned);

                info!("Creating repository at {:?}...", path);

                let repo = Repository::create(&current, path, options)?;

                info!("Created repository.");

                drop(repo);

                println!("Repository initialized successfully.");
            }
            Commands::Check { all } => {
                let repo = get_repo!();

                repo.check(&current, *all)?;

                drop(repo);

                println!("Repository checked successfully.");
            }
            Commands::Cat { raw, r#ref } => {
                let repo = get_repo!();

                let (hash, obj) = repo.lookup(&current, r#ref)?;

                info!("Printing object \"{}\":", HashDisplay(&hash));

                if !raw {
                    println!("{}", obj);
                } else {
                    let content = rmp_serde::to_vec(&obj)?;

                    let _ = stdout().write_all(&content);
                }
            }
            Commands::Add { paths } => {
                let mut repo = get_repo!();

                trace!("Adding {} paths:", paths.len());

                let (set, map) = DiffSide::Tree(repo.info.stage()).read(
                    &current,
                    "",
                    &repo.store,
                    &[AsRef::<Path>::as_ref("").to_path_buf()],
                    &[],
                    &AHashSet::new(),
                )?;

                drop(map);

                for file in paths {
                    repo.add(&current, file, &set, options)?;

                    info!("Added {:?}", file);
                }

                info!("Finished adding.");
            }
            Commands::Sub { paths } => {
                let mut repo = get_repo!();

                trace!("Removing {} paths:", paths.len());

                for file in paths {
                    repo.sub(&current, file)?;

                    info!("Removed {:?}", file);
                }

                info!("Finished removing.");
            }
            Commands::Commit {
                amend,
                message,
                name,
                email,
            } => {
                let mut repo = get_repo!();

                let time = SystemTime::now();

                let mut message = message.as_ref().map(Cow::Borrowed);
                let mut name = name.as_ref().map(Cow::Borrowed);
                let mut email = email.as_ref().map(Cow::Borrowed);

                let mut amend_parent = None;

                if *amend {
                    let (_, Object::Commit(commit)) = repo.lookup(&current, "HEAD")? else {
                        return Err(EvsError::CorruptStateDetected(
                            CorruptState::HeadIsNotACommit,
                        ));
                    };

                    trace!("Amending with {:?}", commit);

                    message.get_or_insert(Cow::Owned(commit.msg));
                    name.get_or_insert(Cow::Owned(commit.name));
                    email.get_or_insert(Cow::Owned(commit.email));
                    amend_parent = Some(commit.parent);
                }

                let Some(name) = name else {
                    return Err(EvsError::MissingCommitInfo("committer name"));
                };

                let Some(email) = email else {
                    return Err(EvsError::MissingCommitInfo("commiter email"));
                };

                let Some(message) = message else {
                    return Err(EvsError::MissingCommitInfo("commit message"));
                };

                trace!(
                    "Committing by {} <{}> at {:?} with message of length {}",
                    name,
                    email,
                    time,
                    message.len()
                );

                let commit = repo.commit(
                    &current,
                    amend_parent,
                    message.into_owned(),
                    name.into_owned(),
                    email.into_owned(),
                    time,
                    options,
                )?;

                info!("Finished committing.");

                println!("HEAD is now at \"{}\".", HashDisplay(&commit));
            }
            Commands::Log {
                r#ref,
                limit,
                oneline,
            } => {
                let repo = get_repo!();

                repo.log(&current, r#ref, *limit, *oneline, options)?;

                info!("Finished printing log.");
            }
            Commands::Gc => {
                let repo = get_repo!();

                repo.gc(&current, options)?;

                info!("Finished collecting garbage.");
            }
            Commands::Resolve { r#ref } => {
                let repo = get_repo!();

                let hash = repo.resolve(&current, r#ref)?;

                trace!("\"{}\" resolved to \"{}\".", r#ref, hash);

                println!("{}", hash);
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

                    let from = DiffSide::Tree(repo.get_tree(&current, repo.info.head())?);

                    (from, to)
                } else {
                    (
                        DiffSide::Tree(
                            from.as_ref()
                                .map(|f| repo.get_tree(&current, repo.lookup(&current, f)?.0))
                                .transpose()?
                                .unwrap_or(repo.info.stage()),
                        ),
                        to.as_ref()
                            .map(|t| {
                                Ok::<DiffSide, EvsError>(DiffSide::Tree(
                                    repo.get_tree(&current, repo.lookup(&current, t)?.0)?,
                                ))
                            })
                            .transpose()?
                            .unwrap_or(DiffSide::Local(repo.workspace.clone())),
                    )
                };

                trace!("Diffing from {:?} to {:?}:", from, to);

                DiffSide::diff_with(
                    from,
                    to,
                    &current,
                    &repo.store,
                    paths
                        .iter()
                        .map(|p| {
                            partial_canonicalize(&current, p)
                                .map_err(|e| (e, p.clone()))?
                                .strip_prefix(&repo.workspace)
                                .map(Path::to_path_buf)
                                .map_err(|_e| EvsError::PathOutsideOfRepo(p.clone()))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    repo.get_ignores(&current, options)?,
                    options,
                )?;

                info!("Finished diff.");
            }
            Commands::Status => {
                let repo = get_repo!();

                repo.status(&current, options)?;

                info!("Finished reporting status.");
            }
            Commands::Show { r#ref } => {
                let repo = get_repo!();

                repo.show(&current, r#ref, options)?;

                info!("Finished showing commit.");
            }
            Commands::Checkout { force, r#ref } => {
                let mut repo = get_repo!();

                let hash = repo.checkout(&current, r#ref, *force, options)?;

                println!("Checked out \"{}\" successfully.", HashDisplay(&hash));
            }
            Commands::Mangen { dir } => {
                let command = Cli::command();

                clap_mangen::generate_to(command, dir).map_err(|e| (e, dir.clone()))?;
            }
            Commands::Completion => unreachable!("Fake command for completion engine"),
        }

        Ok(())
    }
}
