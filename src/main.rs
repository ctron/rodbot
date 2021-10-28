mod config;
mod event;
mod runner;

use crate::{
    config::Config,
    runner::{Context, Runner},
};
use anyhow::Context as _;
use clap::{crate_version, Arg};
use event::Event;
use log::LevelFilter;
use serde_json::json;
use simplelog::{ColorChoice, TermLogger, TerminalMode};
use std::fs::File;

fn main() -> anyhow::Result<()> {
    let matches = clap::App::new("Rodney Bot")
        .author("Jens Reimann <ctron@dentrassi.de>")
        .version(crate_version!())
        .arg(
            Arg::with_name("config")
                .long("config")
                .short("C")
                .takes_value(true)
                .env("RODBOT_CONFIG"),
        )
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .multiple(true),
        )
        .arg(
            Arg::with_name("debug")
                .long("debug")
                .short("d")
                .conflicts_with("verbose"),
        )
        .arg(
            Arg::with_name("quiet")
                .long("quiet")
                .short("q")
                .conflicts_with_all(&["debug", "verbose"]),
        )
        .get_matches();

    let filter = match (
        matches.is_present("quiet"),
        matches.is_present("debug"),
        matches.occurrences_of("verbose"),
    ) {
        (true, _, _) => LevelFilter::Off,
        (_, true, 0) => LevelFilter::Debug,
        (_, true, _) => LevelFilter::Trace,
        (_, false, 0) => LevelFilter::Warn,
        (_, false, 1) => LevelFilter::Info,
        (_, false, 2) => LevelFilter::Debug,
        (_, false, _) => LevelFilter::Trace,
    };

    TermLogger::init(
        filter,
        Default::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    let config = matches.value_of("config").unwrap_or("rodbot.yaml");
    log::debug!("Loading configuration from: {}", config);

    let event = Event::from_env().context("Failed getting event information")?;
    let config: Config =
        serde_yaml::from_reader(File::open(config)?).context("Loading configuration")?;
    log::debug!("Event: {:#?}", event);
    log::debug!("Config: {:#?}", config);

    config.run(&Context {
        payload: &event,
        context: &json!({
            "github": {
                "event": Event::parse_payload::<serde_json::Value>()?
            }
        }),
    })?;

    Ok(())
}
