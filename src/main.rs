mod config;
mod model;
mod preferences;
mod rpc;
mod tui;

use std::process;

use anyhow::Result;
use clap::Parser;
use config::{build_config, Cli};
use env_logger::Env;
use log::LevelFilter;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let config = build_config(&cli)?;
    init_logging(config.log_level);
    tui::run(config)
}

fn init_logging(level: LevelFilter) {
    let env = Env::default().default_filter_or(level.to_string());
    let _ = env_logger::Builder::from_env(env)
        .format_timestamp(None)
        .format_target(false)
        .try_init();
}
