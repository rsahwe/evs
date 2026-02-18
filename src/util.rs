use std::{
    io::{BufRead, Write, stdin, stdout},
    mem::ManuallyDrop,
};

use crate::{cli::Cli, error::EvsError};

pub struct DropAction<F: Fn()>(pub F);

impl<F: Fn()> Drop for DropAction<F> {
    fn drop(&mut self) {
        (self.0)()
    }
}

#[macro_export]
macro_rules! none {
    ($($arg:tt)*) => {
        eprintln!($($arg)*)
    };
}

#[macro_export]
macro_rules! log {
    ($options:expr, $fmt:literal $($arg:tt)*) => {{
        let options: &$crate::cli::Cli = $options;
        if options.verbose >= $crate::cli::VERBOSITY_LOG {
            eprintln!(concat!("# ", $fmt) $($arg)*)
        }
    }};
}

#[macro_export]
macro_rules! trace {
    ($options:expr, $fmt:literal $($arg:tt)*) => {{
        let options: &$crate::cli::Cli = $options;
        if options.verbose >= $crate::cli::VERBOSITY_TRACE {
            eprintln!(concat!("## ", $fmt) $($arg)*)
        }
    }};
}

#[macro_export]
macro_rules! verbose {
    ($options:expr, $fmt:literal $($arg:tt)*) => {{
        let options: &$crate::cli::Cli = $options;
        if options.verbose >= $crate::cli::VERBOSITY_ALL {
            eprintln!(concat!("### ", $fmt) $($arg)*)
        }
    }};
}

pub fn confirmation(prompt: &str, default: bool, options: &Cli) -> Result<bool, EvsError> {
    trace!(options, "confirmation({}, {})", prompt, default);

    let drop = DropAction(|| {
        trace!(options, "confirmation(...) err");
    });

    let mut stdout = stdout().lock();
    let mut stdin = stdin().lock();

    stdout
        .write_fmt(format_args!(
            "{} {}: ",
            prompt,
            if default { "[Y/n]" } else { "[y/N]" }
        ))
        .map_err(|e| (e, "-".to_owned().into()))?;

    stdout.flush().map_err(|e| (e, "-".to_owned().into()))?;

    let mut response = String::new();

    stdin
        .read_line(&mut response)
        .map_err(|e| (e, "-".to_owned().into()))?;

    let response = match response.trim() {
        s if s.eq_ignore_ascii_case("y") => true,
        s if s.eq_ignore_ascii_case("yes") => true,
        _ => false,
    };

    let _ = ManuallyDrop::new(drop);

    trace!(options, "confirmation(...) done");

    Ok(response)
}
