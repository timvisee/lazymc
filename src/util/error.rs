// Allow dead code, until we've fully implemented CLI/error handling
#![allow(dead_code)]

use std::borrow::Borrow;
use std::fmt::{Debug, Display};
use std::io::{self, Write};
pub use std::process::exit;

use anyhow::anyhow;

use crate::util::style::{highlight, highlight_error, highlight_info, highlight_warning};

/// Print the given error in a proper format for the user,
/// with it's causes.
pub fn print_error(err: anyhow::Error) {
    // Report each printable error, count them
    let count = err
        .chain()
        .map(|err| err.to_string())
        .filter(|err| !err.is_empty())
        .enumerate()
        .map(|(i, err)| {
            if i == 0 {
                eprintln!("{} {}", highlight_error("error:"), err);
            } else {
                eprintln!("{} {}", highlight_error("caused by:"), err);
            }
        })
        .count();

    // Fall back to a basic message
    if count == 0 {
        eprintln!("{} an undefined error occurred", highlight_error("error:"),);
    }
}

/// Print the given error message in a proper format for the user,
/// with it's causes.
pub fn print_error_msg<S>(err: S)
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    print_error(anyhow!(err));
}

/// Print a warning.
pub fn print_warning<S>(err: S)
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    eprintln!("{} {}", highlight_warning("warning:"), err);
}

/// Quit the application regularly.
pub fn quit() -> ! {
    exit(0);
}

/// Quit the application with an error code,
/// and print the given error.
pub fn quit_error(err: anyhow::Error, hints: impl Borrow<ErrorHints>) -> ! {
    // Print the error
    print_error(err);

    // Print error hints
    hints.borrow().print(false);

    // Quit
    exit(1);
}

/// Quit the application with an error code,
/// and print the given error message.
pub fn quit_error_msg<S>(err: S, hints: impl Borrow<ErrorHints>) -> !
where
    S: AsRef<str> + Display + Debug + Sync + Send + 'static,
{
    quit_error(anyhow!(err), hints);
}

/// The error hint configuration.
#[derive(Clone, Builder)]
#[builder(default)]
pub struct ErrorHints {
    /// A list of info messages to print along with the error.
    info: Vec<String>,

    /// Show about the config flag.
    config: bool,

    /// Show about the config generate command.
    config_generate: bool,

    /// Show about the config test command.
    config_test: bool,

    /// Show about the verbose flag.
    verbose: bool,

    /// Show about the help flag.
    help: bool,
}

impl ErrorHints {
    /// Check whether any hint should be printed.
    pub fn any(&self) -> bool {
        self.config || self.config_generate || self.config_test || self.verbose || self.help
    }

    /// Print the error hints.
    pub fn print(&self, end_newline: bool) {
        // Print info messages
        for msg in &self.info {
            eprintln!("{} {}", highlight_info("info:"), msg);
        }

        // Stop if nothing should be printed
        if !self.any() {
            return;
        }

        eprintln!();

        // Print hints
        let bin = crate::util::bin_name();
        if self.config_generate {
            eprintln!(
                "Use '{}' to generate a new config file",
                highlight(&format!("{bin} config generate"))
            );
        }
        if self.config {
            eprintln!(
                "Use '{}' to select a config file",
                highlight("--config FILE")
            );
        }
        if self.config_test {
            eprintln!(
                "Use '{}' to test a config file",
                highlight(&format!("{bin} config test -c FILE"))
            );
        }
        if self.verbose {
            eprintln!("For a detailed log add '{}'", highlight("--verbose"));
        }
        if self.help {
            eprintln!("For more information add '{}'", highlight("--help"));
        }

        // End with additional newline
        if end_newline {
            eprintln!();
        }

        // Flush
        let _ = io::stderr().flush();
    }
}

impl Default for ErrorHints {
    fn default() -> Self {
        ErrorHints {
            info: Vec::new(),
            config: false,
            config_generate: false,
            config_test: false,
            verbose: true,
            help: true,
        }
    }
}

impl ErrorHintsBuilder {
    /// Add a single info entry.
    pub fn add_info(mut self, info: String) -> Self {
        // Initialize the info list
        if self.info.is_none() {
            self.info = Some(Vec::new());
        }

        // Add the item to the info list
        if let Some(ref mut list) = self.info {
            list.push(info);
        }

        self
    }
}
