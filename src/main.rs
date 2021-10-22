mod config;
mod event;
mod runner;

use crate::{
    config::Config,
    runner::{Context, Runner},
};
use anyhow::Context as _;
use clap::Arg;
use event::Event;
use log::LevelFilter;
use serde_json::json;
use simplelog::{ColorChoice, TermLogger, TerminalMode};
use std::fs::File;

fn main() -> anyhow::Result<()> {
    let matches = clap::App::new("Rodney Bot")
        .author("Jens Reimann <ctron@dentrassi.de>")
        .arg(
            Arg::with_name("config")
                .long("config")
                .short("C")
                .takes_value(true)
                .env("RODBOT_CONFIG"),
        )
        .get_matches();

    TermLogger::init(
        LevelFilter::Debug,
        Default::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    let config = matches.value_of("config").unwrap_or("rodbot.yaml");

    let event = Event::from_env()?;
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
