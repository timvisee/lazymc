use clap::{App, AppSettings, Arg};

/// The clap app for CLI argument parsing.
pub fn app() -> App<'static> {
    App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .subcommand(
            App::new("start")
                .alias("run")
                .about("Start lazymc and server (default)"),
        )
        .subcommand(
            App::new("config")
                .alias("cfg")
                .about("Config actions")
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(App::new("generate").alias("gen").about("Generate config"))
                .subcommand(App::new("test").about("Test config")),
        )
        .arg(
            Arg::new("config")
                .short('c')
                .alias("cfg")
                .long("config")
                .global(true)
                .value_name("FILE")
                .default_value(crate::config::CONFIG_FILE)
                .about("Use config file")
                .takes_value(true),
        )
}
