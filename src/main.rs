use clap::{CommandFactory, Parser};
use enable_ansi_support::enable_ansi_support;
use evs::{cli::Cli, util::get_color};
use tracing::{Level, subscriber::set_global_default};
use tracing_subscriber::{EnvFilter, FmtSubscriber, fmt::format::FmtSpan};

fn main() {
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let mut cli = Cli::parse();

    if enable_ansi_support().is_err() {
        cli.no_color = true;
    }

    let subscriber = FmtSubscriber::builder()
        .with_ansi(get_color(&cli))
        .with_ansi_sanitization(true)
        .with_file(true)
        .with_level(true)
        .with_line_number(true)
        .with_span_events(FmtSpan::NONE)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(false)
        .with_env_filter(EnvFilter::from_default_env())
        .without_time()
        .with_max_level(match cli.verbose {
            0 => Level::WARN,
            1 => Level::INFO,
            2 => Level::DEBUG,
            _ => Level::TRACE,
        })
        .compact()
        .finish();

    set_global_default(subscriber).unwrap();

    if let Err(e) = cli.command.run(&cli) {
        println!("{}", e);
    }
}
