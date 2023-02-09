use clap::{Arg, Command};

/// The clap app for CLI argument parsing.
pub fn app() -> Command {
    Command::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .subcommand(
            Command::new("start")
                .alias("run")
                .about("Start lazymc and server (default)"),
        )
        .subcommand(
            Command::new("config")
                .alias("cfg")
                .about("Config actions")
                .arg_required_else_help(true)
                .subcommand_required(true)
                .subcommand(
                    Command::new("generate")
                        .alias("gen")
                        .about("Generate config"),
                )
                .subcommand(Command::new("test").about("Test config")),
        )
        .arg(
            Arg::new("bind")
                .short('b')
                .value_name("ADDRESS")
                .default_value("0.0.0.0:25565")
                .help("Address to bind to")
                .num_args(1),
        )
        .arg(
            Arg::new("config")
                .short('c')
                .alias("cfg")
                .long("config")
                .global(true)
                .value_name("FILE")
                .default_value(crate::config::CONFIG_FILE)
                .help("Use config file")
                .num_args(1),
        )
}
