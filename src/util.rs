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
