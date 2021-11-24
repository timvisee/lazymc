use std::collections::HashMap;
use std::sync::Arc;

use clap::ArgMatches;

use crate::config::{self, Config, Server as ConfigServer};
use crate::mc::server_properties;
use crate::proto;
use crate::service;

/// RCON randomized password length.
#[cfg(feature = "rcon")]
const RCON_PASSWORD_LENGTH: usize = 32;

/// Start lazymc.
pub fn invoke(matches: &ArgMatches) -> Result<(), ()> {
    // Load config
    #[allow(unused_mut)]
    let mut config = config::load(matches);

    // Prepare RCON if enabled
    #[cfg(feature = "rcon")]
    prepare_rcon(&mut config);

    // Rewrite server server.properties file
    rewrite_server_properties(&config);

    // Start server service
    let config = Arc::new(config);
    service::server::service(config)
}

/// Prepare RCON.
#[cfg(feature = "rcon")]
fn prepare_rcon(config: &mut Config) {
    use crate::util::error::{quit_error_msg, ErrorHintsBuilder};

    // On Windows, this must be enabled
    if cfg!(windows) && !config.rcon.enabled {
        quit_error_msg(
            "RCON must be enabled on Windows",
            ErrorHintsBuilder::default()
                .add_info("change 'rcon.enabled' to 'true' in the config file".into())
                .build()
                .unwrap(),
        );
    }

    // Skip if not enabled
    if !config.rcon.enabled {
        return;
    }

    // Must configure RCON password with no randomization
    if config.server.address.port() == config.rcon.port {
        quit_error_msg(
            "RCON port cannot be the same as the server",
            ErrorHintsBuilder::default()
                .add_info("change 'rcon.port' in the config file".into())
                .build()
                .unwrap(),
        );
    }

    // Must configure RCON password with no randomization
    if config.rcon.password.trim().is_empty() && !config.rcon.randomize_password {
        quit_error_msg(
            "RCON password can't be empty, or enable randomization",
            ErrorHintsBuilder::default()
                .add_info("change 'rcon.randomize_password' to 'true' in the config file".into())
                .add_info("or change 'rcon.password' in the config file".into())
                .build()
                .unwrap(),
        );
    }

    // RCON password randomization
    if config.rcon.randomize_password {
        // Must enable server.properties rewrite
        if !config.advanced.rewrite_server_properties {
            quit_error_msg(
                format!(
                    "You must enable {} rewrite to use RCON password randomization",
                    server_properties::FILE
                ),
                ErrorHintsBuilder::default()
                    .add_info(
                        "change 'advanced.rewrite_server_properties' to 'true' in the config file"
                            .into(),
                    )
                    .build()
                    .unwrap(),
            );
        }

        // Randomize password
        config.rcon.password = generate_random_password();
    }
}

/// Generate secure random password.
#[cfg(feature = "rcon")]
fn generate_random_password() -> String {
    use rand::{distributions::Alphanumeric, Rng};
    use std::iter;

    let mut rng = rand::thread_rng();
    iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .map(char::from)
        .take(RCON_PASSWORD_LENGTH)
        .collect()
}

/// Rewrite server server.properties file with correct internal IP and port.
fn rewrite_server_properties(config: &Config) {
    // Rewrite must be enabled
    if !config.advanced.rewrite_server_properties {
        return;
    }

    // Ensure server directory is set, it must exist
    let dir = match ConfigServer::server_directory(config) {
        Some(dir) => dir,
        None => {
            warn!(target: "lazymc", "Not rewriting {} file, server directory not configured (server.directory)", server_properties::FILE);
            return;
        }
    };

    // Build list of changes
    #[allow(unused_mut)]
    let mut changes = HashMap::from([
        ("server-ip", config.server.address.ip().to_string()),
        ("server-port", config.server.address.port().to_string()),
        ("enable-status", "true".into()),
        ("query.port", config.server.address.port().to_string()),
    ]);

    // If connecting to server over non-loopback address, disable proxy blocking
    if !config.server.address.ip().is_loopback() {
        changes.extend([("prevent-proxy-connections", "false".into())]);
    }

    // Update network compression threshold for lobby mode
    if config.join.methods.contains(&config::Method::Lobby) {
        changes.extend([(
            "network-compression-threshold",
            proto::COMPRESSION_THRESHOLD.to_string(),
        )]);
    }

    // Add RCON configuration
    #[cfg(feature = "rcon")]
    if config.rcon.enabled {
        changes.extend([
            ("rcon.port", config.rcon.port.to_string()),
            ("rcon.password", config.rcon.password.clone()),
            ("enable-rcon", "true".into()),
        ]);
    }

    // Rewrite file
    server_properties::rewrite_dir(dir, changes)
}
