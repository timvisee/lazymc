use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// File name.
pub const FILE: &str = "server.properties";

/// EOL in server.properties file.
const EOL: &str = "\r\n";

/// Try to rewrite changes in server.properties file in dir.
///
/// Prints an error and stops on failure.
pub fn rewrite_dir<P: AsRef<Path>>(dir: P, changes: HashMap<&str, String>) {
    if changes.is_empty() {
        return;
    }

    // Ensure directory exists
    if !dir.as_ref().is_dir() {
        warn!(target: "lazymc",
            "Not rewriting {} file, configured server directory doesn't exist: {}",
            FILE,
            dir.as_ref().to_str().unwrap_or("?")
        );
        return;
    }

    // Rewrite file
    rewrite_file(dir.as_ref().join(FILE), changes)
}

/// Try to rewrite changes in server.properties file.
///
/// Prints an error and stops on failure.
pub fn rewrite_file<P: AsRef<Path>>(file: P, changes: HashMap<&str, String>) {
    if changes.is_empty() {
        return;
    }

    // File must exist
    if !file.as_ref().is_file() {
        warn!(target: "lazymc",
            "Not writing {} file, not found at: {}",
            FILE,
            file.as_ref().to_str().unwrap_or("?"),
        );
        return;
    }

    // Read contents
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(err) => {
            error!(target: "lazymc",
                "Failed to rewrite {} file, could not load: {}",
                FILE,
                err,
            );
            return;
        }
    };

    // Rewrite file contents, return if nothing changed
    let contents = match rewrite_contents(contents, changes) {
        Some(contents) => contents,
        None => {
            debug!(target: "lazymc",
                "Not rewriting {} file, no changes to apply",
                FILE,
            );
            return;
        }
    };

    // Write changes
    match fs::write(file, contents) {
        Ok(_) => {
            info!(target: "lazymc",
                "Rewritten {} file with updated values",
                FILE,
            );
        }
        Err(err) => {
            error!(target: "lazymc",
                "Failed to rewrite {} file, could not save changes: {}",
                FILE,
                err,
            );
        }
    };
}

/// Rewrite file contents with new properties.
///
/// Returns new file contents if anything has changed.
fn rewrite_contents(contents: String, mut changes: HashMap<&str, String>) -> Option<String> {
    if changes.is_empty() {
        return None;
    }

    let mut changed = false;

    // Build new file
    let mut new_contents: String = contents
        .lines()
        .map(|line| {
            let mut line = line.to_owned();

            // Skip comments or empty lines
            let trim = line.trim();
            if trim.starts_with('#') || trim.is_empty() {
                return line;
            }

            // Try to split property
            let (key, value) = match line.split_once('=') {
                Some(result) => result,
                None => return line,
            };

            // Take any new value, and update it
            if let Some((_, new)) = changes.remove_entry(key.trim().to_lowercase().as_str()) {
                if value != new {
                    line = format!("{key}={new}");
                    changed = true;
                }
            }

            line
        })
        .collect::<Vec<_>>()
        .join(EOL);

    // Append any missed changes
    for (key, value) in changes {
        new_contents += &format!("{EOL}{key}={value}");
        changed = true;
    }

    // Return new contents if changed
    if changed {
        Some(new_contents)
    } else {
        None
    }
}

/// Read the given property from the given server.properties file.o
///
/// Returns `None` if file does not contain the property.
pub fn read_property<P: AsRef<Path>>(file: P, property: &str) -> Option<String> {
    // File must exist
    if !file.as_ref().is_file() {
        warn!(target: "lazymc",
            "Failed to read property from {} file, it does not exist",
            FILE,
        );
        return None;
    }

    // Read contents
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(err) => {
            error!(target: "lazymc",
                "Failed to read property from {} file, could not load: {}",
                FILE,
                err,
            );
            return None;
        }
    };

    // Find property, return value
    contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(p, _)| p.trim().to_lowercase() == property.to_lowercase())
        .map(|(_, v)| v.trim().to_string())
}
