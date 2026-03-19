use clap::Parser;
use evs::cli::Cli;

fn main() {
    if let Err(e) = Cli::parse().run() {
        println!("{}", e);
    }
}
