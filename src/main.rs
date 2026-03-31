use clap::{CommandFactory, Parser};
use evs::cli::Cli;

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    if let Err(e) = Cli::parse().run() {
        println!("{}", e);
    }
}
