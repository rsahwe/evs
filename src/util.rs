use std::{
    env::var_os,
    fmt::Display,
    io::{BufRead, IsTerminal, Write, stdin, stdout},
};

use tracing::{debug, instrument};

use crate::{cli::Cli, error::EvsError};

#[instrument(level = "debug", err(level = "debug"), skip_all)]
pub fn confirmation(prompt: &str, default: bool) -> Result<bool, EvsError> {
    let yn = if default { "[Y/n]" } else { "[y/N]" };

    debug!("confirmation(\"{}\", {})", prompt, yn);

    let mut stdout = stdout().lock();
    let mut stdin = stdin().lock();

    stdout
        .write_fmt(format_args!("{} {}: ", prompt, yn))
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
