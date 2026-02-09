use clap::Parser;
use evs::{cli::Cli, none};

fn main() {
    if let Err(e) = Cli::parse().run() {
        none!("{}", e);
    }
}
