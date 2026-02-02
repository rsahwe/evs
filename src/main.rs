use clap::{Parser, Subcommand};

/// Ev source control.
///
/// Basically a git clone.
#[derive(Parser, Debug)]
#[command(version, about = "Ev source control")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    //TODO:
    Test,
}

fn main() {
    let cli = Cli::parse();

    //TODO:
    let _ = cli;
}
