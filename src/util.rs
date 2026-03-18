use std::{
    env::var_os,
    fmt::Display,
    io::{BufRead, IsTerminal, Write, stdin, stdout},
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
        s if s.eq_ignore_ascii_case("n") => false,
        s if s.eq_ignore_ascii_case("no") => false,
        "" | _ => default,
    };

    let _ = ManuallyDrop::new(drop);

    trace!(options, "confirmation(...) done");

    Ok(response)
}

pub fn get_color(options: &Cli) -> bool {
    !(options.no_color || var_os("NO_COLOR").is_some_and(|v| v != "") || !stdout().is_terminal())
        || options.force_color
}

pub const INFO_COLOR: &str = "\x1b[36m";
pub const ADD_COLOR: &str = "\x1b[32m";
pub const SUB_COLOR: &str = "\x1b[31m";
pub const MOD_COLOR: &str = "\x1b[33m";
pub const NONE_COLOR: &str = "\x1b[0m";

pub struct SizeDisplay(pub usize, pub bool);

impl Display for SizeDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0..1000 => write!(f, "{}{}B{}", ADD_COLOR, self.0, NONE_COLOR),
            1000..1000000 => write!(
                f,
                "{}{}.{}KB{}",
                ADD_COLOR,
                self.0 / 1000,
                (self.0 / 100) % 10,
                NONE_COLOR
            ),
            1000000..20000000 => write!(
                f,
                "{}{}.{}MB{}",
                ADD_COLOR,
                self.0 / 1000000,
                (self.0 / 100000) % 10,
                NONE_COLOR
            ),
            20000000..1000000000 => write!(
                f,
                "{}{}.{}MB{}",
                MOD_COLOR,
                self.0 / 1000000,
                (self.0 / 100000) % 10,
                NONE_COLOR
            ),
            1000000000.. => write!(
                f,
                "{}{}.{}GB{}",
                SUB_COLOR,
                self.0 / 1000000000,
                (self.0 / 100000000) % 10,
                NONE_COLOR
            ),
        }
    }
}
