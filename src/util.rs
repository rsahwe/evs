use std::{
    env::{current_dir, var_os},
    ffi::OsStr,
    fmt::{self, Arguments, Display, Formatter},
    io::{self, BufRead as _, ErrorKind, IsTerminal as _, Write as _, stdin, stdout},
    path::{self, Path, PathBuf, absolute},
};

use clap_complete::CompletionCandidate;
use glob::glob;
use tracing::{debug, instrument, trace};

use crate::{
    cli::{Cli, Commands},
    error::EvsError,
    repo::Repository,
};

#[macro_export]
macro_rules! confirmation {
    ($default:literal, $fmt:literal $($arg:tt)*) => {
        $crate::util::confirmation_impl(format_args!($fmt $($arg)*), $default)
    };
}

#[inline]
#[instrument(level = "debug", err(level = "debug"), skip_all)]
pub fn confirmation_impl(prompt: Arguments, default: bool) -> Result<bool, EvsError> {
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
        _ => default,
    };

    Ok(response)
}

#[inline]
#[must_use]
pub fn get_color(options: &Cli) -> bool {
    !(options.no_color
        || var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
        || !stdout().is_terminal())
        || options.force_color
}

pub const INFO_COLOR: &str = "\x1b[36m";
pub const ADD_COLOR: &str = "\x1b[32m";
pub const SUB_COLOR: &str = "\x1b[31m";
pub const MOD_COLOR: &str = "\x1b[33m";
pub const NONE_COLOR: &str = "\x1b[0m";

pub struct SizeDisplay(pub usize, pub bool);

impl Display for SizeDisplay {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0 {
            0..1_000 => write!(f, "{}{}B{}", ADD_COLOR, self.0, NONE_COLOR),
            1_000..1_000_000 => write!(
                f,
                "{}{}.{}KB{}",
                ADD_COLOR,
                self.0 / 1000,
                (self.0 / 100) % 10,
                NONE_COLOR
            ),
            1_000_000..20_000_000 => write!(
                f,
                "{}{}.{}MB{}",
                ADD_COLOR,
                self.0 / 1_000_000,
                (self.0 / 100_000) % 10,
                NONE_COLOR
            ),
            20_000_000..1_000_000_000 => write!(
                f,
                "{}{}.{}MB{}",
                MOD_COLOR,
                self.0 / 1_000_000,
                (self.0 / 100_000) % 10,
                NONE_COLOR
            ),
            1_000_000_000.. => write!(
                f,
                "{}{}.{}GB{}",
                SUB_COLOR,
                self.0 / 1_000_000_000,
                (self.0 / 100_000_000) % 10,
                NONE_COLOR
            ),
        }
    }
}

#[inline]
pub fn repo_ref_completer(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };

    let cli = Cli {
        verbose: 0,
        no_color: true,
        force_color: false,
        command: Commands::Completion,
    };

    let Ok(repo) = Repository::find(".", &cli) else {
        return Vec::new();
    };

    let Some(store_dir) = repo.store.path().to_str() else {
        return Vec::new();
    };

    //TODO: BRANCHES

    let mut result = if "HEAD".starts_with(current) {
        vec![CompletionCandidate::new("HEAD")]
    } else {
        Vec::new()
    };

    if let Ok(paths) = glob(&format!(
        "{}{}{}*",
        store_dir,
        path::MAIN_SEPARATOR,
        current
    )) {
        result.extend(
            paths
                .flatten()
                .flat_map(|p| p.strip_prefix(store_dir).map(Path::to_path_buf))
                .map(CompletionCandidate::new),
        );
    }

    result
}

#[inline]
#[instrument(level = "debug", err(level = "debug"), skip_all)]
pub fn partial_canonicalize<T: AsRef<Path>>(path: T) -> io::Result<PathBuf> {
    debug!("partial_canonicalize({:?})", path.as_ref());

    let path = current_dir()?.join(path.as_ref());

    trace!("Using path {:?}.", path);

    for ancestor in path.ancestors() {
        if let Ok(real_ancestor) = ancestor.canonicalize() {
            let rest = path.strip_prefix(ancestor).unwrap();

            trace!("Found ancestor {:?} with rest {:?}...", real_ancestor, rest);

            if rest == "" {
                return Ok(real_ancestor);
            }

            if rest
                .components()
                .find(|c| {
                    matches!(
                        c,
                        path::Component::ParentDir
                            | path::Component::RootDir
                            | path::Component::Prefix(_)
                    )
                })
                .is_some()
            {
                return Err(io::Error::new(
                    ErrorKind::InvalidFilename,
                    "cannot predict canonical form of ambiguous file path",
                ));
            }

            return absolute(real_ancestor.join(rest));
        }
    }

    unreachable!("An absolute path always has an ancestor.")
}
