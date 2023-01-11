use std::fs;
use std::path::PathBuf;

use clap::ArgMatches;

use crate::util::cli::prompt_yes;
use crate::util::error::{quit, quit_error, ErrorHintsBuilder};

/// Invoke config test command.
pub fn invoke(matches: &ArgMatches) {
    // Get config path, attempt to canonicalize
    let mut path = PathBuf::from(matches.get_one::<String>("config").unwrap());
    if let Ok(p) = path.canonicalize() {
        path = p;
    }

    // Confirm to overwrite if it exists
    if path.is_file()
        && !prompt_yes(
            &format!(
                "Config file already exists, overwrite?\nPath: {}",
                path.to_str().unwrap_or("?")
            ),
            Some(true),
        )
    {
        quit();
    }

    // Generate file
    if let Err(err) = fs::write(&path, include_bytes!("../../res/lazymc.toml")) {
        quit_error(
            anyhow!(err).context("Failed to generate config file"),
            ErrorHintsBuilder::default().build().unwrap(),
        );
    }

    eprintln!("Config saved at: {}", path.to_str().unwrap_or("?"));
}
