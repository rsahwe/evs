use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

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
    #[arg(short, action(ArgAction::Count))]
    pub verbose: u8,

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
    Check,
}
